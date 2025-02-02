use rayon::prelude::*;
use reversi_tools::position::*;

#[inline]
fn lowest_set_bit(x: u64) -> u64 {
    x & x.wrapping_neg()
}

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

pub fn eval_position_with_cfg(white: u64, black: u64, eval_cfg: EvalCfg) -> i32 {
    const CORNER_MASK: u64 = 0x8100000000000081;
    const EDGE_MASK: u64 = 0x42C300000000C342;
    const ANTIEDGE_MASK: u64 = 4792111478498951490;
    const ANTICORNER_MASK: u64 = 18577348462920192;

    let white_score = (white & CORNER_MASK).count_ones() as i32 * eval_cfg.corner_value
        + (white & EDGE_MASK).count_ones() as i32 * eval_cfg.edge_value
        + white.count_ones() as i32
        + (white & ANTIEDGE_MASK).count_ones() as i32 * eval_cfg.antiedge_value
        + (white & ANTICORNER_MASK).count_ones() as i32 * eval_cfg.anticorner_value;

    let black_score = (black & CORNER_MASK).count_ones() as i32 * eval_cfg.corner_value
        + (black & EDGE_MASK).count_ones() as i32 * eval_cfg.edge_value
        + black.count_ones() as i32
        + (black & ANTIEDGE_MASK).count_ones() as i32 * eval_cfg.antiedge_value
        + (black & ANTICORNER_MASK).count_ones() as i32 * eval_cfg.anticorner_value;

    black_score - white_score
}

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
    // WARNING: NO PRUNING GOING ON!
    let outcome = check_game_status(white, black, is_white_move);
    if outcome == (u64::MAX - 2) {
        return (u64::MAX, -10000);
    } else if outcome == (u64::MAX - 1) {
        return (u64::MAX, 10000);
    } else if outcome == (u64::MAX - 3) {
        return (u64::MAX, 0);
    }
    if depth == 0 {
        return (u64::MAX, eval_position_with_cfg(white, black, cfg));
    }
    let possible_moves: Vec<u64> = find_legal_moves_alt(white, black, is_white_move);
    if possible_moves.len() == 0 {
        if outcome == u64::MAX {
            if depth == orig_depth {
                return (u64::MAX, eval_position_with_cfg(white, black, cfg));
            }
            let eval: i32;
            if depth > 0 {
                (_, eval) = search_moves_opt(
                    white,
                    black,
                    !is_white_move,
                    depth - 1,
                    alpha,
                    beta,
                    orig_depth,
                    cfg,
                );
            } else {
                (_, eval) = search_moves_opt(
                    white,
                    black,
                    !is_white_move,
                    depth,
                    alpha,
                    beta,
                    orig_depth,
                    cfg,
                );
            }
            return (u64::MAX, eval);
        }
    }
    let (best_move, _best_eval, best_orig_eval) = possible_moves
        .into_par_iter()
        .map(|candidate| {
            let next_white: u64;
            let next_black: u64;
            let new_pos_opt = apply_move(white, black, candidate, is_white_move);
            match new_pos_opt {
                Ok((w, b)) => {
                    next_white = w;
                    next_black = b;
                }
                Err(_) => {
                    //println!("Move error: {}", s);
                    return (0, 0, 0);
                }
            }
            if depth == 0 {
                let orig_eval = eval_position_with_cfg(next_white, next_black, cfg);

                let eval = if is_white_move { -orig_eval } else { orig_eval };
                (candidate, eval, orig_eval)
            } else {
                if orig_depth - depth > 0 {
                    let (_, orig_eval) = search_moves_opt(
                        next_white,
                        next_black,
                        !is_white_move,
                        depth - 1,
                        alpha,
                        beta,
                        orig_depth,
                        cfg,
                    );
                    let eval = if is_white_move { -orig_eval } else { orig_eval };
                    (candidate, eval, orig_eval)
                } else {
                    let (_, mut orig_eval) = search_moves_par(
                        next_white,
                        next_black,
                        !is_white_move,
                        depth - 1,
                        alpha,
                        beta,
                        orig_depth,
                        cfg,
                    );
                    if orig_eval > 5000 {
                        orig_eval -= 1;
                    } else if orig_eval < -5000 {
                        orig_eval += 1;
                    }
                    let eval = if is_white_move { -orig_eval } else { orig_eval };
                    (candidate, eval, orig_eval)
                }
            }
        })
        .reduce(
            || (0, i32::MIN, i32::MIN),
            |acc, x| {
                let (_, acc_eval, _acc_orig) = acc;
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
    let outcome = check_game_status(white, black, is_white_move);
    if outcome == (u64::MAX - 2) {
        return (u64::MAX, -10000);
    } else if outcome == (u64::MAX - 1) {
        return (u64::MAX, 10000);
    } else if outcome == (u64::MAX - 3) {
        return (u64::MAX, 0);
    } else if outcome == u64::MAX {
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
    } else if outcome == (u64::MAX - 3) {
        let white_cnt = white.count_ones();
        let black_cnt = black.count_ones();
        if white_cnt > black_cnt {
            return (u64::MAX, -10000);
        } else if black_cnt > white_cnt {
            return (u64::MAX, 10000);
        } else {
            return (u64::MAX, 0);
        }
    }
    if depth == 0 {
        return (u64::MAX, eval_position_with_cfg(white, black, cfg));
    }
    let mut best_move: u64 = u64::MAX;
    let mut best_eval: i32 = i32::MIN;
    let mut best_orig_eval: i32 = 0;
    let mut local_alpha = alpha;
    let mut local_beta = beta;
    const CORNER_MASK: u64 = 0x8100000000000081;
    const EDGE_MASK: u64 = 0x42C300000000C342;
    const ANTIEDGE_MASK: u64 = 4792111478498951490;
    const ANTICORNER_MASK: u64 = 18577348462920192;
    let mut corner_moves = outcome & CORNER_MASK;
    let mut edge_moves = outcome & EDGE_MASK & (!ANTIEDGE_MASK);
    let mut other_moves = outcome & (!(CORNER_MASK | EDGE_MASK | ANTIEDGE_MASK | ANTICORNER_MASK));
    let mut shit_moves = outcome & (ANTIEDGE_MASK | ANTICORNER_MASK);
    while corner_moves > 0 || edge_moves > 0 || other_moves > 0 || shit_moves > 0 {
        let candidate: u64;
        if corner_moves > 0 {
            candidate = lowest_set_bit(corner_moves);
            corner_moves &= !candidate;
        } else if edge_moves > 0 {
            candidate = lowest_set_bit(edge_moves);
            edge_moves &= !candidate;
        } else if other_moves > 0 {
            candidate = lowest_set_bit(other_moves);
            other_moves &= !candidate;
        } else {
            candidate = lowest_set_bit(shit_moves);
            shit_moves &= !candidate;
        }
        let next_white: u64;
        let next_black: u64;
        let new_pos_opt = apply_move(white, black, candidate, is_white_move);
        match new_pos_opt {
            Ok((w, b)) => {
                next_white = w;
                next_black = b;
            }
            Err(_) => {
                continue;
            }
        }
        let eval: i32;
        let mut orig_eval: i32;
        if depth == 0 {
            orig_eval = eval_position_with_cfg(next_white, next_black, cfg);
        } else {
            let new_move: u64;
            (new_move, orig_eval) = search_moves_opt(
                next_white,
                next_black,
                !is_white_move,
                depth - 1,
                local_alpha,
                local_beta,
                orig_depth,
                cfg,
            );
            if new_move == 0 {
                continue;
            }
        }
        if orig_eval > 5000 {
            orig_eval -= 1;
        } else if orig_eval < -5000 {
            orig_eval += 1;
        }
        if is_white_move {
            eval = -1 * orig_eval;
        } else {
            eval = orig_eval;
        }
        if eval > best_eval {
            best_orig_eval = orig_eval;
            best_eval = eval;
            best_move = candidate;
            if is_white_move {
                if orig_eval < local_alpha {
                    return (candidate, orig_eval);
                } else {
                    local_beta = orig_eval;
                }
            } else {
                if orig_eval > local_beta {
                    return (candidate, orig_eval);
                } else {
                    local_alpha = orig_eval;
                }
            }
        }
    }
    (best_move, best_orig_eval)
}
