use rayon::prelude::*;
use reversi_tools::position::*;

use crate::tt::{
    hash_position, tt, BOUND_EXACT, BOUND_LOWER, BOUND_NONE, BOUND_UPPER, NO_MOVE_SQ,
};
use crate::utils::splitmix64;

// --------------------------------------------------------------------------
// Constants shared across the engine
// --------------------------------------------------------------------------

const CORNER_MASK: u64 = 0x8100_0000_0000_0081;
const EDGE_MASK: u64 = 0x42C3_0000_0000_C342;
const ANTIEDGE_MASK: u64 = 4_792_111_478_498_951_490;
const ANTICORNER_MASK: u64 = 18_577_348_462_920_192;

// Mate magnitude: scores whose absolute value exceeds this threshold are
// mate-distance scores that get shrunk by one each ply as they propagate up.
const MATE_THRESHOLD: i32 = 5000;

// Below this remaining depth the branching factor is small enough that the
// `compute_moves`-per-candidate cost of mobility-based ordering exceeds the
// pruning savings, so we fall back to the cheap bucket ordering.
const MOBILITY_ORDER_MIN_DEPTH: u32 = 3;

// Killer-move table: two slots per ply, remembering the moves that most
// recently caused a beta cutoff at that ply in a sibling subtree. After
// the TT move (which is per-position), killers are the next candidates
// to try, since similar positions at the same ply tend to share refutation
// moves. `KILLER_PLIES` must exceed the deepest iterative-deepening depth
// the engine is ever asked to run - 64 is more than enough for 8x8 Othello.
const KILLER_PLIES: usize = 64;

#[derive(Copy, Clone)]
pub struct KillerTable([[u64; 2]; KILLER_PLIES]);

impl KillerTable {
    #[inline(always)]
    pub fn new() -> Self {
        Self([[0u64; 2]; KILLER_PLIES])
    }
}

impl Default for KillerTable {
    fn default() -> Self {
        Self::new()
    }
}

// Per-search context. Everything that's constant or monotonically mutable
// over the whole search is bundled here and passed by `&mut` through the
// recursion. This keeps the hot `nega_search_impl` signature at 6
// arguments (us, them, depth, alpha, beta, ctx) - all sysv-abi register
// candidates - and spares each recursive call from re-shuffling four
// extra values onto the stack frame.
pub struct SearchCtx {
    pub orig_depth: u32,
    pub cfg: EvalCfg,
    /// Full-avalanche hash of the active `EvalCfg`. XORed into every
    /// TT key so different configs get distinct logical partitions of
    /// the shared TT. This is essential during tuning (where two
    /// configs alternate within one game) to prevent cross-config
    /// score pollution: without it, config A's cached heuristic eval
    /// would be returned to config B's probes as if authoritative,
    /// producing spurious ~0.5% "wins" that don't reproduce on
    /// independent validation sets. In production (single-config
    /// search) this is a no-op since `cfg_key` is constant.
    pub cfg_key: u64,
    pub node_count: u64,
    pub killers: KillerTable,
}

impl SearchCtx {
    #[inline(always)]
    pub fn new(orig_depth: u32, cfg: EvalCfg) -> Self {
        Self {
            orig_depth,
            cfg,
            cfg_key: eval_cfg_key(&cfg),
            node_count: 0,
            killers: KillerTable::new(),
        }
    }
}

