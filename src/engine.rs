use rayon::prelude::*;
use reversi_tools::position::*;

use crate::tt::{
    hash_position, tt, TTData, BOUND_EXACT, BOUND_LOWER, BOUND_NONE, BOUND_UPPER, NO_MOVE_SQ,
};

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

#[derive(Clone, Copy)]
pub struct EvalCfg {
    pub corner_value: i32,
    pub edge_value: i32,
    pub antiedge_value: i32,
    pub anticorner_value: i32,
}

pub static DEFAULT_CFG: EvalCfg = EvalCfg {
    corner_value: 70,
    edge_value: 17,
    antiedge_value: -22,
    anticorner_value: -34,
};

#[inline(always)]
fn side_score(bb: u64, cfg: EvalCfg) -> i32 {
    (bb & CORNER_MASK).count_ones() as i32 * cfg.corner_value
        + (bb & EDGE_MASK).count_ones() as i32 * cfg.edge_value
        + bb.count_ones() as i32
        + (bb & ANTIEDGE_MASK).count_ones() as i32 * cfg.antiedge_value
        + (bb & ANTICORNER_MASK).count_ones() as i32 * cfg.anticorner_value
}

pub fn eval_position_with_cfg(white: u64, black: u64, eval_cfg: EvalCfg) -> i32 {
    side_score(black, eval_cfg) - side_score(white, eval_cfg)
}

#[inline(always)]
fn eval_us_them(us: u64, them: u64, cfg: EvalCfg) -> i32 {
    side_score(us, cfg) - side_score(them, cfg)
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
    orig_depth: u32,
    cfg: EvalCfg,
    counter: &mut u64,
) -> (u64, i32) {
    if COUNT {
        *counter += 1;
    }

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
        let (_, child) = nega_search_impl::<COUNT>(
            them, us, depth, -beta, -alpha, orig_depth, cfg, counter,
        );
        return (u64::MAX, -child);
    }

    if depth == 0 {
        return (u64::MAX, eval_us_them(us, them, cfg));
    }

    // ---- TT probe -------------------------------------------------------
    let key = hash_position(us, them);
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

    let mut best_move: u64 = u64::MAX;
    let mut best_v: i32 = i32::MIN;

    // Helper: search a single candidate, update best_v/best_move, and bail
    // out on a beta cutoff (also storing the LOWER bound to the TT first).
    macro_rules! try_move {
        ($candidate:expr) => {{
            let candidate = $candidate;
            let (new_us, new_them) = apply_move_us_them(us, them, candidate);
            let (_, child_v) = nega_search_impl::<COUNT>(
                new_them,
                new_us,
                depth - 1,
                -b,
                -a,
                orig_depth,
                cfg,
                counter,
            );
            let v = adjust_mate_distance(-child_v);
            if v > best_v {
                best_v = v;
                best_move = candidate;
                if v > a {
                    a = v;
                }
                if a >= b {
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

    // Try the TT move first (if legal) - best candidate for beta cutoff.
    if tt_move_bit != 0 {
        try_move!(tt_move_bit);
    }

    let mut corner_moves = outcome & CORNER_MASK & !tt_move_bit;
    let mut edge_moves = outcome & EDGE_MASK & !ANTIEDGE_MASK & !tt_move_bit;
    let mut other_moves =
        outcome & !(CORNER_MASK | EDGE_MASK | ANTIEDGE_MASK | ANTICORNER_MASK) & !tt_move_bit;
    let mut bad_moves = outcome & (ANTIEDGE_MASK | ANTICORNER_MASK) & !tt_move_bit;

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

fn nega_search(
    us: u64,
    them: u64,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
    cfg: EvalCfg,
) -> (u64, i32) {
    let mut dummy = 0u64;
    nega_search_impl::<false>(us, them, depth, alpha, beta, orig_depth, cfg, &mut dummy)
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
    nega_search_impl::<true>(us, them, depth, alpha, beta, orig_depth, cfg, counter)
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
        let next_depth = depth.saturating_sub(1);
        let (_, eval) = search_moves_opt(
            white,
            black,
            !is_white_move,
            next_depth,
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
