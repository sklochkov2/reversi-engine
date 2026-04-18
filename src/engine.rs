use rayon::prelude::*;
use reversi_tools::position::*;

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
/// slower losses. This is magnitude-preserving enough that it commutes with
/// sign flips (used in the negamax recursion).
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

/// Raw one-sided evaluation: how much the given bitmap is worth according to
/// the configuration.
#[inline(always)]
fn side_score(bb: u64, cfg: EvalCfg) -> i32 {
    (bb & CORNER_MASK).count_ones() as i32 * cfg.corner_value
        + (bb & EDGE_MASK).count_ones() as i32 * cfg.edge_value
        + bb.count_ones() as i32
        + (bb & ANTIEDGE_MASK).count_ones() as i32 * cfg.antiedge_value
        + (bb & ANTICORNER_MASK).count_ones() as i32 * cfg.anticorner_value
}

/// Public evaluation, in absolute (black-white) space. Preserved for callers.
pub fn eval_position_with_cfg(white: u64, black: u64, eval_cfg: EvalCfg) -> i32 {
    side_score(black, eval_cfg) - side_score(white, eval_cfg)
}

/// Evaluation from the point of view of the side to move ("us"). Positive
/// means we are ahead.
#[inline(always)]
fn eval_us_them(us: u64, them: u64, cfg: EvalCfg) -> i32 {
    side_score(us, cfg) - side_score(them, cfg)
}

// --------------------------------------------------------------------------
// Core negamax search (operates on us/them bitmaps)
// --------------------------------------------------------------------------
//
// All scores returned here are in the side-to-move's frame: +10000 means "we
// (the side currently to move) just won", -10000 means "we just lost". This
// removes the per-node `is_white_move` branch that the previous minimax
// implementation had to carry around.
//
// The helper functions from `reversi_tools` (`apply_move_unchecked`,
// `check_game_status`) are still parameterised over a colour flag; calling
// them with a constant `true` lets the compiler specialise away the
// redundant branches thanks to `#[inline(always)]` on those helpers.

#[inline(always)]
fn apply_move_us_them(us: u64, them: u64, move_bit: u64) -> (u64, u64) {
    // Passing `true` keeps the first argument as "me" and returns
    // (new_me, new_opp) without re-swapping.
    apply_move_unchecked(us, them, move_bit, true)
}

#[inline(always)]
fn game_status_us_them(us: u64, them: u64) -> u64 {
    // With is_white_move=true the helper treats the first argument as the
    // side to move. The terminal codes it returns depend on piece counts
    // (white vs black in the helper's nomenclature), so:
    //   WHITE_WON_OUTCOME  -> us won
    //   BLACK_WON_OUTCOME  -> them won
    //   DRAW_OUTCOME       -> draw
    //   PASS_OUTCOME       -> us must pass
    //   otherwise          -> bitmap of our legal moves
    check_game_status(us, them, true)
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
    let outcome = game_status_us_them(us, them);

    // Terminal / pass handling (uncommon; kept branch-heavy only here).
    if outcome >= DRAW_OUTCOME {
        if outcome == WHITE_WON_OUTCOME {
            return (u64::MAX, 10_000); // "us" won
        }
        if outcome == BLACK_WON_OUTCOME {
            return (u64::MAX, -10_000); // "them" won
        }
        if outcome == DRAW_OUTCOME {
            return (u64::MAX, 0);
        }
        // Pass: swap sides without consuming depth, then negate child's score
        // to transform it back into our frame.
        let (_, child) = nega_search(them, us, depth, -beta, -alpha, orig_depth, cfg);
        return (u64::MAX, -child);
    }

    if depth == 0 {
        return (u64::MAX, eval_us_them(us, them, cfg));
    }

    // Move ordering: corners first, then edges (but not squares that also
    // belong to the "antiedge" danger squares), then quiet moves, then bad
    // squares. Identical to the previous implementation.
    let mut corner_moves = outcome & CORNER_MASK;
    let mut edge_moves = outcome & EDGE_MASK & !ANTIEDGE_MASK;
    let mut other_moves = outcome & !(CORNER_MASK | EDGE_MASK | ANTIEDGE_MASK | ANTICORNER_MASK);
    let mut bad_moves = outcome & (ANTIEDGE_MASK | ANTICORNER_MASK);

    let mut best_move: u64 = u64::MAX;
    let mut best_v: i32 = i32::MIN;
    let mut a = alpha;

    macro_rules! run_bucket {
        ($moves:ident) => {
            while $moves != 0 {
                let candidate = pop_lsb(&mut $moves);

                let (new_us, new_them) = apply_move_us_them(us, them, candidate);

                // Child returns score in the *child's* frame; flip to ours.
                let (_, child_v) = nega_search(
                    new_them,
                    new_us,
                    depth - 1,
                    -beta,
                    -a,
                    orig_depth,
                    cfg,
                );
                let v = adjust_mate_distance(-child_v);

                if v > best_v {
                    best_v = v;
                    best_move = candidate;

                    if v > a {
                        a = v;
                    }
                    if a >= beta {
                        // Fail-soft beta cutoff: propagate the actual score.
                        return (candidate, v);
                    }
                }
            }
        };
    }

    run_bucket!(corner_moves);
    run_bucket!(edge_moves);
    run_bucket!(other_moves);
    run_bucket!(bad_moves);

    (best_move, best_v)
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
    *counter += 1;

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
        let (_, child) =
            nega_search_cntr(them, us, depth, -beta, -alpha, orig_depth, cfg, counter);
        return (u64::MAX, -child);
    }

    if depth == 0 {
        return (u64::MAX, eval_us_them(us, them, cfg));
    }

    let mut corner_moves = outcome & CORNER_MASK;
    let mut edge_moves = outcome & EDGE_MASK & !ANTIEDGE_MASK;
    let mut other_moves = outcome & !(CORNER_MASK | EDGE_MASK | ANTIEDGE_MASK | ANTICORNER_MASK);
    let mut bad_moves = outcome & (ANTIEDGE_MASK | ANTICORNER_MASK);

    let mut best_move: u64 = u64::MAX;
    let mut best_v: i32 = i32::MIN;
    let mut a = alpha;

    macro_rules! run_bucket {
        ($moves:ident) => {
            while $moves != 0 {
                let candidate = pop_lsb(&mut $moves);

                let (new_us, new_them) = apply_move_us_them(us, them, candidate);

                let (_, child_v) = nega_search_cntr(
                    new_them,
                    new_us,
                    depth - 1,
                    -beta,
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
                    if a >= beta {
                        return (candidate, v);
                    }
                }
            }
        };
    }

    run_bucket!(corner_moves);
    run_bucket!(edge_moves);
    run_bucket!(other_moves);
    run_bucket!(bad_moves);

    (best_move, best_v)
}