/// Compute a 64-bit full-avalanche key from an [`EvalCfg`]. Used to
/// partition TT entries by config so cross-config pollution can't
/// occur (see `SearchCtx::cfg_key`). Any change to the eval function
/// signature (adding new fields) MUST be reflected here to keep the
/// partition complete.
#[inline]
pub fn eval_cfg_key(cfg: &EvalCfg) -> u64 {
    // Fold all coefficients into the hash via successive splitmix64
    // rounds so a single-coefficient ±1 change propagates through
    // every output bit. The exact pack order doesn't matter as long
    // as every field contributes.
    let mut h: u64 = 0xA2A8_8E47_2F35_8101;
    let fields: [i32; 10] = [
        cfg.corner_value,
        cfg.edge_value,
        cfg.antiedge_value,
        cfg.anticorner_value,
        cfg.disc_values[0],
        cfg.disc_values[1],
        cfg.disc_values[2],
        cfg.mobility_values[0],
        cfg.mobility_values[1],
        cfg.mobility_values[2],
    ];
    for f in fields {
        h = splitmix64(h.wrapping_add((f as u32) as u64));
    }
    h
}

// Special return codes from `check_game_status`. Anything below `PASS_OUTCOME`
// is an actual move bitmap.
const DRAW_OUTCOME: u64 = u64::MAX - 3;
const BLACK_WON_OUTCOME: u64 = u64::MAX - 1;
const WHITE_WON_OUTCOME: u64 = u64::MAX - 2;
const PASS_OUTCOME: u64 = u64::MAX;

#[inline(always)]
fn lowest_set_bit(x: u64) -> u64 {
    x & x.wrapping_neg()
}

#[inline(always)]
fn pop_lsb(bits: &mut u64) -> u64 {
    let b = *bits;
    let lsb = b & b.wrapping_neg();
    *bits = b & (b - 1);
    lsb
}

/// Mate-distance shrink: scores more extreme than ±MATE_THRESHOLD are pulled
/// one step towards zero on every ply so the engine prefers quicker wins /
/// slower losses. Magnitude-preserving enough that it commutes with sign
/// flips (used in the negamax recursion).
#[inline(always)]
fn adjust_mate_distance(v: i32) -> i32 {
    if v > MATE_THRESHOLD {
        v - 1
    } else if v < -MATE_THRESHOLD {
        v + 1
    } else {
        v
    }
}

// --------------------------------------------------------------------------
// Legal move enumeration
// --------------------------------------------------------------------------

pub fn find_legal_moves_alt(white: u64, black: u64, is_white_to_move: bool) -> Vec<u64> {
    let (me, opp) = if is_white_to_move {
        (white, black)
    } else {
        (black, white)
    };

    let all_moves = compute_moves(me, opp);

    let mut result = Vec::new();
    let mut tmp = all_moves;
    while tmp != 0 {
        let bit = lowest_set_bit(tmp);
        result.push(bit);
        tmp &= !bit;
    }
    result
}

// --------------------------------------------------------------------------
// Static evaluation
// --------------------------------------------------------------------------

/// Per-phase tunable coefficients. Features that only matter at
/// certain stages of the game - or matter very differently at them -
/// get one value per phase; purely geometric features (corner vs
/// X-square) share a single value across all phases since the board
/// itself doesn't change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalCfg {
    // ---- Phase-independent positional coefficients ---------------
    // The value of a square derives from its structural role
    // (corner=untakeable, X-square=dangerous-next-to-empty-corner)
    // and this role doesn't change with move number.
    pub corner_value: i32,
    pub edge_value: i32,
    pub antiedge_value: i32,
    pub anticorner_value: i32,

    // ---- Phase-dependent counting coefficients -------------------
    // Indexed as [opening, midgame, endgame] (see `phase_index`).
    // Disc count barely matters in the opening (most discs will flip
    // many times) but *is* the final score at the end of the game;
    // mobility peaks in importance around the midgame where the
    // branching factor is high and whole position families diverge.
    pub disc_values: [i32; 3],
    pub mobility_values: [i32; 3],
}

/// Game-phase bucketing by empty-square count. Three buckets balance
/// expressive power against the size of the tuning space (10 total
/// coefficients). Boundaries at 40 / 20 empties are standard-ish for
/// Othello engines and align with observed transitions in mobility
/// and disc-count importance.
#[inline(always)]
fn phase_index(empties: u32) -> usize {
    if empties >= 40 {
        0 // opening
    } else if empties >= 20 {
        1 // midgame
    } else {
        2 // endgame
    }
}

pub static DEFAULT_CFG: EvalCfg = EvalCfg {
    // These coefficients were obtained by the (1+1)-ES tuner in
    // `src/tune.rs` at d7 over ~1300 symmetry-reduced training
    // positions and validated independently at d7 and d8 on the
    // full 1893-position set. Held-out margins:
    //   d7 full set: +56.21% vs hand-picked extended defaults
    //   d8 full set: +52.01% vs hand-picked extended defaults
    // and the old 4-coefficient eval loses ~79% to the hand-picked
    // extended defaults on top of that - so this config is roughly
    // 3x stronger (per head-to-head margin) than the pre-Stage-2
    // engine at the same search depth.
    //
    // Directional notes for maintainers:
    // - Positional coefs barely moved from the old 4-coef tuned
    //   values; that eval dimension was already saturated.
    // - `disc_values[0..=1]` are NEGATIVE: in opening and midgame,
    //   having fewer discs is *better* because fewer discs leave
    //   more opponent moves tied up and preserve your own mobility.
    //   The tuner rediscovered this classic Othello principle from
    //   scratch.
    // - `mobility_values[2] = 16` (endgame mobility) is the single
    //   largest non-corner coefficient: in the endgame even a
    //   one-move mobility advantage is often decisive.
    corner_value: 69,
    edge_value: 18,
    antiedge_value: -21,
    anticorner_value: -30,
    disc_values: [-7, -1, 1],
    mobility_values: [7, 4, 16],
};

/// Phase-independent positional score. The disc-count and mobility
/// contributions are added by the caller from the phase-selected
/// coefficients.
#[inline(always)]
fn side_positional(bb: u64, cfg: EvalCfg) -> i32 {
    (bb & CORNER_MASK).count_ones() as i32 * cfg.corner_value
        + (bb & EDGE_MASK).count_ones() as i32 * cfg.edge_value
        + (bb & ANTIEDGE_MASK).count_ones() as i32 * cfg.antiedge_value
        + (bb & ANTICORNER_MASK).count_ones() as i32 * cfg.anticorner_value
}

/// Full static evaluation in the us-frame: positional + disc count
/// + mobility, with disc and mobility weights indexed by game phase.
/// Mobility uses `compute_moves` (SIMD-accelerated in
/// reversi-tools), which costs ~2x the previous eval's popcnts - a
/// worthwhile trade against the per-leaf quality improvement this
/// buys.
#[inline(always)]
fn eval_us_them(us: u64, them: u64, cfg: EvalCfg) -> i32 {
    let empties = (!(us | them)).count_ones();
    let phase = phase_index(empties);

    let our_mobility = compute_moves(us, them).count_ones() as i32;
    let their_mobility = compute_moves(them, us).count_ones() as i32;
    let mobility_score = (our_mobility - their_mobility) * cfg.mobility_values[phase];

    let disc_score =
        (us.count_ones() as i32 - them.count_ones() as i32) * cfg.disc_values[phase];

    let positional_score = side_positional(us, cfg) - side_positional(them, cfg);

    positional_score + mobility_score + disc_score
}

pub fn eval_position_with_cfg(white: u64, black: u64, eval_cfg: EvalCfg) -> i32 {
    // Absolute frame (black - white) for callers that don't work in
    // us/them. Mobility is computed from black's perspective.
    let empties = (!(white | black)).count_ones();
    let phase = phase_index(empties);

    let black_mobility = compute_moves(black, white).count_ones() as i32;
    let white_mobility = compute_moves(white, black).count_ones() as i32;
    let mobility_score =
        (black_mobility - white_mobility) * eval_cfg.mobility_values[phase];

    let disc_score = (black.count_ones() as i32 - white.count_ones() as i32)
        * eval_cfg.disc_values[phase];

    let positional = side_positional(black, eval_cfg) - side_positional(white, eval_cfg);

    positional + mobility_score + disc_score
}