// --------------------------------------------------------------------------
// Public white/black wrappers (preserve external API semantics)
// --------------------------------------------------------------------------
//
// External callers talk in (white, black, is_white_move) and expect scores in
// absolute "black minus white" space. These helpers convert the frame once at
// the boundary; the hot inner search never has to look at the colour flag.

#[inline(always)]
fn to_us_them(white: u64, black: u64, is_white_move: bool) -> (u64, u64) {
    if is_white_move {
        (white, black)
    } else {
        (black, white)
    }
}

/// Map alpha/beta from absolute (black-white) space into the us-perspective
/// used by the negamax search, and return a callback to unwrap the resulting
/// score back to absolute space.
#[inline(always)]
fn us_frame_bounds(alpha: i32, beta: i32, is_white_move: bool) -> (i32, i32) {
    if is_white_move {
        // Absolute interval [alpha, beta] => us-interval [-beta, -alpha].
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
// Only invoked at the root (and once more one ply deeper as part of the
// existing logic); not performance-critical on a per-node basis, but still
// benefits from the negamax core.

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

    // Pass handling at root preserves the quirks of the previous
    // implementation: at the root we just return the static eval; elsewhere
    // we recurse (consuming a ply of depth, matching the prior code).
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

    // Collect legal moves in plain ascending bit order so that rayon's
    // `reduce` - which picks the leftmost candidate among equally-valued
    // moves - behaves identically to the original `find_legal_moves_alt`
    // based implementation. Bucket ordering matters for alpha-beta move
    // ordering in the sequential inner search (see `nega_search`), but in
    // the parallel root every candidate is searched anyway, so using the
    // same enumeration order as before preserves tie-break semantics.
    let mut candidates: Vec<u64> = Vec::new();
    let mut remaining = outcome;
    while remaining != 0 {
        candidates.push(pop_lsb(&mut remaining));
    }

    // Us-frame sign so we can maximise a single scalar in the reduction.
    let sign_us: i32 = if is_white_move { -1 } else { 1 };

    let (best_move, _best_eval_us, best_orig_eval) = candidates
        .into_par_iter()
        .map(|candidate| {
            let (new_us, new_them) = apply_move_us_them(us, them, candidate);
            let child_white = new_white(is_white_move, new_us, new_them);
            let child_black = new_black(is_white_move, new_us, new_them);

            if orig_depth - depth > 0 {
                // Non-root: sequential alpha-beta via the negamax core.
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
                // Root-recursion branch preserved from the original.
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

// Helpers for mapping (us, them) pairs back to (white, black) in the parallel
// root loop without resurrecting a branch inside the hot negamax kernel.
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