// --------------------------------------------------------------------------
// Core negamax search with transposition table
// --------------------------------------------------------------------------
//
// All scores are in the side-to-move's frame (+10000 = we just won). The
// colour flag never appears inside the hot path; the public API wrappers
// convert between absolute (black - white) and us-perspective scores at
// the call boundary.
//
// The `COUNT` const generic selects whether the search increments a node
// counter - used by the benchmark harness. The compiler monomorphises
// the function into two copies so the counter path imposes no overhead
// on the production search.

#[inline(always)]
fn apply_move_us_them(us: u64, them: u64, move_bit: u64) -> (u64, u64) {
    apply_move_unchecked(us, them, move_bit, true)
}

#[inline(always)]
fn game_status_us_them(us: u64, them: u64) -> u64 {
    check_game_status(us, them, true)
}

fn nega_search_impl<const COUNT: bool>(
    us: u64,
    them: u64,
    depth: u32,
    alpha: i32,
    beta: i32,
    ctx: &mut SearchCtx,
) -> (u64, i32) {
    if COUNT {
        ctx.node_count += 1;
    }
    let orig_depth = ctx.orig_depth;

    let outcome = game_status_us_them(us, them);

    if outcome >= DRAW_OUTCOME {
        if outcome == WHITE_WON_OUTCOME {
            return (u64::MAX, 10_000);
        }
        if outcome == BLACK_WON_OUTCOME {
            return (u64::MAX, -10_000);
        }
        if outcome == DRAW_OUTCOME {
            return (u64::MAX, 0);
        }
        // Pass: swap sides without consuming depth, then negate child's
        // score back into our frame.
        let (_, child) = nega_search_impl::<COUNT>(them, us, depth, -beta, -alpha, ctx);
        return (u64::MAX, -child);
    }

    if depth == 0 {
        return (u64::MAX, eval_us_them(us, them, ctx.cfg));
    }

    // ---- TT probe -------------------------------------------------------
    // XOR in `cfg_key` so different eval configs access disjoint TT
    // slots (see `SearchCtx::cfg_key` for rationale).
    let key = hash_position(us, them) ^ ctx.cfg_key;
    let mut tt_move_bit: u64 = 0;
    let mut a = alpha;
    let mut b = beta;

    if let Some(entry) = tt().probe(key) {
        if entry.bound != BOUND_NONE && entry.depth as i32 >= depth as i32 {
            let s = entry.score;
            let stored_move = if entry.move_sq < NO_MOVE_SQ {
                1u64 << entry.move_sq
            } else {
                u64::MAX
            };
            match entry.bound {
                BOUND_EXACT => return (stored_move, s),
                BOUND_LOWER => {
                    if s >= b {
                        return (stored_move, s);
                    }
                    if s > a {
                        a = s;
                    }
                }
                BOUND_UPPER => {
                    if s <= a {
                        return (stored_move, s);
                    }
                    if s < b {
                        b = s;
                    }
                }
                _ => {}
            }
        }
        if entry.move_sq < NO_MOVE_SQ {
            let candidate = 1u64 << entry.move_sq;
            if outcome & candidate != 0 {
                tt_move_bit = candidate;
            }
        }
    }

    // "alpha we searched with", captured before any mutation during the
    // move loop - used for final bound classification.
    let alpha_used = a;

    // Killer-move slots for this ply (clamped against the static table size
    // so absurdly deep ID targets don't OOB).
    let ply_idx = (orig_depth.saturating_sub(depth) as usize).min(KILLER_PLIES - 1);
    let k0_raw = ctx.killers.0[ply_idx][0];
    let k1_raw = ctx.killers.0[ply_idx][1];
    let killer0 = if k0_raw != 0 && k0_raw != tt_move_bit && (outcome & k0_raw) != 0 {
        k0_raw
    } else {
        0
    };
    let killer1 = if k1_raw != 0
        && k1_raw != tt_move_bit
        && k1_raw != killer0
        && (outcome & k1_raw) != 0
    {
        k1_raw
    } else {
        0
    };
    // Moves already tried via TT / killer slots - exclude them from the
    // ordered-remainder pass so we don't re-search duplicates.
    let already_tried = tt_move_bit | killer0 | killer1;

    let mut best_move: u64 = u64::MAX;
    let mut best_v: i32 = i32::MIN;
    // `searched_any` controls Principal Variation Search: the first move at
    // a node gets a full-window search (full alpha-beta accuracy); every
    // subsequent move is speculatively searched with a null window, and
    // only re-searched with the full window if it fails high. This is
    // strictly cheaper when move ordering is good (which, with the TT-move
    // seed and the coarse bucket ordering, it usually is).
    let mut searched_any = false;

    macro_rules! try_move_cached {
        ($candidate:expr, $new_us:expr, $new_them:expr) => {{
            let candidate = $candidate;
            let new_us_c = $new_us;
            let new_them_c = $new_them;

            let child_v = if !searched_any {
                let (_, cv) = nega_search_impl::<COUNT>(
                    new_them_c,
                    new_us_c,
                    depth - 1,
                    -b,
                    -a,
                    ctx,
                );
                cv
            } else {
                let (_, cv) = nega_search_impl::<COUNT>(
                    new_them_c,
                    new_us_c,
                    depth - 1,
                    -a - 1,
                    -a,
                    ctx,
                );
                let tentative = adjust_mate_distance(-cv);
                if tentative > a && tentative < b && a + 1 < b {
                    let (_, cv2) = nega_search_impl::<COUNT>(
                        new_them_c,
                        new_us_c,
                        depth - 1,
                        -b,
                        -a,
                        ctx,
                    );
                    cv2
                } else {
                    cv
                }
            };
            let v = adjust_mate_distance(-child_v);
            searched_any = true;
            if v > best_v {
                best_v = v;
                best_move = candidate;
                if v > a {
                    a = v;
                }
                if a >= b {
                    // Record the cutoff move as a killer at this ply, unless
                    // it's already slot 0 (so the two slots are always
                    // distinct). Slot 0 shifts to slot 1 (a tiny LRU).
                    let cur_k0 = ctx.killers.0[ply_idx][0];
                    if candidate != cur_k0 {
                        ctx.killers.0[ply_idx][1] = cur_k0;
                        ctx.killers.0[ply_idx][0] = candidate;
                    }
                    tt().store(
                        key,
                        v,
                        depth as i8,
                        BOUND_LOWER,
                        candidate.trailing_zeros() as u8,
                    );
                    return (candidate, v);
                }
            }
        }};
    }

    macro_rules! try_move {
        ($candidate:expr) => {{
            let candidate = $candidate;
            let (new_us_c, new_them_c) = apply_move_us_them(us, them, candidate);
            try_move_cached!(candidate, new_us_c, new_them_c);
        }};
    }

    // Try the TT move first (if legal) - best candidate for beta cutoff.
    if tt_move_bit != 0 {
        try_move!(tt_move_bit);
    }

    // Then the killers (if legal, distinct, and not the TT move).
    if killer0 != 0 {
        try_move!(killer0);
    }
    if killer1 != 0 {
        try_move!(killer1);
    }

    // For the remaining moves we have two ordering strategies:
    //   (a) Deep nodes (depth >= 3): sort candidates by opponent mobility
    //       after the move, with positional biases. Paying `compute_moves`
    //       per candidate is cheaper than wasting an unordered subtree.
    //   (b) Shallow nodes: the four-bucket static ordering (corners, good
    //       edges, quiet squares, bad squares). The bucket split is just
    //       four mask-AND operations and wins on raw throughput when the
    //       subtree doesn't offer much to prune.
    if depth >= MOBILITY_ORDER_MIN_DEPTH {
        // Score / apply / cache each move on the stack.
        #[derive(Copy, Clone)]
        struct Scored {
            priority: i32,
            candidate: u64,
            new_us: u64,
            new_them: u64,
        }
        let mut scored: [Scored; 32] = [Scored {
            priority: 0,
            candidate: 0,
            new_us: 0,
            new_them: 0,
        }; 32];
        let mut n = 0usize;
        let mut remaining = outcome & !already_tried;
        while remaining != 0 {
            let candidate = pop_lsb(&mut remaining);
            let (new_us_c, new_them_c) = apply_move_us_them(us, them, candidate);
            let mob = compute_moves(new_them_c, new_us_c).count_ones() as i32;
            // Positional biases keep the coarse corner/edge/X-square
            // preferences of the old ordering without having to special-
            // case them in the sort below.
            let mut priority = mob;
            if candidate & CORNER_MASK != 0 {
                priority -= 1000;
            } else if candidate & ANTICORNER_MASK != 0 {
                priority += 200;
            } else if candidate & ANTIEDGE_MASK != 0 {
                priority += 80;
            } else if candidate & EDGE_MASK != 0 {
                priority -= 20;
            }
            scored[n] = Scored {
                priority,
                candidate,
                new_us: new_us_c,
                new_them: new_them_c,
            };
            n += 1;
        }
        scored[..n].sort_unstable_by_key(|s| s.priority);
        for i in 0..n {
            let s = scored[i];
            try_move_cached!(s.candidate, s.new_us, s.new_them);
        }
    } else {
        let mut corner_moves = outcome & CORNER_MASK & !already_tried;
        let mut edge_moves = outcome & EDGE_MASK & !ANTIEDGE_MASK & !already_tried;
        let mut other_moves = outcome
            & !(CORNER_MASK | EDGE_MASK | ANTIEDGE_MASK | ANTICORNER_MASK)
            & !already_tried;
        let mut bad_moves = outcome & (ANTIEDGE_MASK | ANTICORNER_MASK) & !already_tried;

        macro_rules! run_bucket {
            ($moves:ident) => {
                while $moves != 0 {
                    let candidate = pop_lsb(&mut $moves);
                    try_move!(candidate);
                }
            };
        }

        run_bucket!(corner_moves);
        run_bucket!(edge_moves);
        run_bucket!(other_moves);
        run_bucket!(bad_moves);
    }

    // No beta cutoff. Classify and store.
    let bound = if best_v > alpha_used {
        BOUND_EXACT
    } else {
        BOUND_UPPER
    };
    let move_sq = if best_move != u64::MAX && best_move != 0 {
        best_move.trailing_zeros() as u8
    } else {
        NO_MOVE_SQ
    };
    tt().store(key, best_v, depth as i8, bound, move_sq);

    (best_move, best_v)
}

// --------------------------------------------------------------------------
// Endgame: experiment notes (no code)
// --------------------------------------------------------------------------
//
// A specialised exact endgame solver was prototyped on the
// `ai_improvements` branch and reverted. The hypothesis was that
// pivoting into a leaner alpha-beta (no static eval, simpler ordering,
// selective TT) once the board had <= ~12 empty squares would beat the
// main search at its own game. Measured empirically on rolled-forward
// positions at 8, 10, and 14 empties, the naive solver consumed 10-20%
// more nodes and wall-clock than `nega_search_impl` even after being
// outfitted with PVS, killers, and mobility ordering - there is no
// overhead worth stripping once those features are present, and the
// main search's iterative-deepening TT warm-up gives it a structural
// advantage the one-shot solver can't match.
//
// The `--benchmark-endgame` harness is kept for anyone revisiting the
// problem: beating the main search here requires Reversi-specific
// machinery (parity-based move ordering, stability-based alpha-beta
// narrowing) rather than a generic alpha-beta rewrite.

fn nega_search(
    us: u64,
    them: u64,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
    cfg: EvalCfg,
) -> (u64, i32) {
    let mut ctx = SearchCtx::new(orig_depth, cfg);
    nega_search_impl::<false>(us, them, depth, alpha, beta, &mut ctx)
}

fn nega_search_cntr(
    us: u64,
    them: u64,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
    cfg: EvalCfg,
    counter: &mut u64,
) -> (u64, i32) {
    let mut ctx = SearchCtx::new(orig_depth, cfg);
    let result = nega_search_impl::<true>(us, them, depth, alpha, beta, &mut ctx);
    *counter += ctx.node_count;
    result
}

// --------------------------------------------------------------------------
// Public white/black wrappers (preserve external API semantics)
// --------------------------------------------------------------------------

#[inline(always)]
fn to_us_them(white: u64, black: u64, is_white_move: bool) -> (u64, u64) {
    if is_white_move {
        (white, black)
    } else {
        (black, white)
    }
}

#[inline(always)]
fn us_frame_bounds(alpha: i32, beta: i32, is_white_move: bool) -> (i32, i32) {
    if is_white_move {
        (-beta, -alpha)
    } else {
        (alpha, beta)
    }
}

#[inline(always)]
fn to_absolute(v_us: i32, is_white_move: bool) -> i32 {
    if is_white_move {
        -v_us
    } else {
        v_us
    }
}

pub fn search_moves_opt(
    white: u64,
    black: u64,
    is_white_move: bool,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
    cfg: EvalCfg,
) -> (u64, i32) {
    let (us, them) = to_us_them(white, black, is_white_move);
    let (a_us, b_us) = us_frame_bounds(alpha, beta, is_white_move);
    let (mv, v_us) = nega_search(us, them, depth, a_us, b_us, orig_depth, cfg);
    (mv, to_absolute(v_us, is_white_move))
}

pub fn search_moves_opt_cntr(
    white: u64,
    black: u64,
    is_white_move: bool,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
    cfg: EvalCfg,
    counter: &mut u64,
) -> (u64, i32) {
    let (us, them) = to_us_them(white, black, is_white_move);
    let (a_us, b_us) = us_frame_bounds(alpha, beta, is_white_move);
    let (mv, v_us) = nega_search_cntr(us, them, depth, a_us, b_us, orig_depth, cfg, counter);
    (mv, to_absolute(v_us, is_white_move))
}

// --------------------------------------------------------------------------
// Parallel root search
// --------------------------------------------------------------------------
//
// Rayon-parallel evaluation of root candidates. Individual subtrees still
// run the sequential TT-aware `nega_search`, so all threads share the same
// transposition table (Hyatt's XOR trick keeps probes internally consistent
// under Relaxed-ordered atomic writes).

pub fn search_moves_par(
    white: u64,
    black: u64,
    is_white_move: bool,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
    cfg: EvalCfg,
) -> (u64, i32) {
    let (us, them) = to_us_them(white, black, is_white_move);
    let outcome = game_status_us_them(us, them);

    if outcome == WHITE_WON_OUTCOME {
        return (u64::MAX, to_absolute(10_000, is_white_move));
    }
    if outcome == BLACK_WON_OUTCOME {
        return (u64::MAX, to_absolute(-10_000, is_white_move));
    }
    if outcome == DRAW_OUTCOME {
        return (u64::MAX, 0);
    }

    if depth == 0 {
        return (u64::MAX, eval_position_with_cfg(white, black, cfg));
    }

    if outcome == PASS_OUTCOME {
        if depth == orig_depth {
            return (u64::MAX, eval_position_with_cfg(white, black, cfg));
        }
        // Must match `nega_search_impl`: a pass swaps sides without
        // consuming a ply of the remaining search budget. Using
        // `depth - 1` here was a bug — it made the parallel root path
        // one ply shallower than `search_moves_opt` / `nega_search`
        // for the same position after a pass.
        let (_, eval) = search_moves_opt(
            white,
            black,
            !is_white_move,
            depth,
            alpha,
            beta,
            orig_depth,
            cfg,
        );
        return (u64::MAX, eval);
    }

    // Plain ascending bit order preserves rayon-reduce tie-break behaviour
    // w.r.t. the original find_legal_moves_alt-based implementation.
    let mut candidates: Vec<u64> = Vec::new();
    let mut remaining = outcome;
    while remaining != 0 {
        candidates.push(pop_lsb(&mut remaining));
    }

    let sign_us: i32 = if is_white_move { -1 } else { 1 };

    let (best_move, _best_eval_us, best_orig_eval) = candidates
        .into_par_iter()
        .map(|candidate| {
            let (new_us, new_them) = apply_move_us_them(us, them, candidate);
            let child_white = new_white(is_white_move, new_us, new_them);
            let child_black = new_black(is_white_move, new_us, new_them);

            if orig_depth - depth > 0 {
                let (_, orig) = search_moves_opt(
                    child_white,
                    child_black,
                    !is_white_move,
                    depth - 1,
                    alpha,
                    beta,
                    orig_depth,
                    cfg,
                );
                let eval_us_local = orig * sign_us;
                (candidate, eval_us_local, orig)
            } else {
                let (_, mut orig) = search_moves_par(
                    child_white,
                    child_black,
                    !is_white_move,
                    depth - 1,
                    alpha,
                    beta,
                    orig_depth,
                    cfg,
                );
                orig = adjust_mate_distance(orig);
                let eval_us_local = orig * sign_us;
                (candidate, eval_us_local, orig)
            }
        })
        .reduce(
            || (0, i32::MIN, i32::MIN),
            |acc, x| {
                let (_, acc_eval, _) = acc;
                let (cand, x_eval, x_orig) = x;
                if x_eval > acc_eval && cand != 0 {
                    (cand, x_eval, x_orig)
                } else {
                    acc
                }
            },
        );

    (best_move, best_orig_eval)
}

#[inline(always)]
fn new_white(is_white_move: bool, new_us: u64, new_them: u64) -> u64 {
    if is_white_move {
        new_us
    } else {
        new_them
    }
}

#[inline(always)]
fn new_black(is_white_move: bool, new_us: u64, new_them: u64) -> u64 {
    if is_white_move {
        new_them
    } else {
        new_us
    }
}

// --------------------------------------------------------------------------
// Iterative deepening drivers
// --------------------------------------------------------------------------
//
// The transposition table makes iterative deepening nearly-free: each prior
// iteration seeds the next with good move ordering (via the TT-move-first
// probe in `nega_search_impl`), and completed subtrees turn into cutoffs.
// These helpers are the recommended entry points for game-play code.

pub fn search_iterative(
    white: u64,
    black: u64,
    is_white_move: bool,
    max_depth: u32,
    cfg: EvalCfg,
) -> (u64, i32) {
    tt().new_age();
    let mut best = (u64::MAX, 0i32);
    for d in 1..=max_depth {
        best = search_moves_par(white, black, is_white_move, d, -20000, 20000, d, cfg);
    }
    best
}

pub fn search_iterative_cntr(
    white: u64,
    black: u64,
    is_white_move: bool,
    max_depth: u32,
    cfg: EvalCfg,
    counter: &mut u64,
) -> (u64, i32) {
    tt().new_age();
    let mut best = (u64::MAX, 0i32);
    for d in 1..=max_depth {
        best = search_moves_opt_cntr(
            white,
            black,
            is_white_move,
            d,
            -20000,
            20000,
            d,
            cfg,
            counter,
        );
    }
    best
}
