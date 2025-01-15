use chrono;
use clap::Parser;
use rayon::prelude::*;
use std::path::Path;

mod model;
use model::*;

mod openingbook;
use openingbook::*;

use std::{thread, time};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// API base URL, e. g. http://example.com:8080/
    #[arg(short, long, default_value_t = String::new())]
    api_url: String,

    /// Player UUID as provided by server API
    #[arg(short, long, default_value_t = String::new())]
    player_uuid: String,

    /// Search depth
    #[arg(short, long, default_value_t = 8)]
    search_depth: u32,

    /// Opening book path
    #[arg(short, long, default_value_t = String::new())]
    book_path: String,

    /// Whether to generate an opening book
    #[arg(short, long, default_value_t = false)]
    generate_book: bool,

    /// When generating an opening book, how deeply to evaluate all moves
    #[arg(short, long, default_value_t = 5)]
    full_depth: u32,

    #[arg(short, long, default_value_t = 7)]
    /// When generating an opening book, how deeply to analyze main lines
    k_partial_depth: u32,
}

fn print_board(white: u64, black: u64, last_move: u64, flips: u64, mark_last_move: bool) {
    let mut res: String = "========\n".to_string();
    for i in 0..8 {
        for j in 0..8 {
            let index = ((7 - i) * 8 + j) as usize;
            let bit = 1u64 << index;
            if mark_last_move && bit == last_move {
                res += "\x1b[41m";
            } else if bit & flips > 0 {
                res += "\x1b[42m";
            }
            if (white & bit) != 0 {
                res += "o";
            } else if (black & bit) != 0 {
                res += "x";
            } else {
                res += ".";
            }
            if (mark_last_move && bit == last_move) || bit & flips > 0 {
                res += "\x1b[0m";
            }
        }
        res += "\n";
    }
    res += "========";
    println!("{}", res);
}

fn flip_in_dir(move_bit: u64, me: u64, opp: u64, shift: fn(u64) -> u64) -> u64 {
    let mut mask = shift(move_bit);
    let mut flipped = 0_u64;

    while (mask & opp) != 0 {
        flipped |= mask;
        mask = shift(mask);
    }

    if (mask & me) != 0 {
        flipped
    } else {
        0
    }
}

pub fn apply_move_opt(
    white: u64,
    black: u64,
    move_bit: u64,
    is_white_move: bool,
) -> Result<(u64, u64), &'static str> {
    let occupied = white | black;
    if (move_bit & occupied) != 0 {
        return Err("Square already occupied");
    }

    let (me, opp) = if is_white_move {
        (white, black)
    } else {
        (black, white)
    };

    let mut flip_mask = 0_u64;
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_north);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_south);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_east);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_west);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_ne);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_nw);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_se);
    flip_mask |= flip_in_dir(move_bit, me, opp, shift_sw);

    if flip_mask == 0 {
        return Err("No flips!");
    }

    let new_me = me | move_bit | flip_mask;
    let new_opp = opp & !flip_mask;

    if is_white_move {
        Ok((new_me, new_opp))
    } else {
        Ok((new_opp, new_me))
    }
}

fn apply_move(
    white: u64,
    black: u64,
    move_bit: u64,
    is_white_move: bool,
) -> Result<(u64, u64), &'static str> {
    const DIRECTIONS: [(i32, i32); 8] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];

    let (player, opponent) = if is_white_move {
        (white, black)
    } else {
        (black, white)
    };

    if (player | opponent) & move_bit != 0 {
        return Err("Square already occupied");
    }

    let mut flips = 0u64;

    for &(dx, dy) in DIRECTIONS.iter() {
        let mut current_flips = 0u64;
        let mut x = (move_bit.trailing_zeros() % 8) as i32 + dx;
        let mut y = (move_bit.trailing_zeros() / 8) as i32 + dy;
        let mut found_opponent = false;

        while x >= 0 && x < 8 && y >= 0 && y < 8 {
            let index = (y * 8 + x) as usize;
            let bit = 1u64 << index;

            if (opponent & bit) != 0 {
                current_flips |= bit;
                found_opponent = true;
            } else if (player & bit) != 0 {
                if found_opponent {
                    flips |= current_flips;
                }
                break;
            } else {
                break;
            }

            x += dx;
            y += dy;
        }
    }

    if flips == 0 {
        return Err("Invalid move, no discs flipped");
    }

    let player = player | move_bit | flips;
    let opponent = opponent & !flips;

    let (next_white, next_black) = if is_white_move {
        (player, opponent)
    } else {
        (opponent, player)
    };
    print_board(next_white, next_black, move_bit, flips, true);
    Ok((next_white, next_black))
}

#[inline]
fn lowest_set_bit(x: u64) -> u64 {
    x & x.wrapping_neg()
}

const NOT_A_FILE: u64 = 0xfefefefefefefefe;
const NOT_H_FILE: u64 = 0x7f7f7f7f7f7f7f7f;

#[inline]
fn shift_north(x: u64) -> u64 {
    x << 8
}
#[inline]
fn shift_south(x: u64) -> u64 {
    x >> 8
}
#[inline]
fn shift_east(x: u64) -> u64 {
    (x & NOT_H_FILE) << 1
}
#[inline]
fn shift_west(x: u64) -> u64 {
    (x & NOT_A_FILE) >> 1
}
#[inline]
fn shift_ne(x: u64) -> u64 {
    (x & NOT_H_FILE) << 9
}
#[inline]
fn shift_nw(x: u64) -> u64 {
    (x & NOT_A_FILE) << 7
}
#[inline]
fn shift_se(x: u64) -> u64 {
    (x & NOT_H_FILE) >> 7
}
#[inline]
fn shift_sw(x: u64) -> u64 {
    (x & NOT_A_FILE) >> 9
}

fn compute_moves(me: u64, opp: u64) -> u64 {
    let empty = !(me | opp);

    let mut moves = 0_u64;

    let mut mask = shift_north(me) & opp;
    mask |= shift_north(mask) & opp;
    mask |= shift_north(mask) & opp;
    mask |= shift_north(mask) & opp;
    mask |= shift_north(mask) & opp;
    mask |= shift_north(mask) & opp;
    moves |= shift_north(mask) & empty;

    // South
    mask = shift_south(me) & opp;
    mask |= shift_south(mask) & opp;
    mask |= shift_south(mask) & opp;
    mask |= shift_south(mask) & opp;
    mask |= shift_south(mask) & opp;
    mask |= shift_south(mask) & opp;
    moves |= shift_south(mask) & empty;

    // East
    mask = shift_east(me) & opp;
    mask |= shift_east(mask) & opp;
    mask |= shift_east(mask) & opp;
    mask |= shift_east(mask) & opp;
    mask |= shift_east(mask) & opp;
    mask |= shift_east(mask) & opp;
    moves |= shift_east(mask) & empty;

    // West
    mask = shift_west(me) & opp;
    mask |= shift_west(mask) & opp;
    mask |= shift_west(mask) & opp;
    mask |= shift_west(mask) & opp;
    mask |= shift_west(mask) & opp;
    mask |= shift_west(mask) & opp;
    moves |= shift_west(mask) & empty;

    // Northeast
    mask = shift_ne(me) & opp;
    mask |= shift_ne(mask) & opp;
    mask |= shift_ne(mask) & opp;
    mask |= shift_ne(mask) & opp;
    mask |= shift_ne(mask) & opp;
    mask |= shift_ne(mask) & opp;
    moves |= shift_ne(mask) & empty;

    // Northwest
    mask = shift_nw(me) & opp;
    mask |= shift_nw(mask) & opp;
    mask |= shift_nw(mask) & opp;
    mask |= shift_nw(mask) & opp;
    mask |= shift_nw(mask) & opp;
    mask |= shift_nw(mask) & opp;
    moves |= shift_nw(mask) & empty;

    // Southeast
    mask = shift_se(me) & opp;
    mask |= shift_se(mask) & opp;
    mask |= shift_se(mask) & opp;
    mask |= shift_se(mask) & opp;
    mask |= shift_se(mask) & opp;
    mask |= shift_se(mask) & opp;
    moves |= shift_se(mask) & empty;

    // Southwest
    mask = shift_sw(me) & opp;
    mask |= shift_sw(mask) & opp;
    mask |= shift_sw(mask) & opp;
    mask |= shift_sw(mask) & opp;
    mask |= shift_sw(mask) & opp;
    mask |= shift_sw(mask) & opp;
    moves |= shift_sw(mask) & empty;

    moves
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

fn check_game_status(white: u64, black: u64) -> u32 {
    let all_discs = white | black;

    if all_discs == 0xFFFFFFFFFFFFFFFF {
        let player_count = white.count_ones();
        let opponent_count = black.count_ones();

        return if player_count > opponent_count {
            1
        } else if opponent_count > player_count {
            2
        } else {
            0
        };
    }

    if compute_moves(white, black) > 0 || compute_moves(black, white) > 0 {
        3
    } else {
        let player_count = white.count_ones();
        let opponent_count = black.count_ones();

        if player_count > opponent_count {
            1
        } else if opponent_count > player_count {
            2
        } else {
            0
        }
    }
}

fn check_game_status_alt(white: u64, black: u64, is_white_move: bool) -> u64 {
    let (me, opp) = if is_white_move {
        (white, black)
    } else {
        (black, white)
    };
    let my_moves: u64 = compute_moves(me, opp);
    if my_moves > 0 {
        return my_moves;
    }
    let opp_moves: u64 = compute_moves(opp, me);
    if opp_moves > 0 {
        println!("AAAAAA");
        return u64::MAX;
    }
    let white_count = white.count_ones();
    let black_count = black.count_ones();

    if white_count > black_count {
        println!("BBBBBB");
        return u64::MAX - 2;
    } else if black_count > white_count {
        println!("CCCCCCC");
        return u64::MAX - 1;
    } else {
        println!("DDDDDD");
        return u64::MAX - 3;
    };
}

fn move_to_algebraic(move_bit: u64) -> Option<String> {
    if move_bit.count_ones() != 1 {
        return None;
    }

    let pos = move_bit.trailing_zeros() as usize;
    let file = (pos % 8) as u8 + b'a';
    let rank = (pos / 8) as u8 + b'1';

    Some(format!("{}{}", file as char, rank as char))
}

fn move_to_bitmap(move_notation: &str) -> Result<u64, &str> {
    if move_notation.len() != 2 {
        return Err("Invalid move notation");
    }
    let file = move_notation.chars().next().unwrap().to_ascii_lowercase() as usize - 'a' as usize;
    let rank = move_notation.chars().nth(1).unwrap().to_digit(10).unwrap() as usize - 1;
    if file >= 8 || rank >= 8 {
        return Err("Invalid move notation");
    }

    let move_pos = rank * 8 + file;
    let move_bit = 1u64 << move_pos;
    Ok(move_bit)
}

fn eval_position(white: u64, black: u64) -> i32 {
    const CORNER_MASK: u64 = 0x8100000000000081;
    const EDGE_MASK: u64 = 0x42C300000000C342;

    let white_score = (white & CORNER_MASK).count_ones() as i32 * 10
        + (white & EDGE_MASK).count_ones() as i32 * 5
        + white.count_ones() as i32;

    let black_score = (black & CORNER_MASK).count_ones() as i32 * 10
        + (black & EDGE_MASK).count_ones() as i32 * 5
        + black.count_ones() as i32;

    black_score - white_score
}

fn search_moves_par(
    white: u64,
    black: u64,
    is_white_move: bool,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
) -> (u64, i32) {
    // WARNING: NO PRUNING GOING ON!
    let outcome = check_game_status(white, black);
    if outcome == 1 {
        return (u64::MAX, -1000);
    } else if outcome == 2 {
        return (u64::MAX, 1000);
    } else if outcome == 0 {
        return (u64::MAX, 0);
    }
    let possible_moves: Vec<u64> = find_legal_moves_alt(white, black, is_white_move);
    if possible_moves.len() == 0 {
        if outcome == 3 {
            if depth == orig_depth {
                return (u64::MAX, eval_position(white, black));
            }
            if depth > 0 {
                return search_moves_opt(
                    white,
                    black,
                    !is_white_move,
                    depth - 1,
                    alpha,
                    beta,
                    orig_depth,
                );
            } else {
                return search_moves_opt(
                    white,
                    black,
                    !is_white_move,
                    depth,
                    alpha,
                    beta,
                    orig_depth,
                );
            }
        }
    }
    let (best_move, _best_eval, best_orig_eval) = possible_moves
        .into_par_iter()
        .map(|candidate| {
            let next_white: u64;
            let next_black: u64;
            let new_pos_opt = apply_move_opt(white, black, candidate, is_white_move);
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
                let orig_eval = eval_position(next_white, next_black);

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
                    );
                    if orig_eval > 500 {
                        orig_eval -= 1;
                    } else if orig_eval < -500 {
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

fn search_moves_opt(
    white: u64,
    black: u64,
    is_white_move: bool,
    depth: u32,
    alpha: i32,
    beta: i32,
    orig_depth: u32,
) -> (u64, i32) {
    let outcome = check_game_status_alt(white, black, is_white_move);
    if outcome == (u64::MAX - 2) {
        return (u64::MAX, -1000);
    } else if outcome == (u64::MAX - 1) {
        return (u64::MAX, 1000);
    } else if outcome == (u64::MAX - 3) {
        return (u64::MAX, 0);
    } else if outcome == u64::MAX {
        //return (u64::MAX, eval_position(white, black));
        return search_moves_opt(white, black, !is_white_move, depth, alpha, beta, orig_depth);
    } else if outcome == (u64::MAX - 3) {
        //return (u64::MAX, eval_position(white, black));
        let white_cnt = white.count_ones();
        let black_cnt = black.count_ones();
        if white_cnt > black_cnt {
            return (u64::MAX, -1000);
        } else if black_cnt > white_cnt {
            return (u64::MAX, 1000);
        } else {
            return (u64::MAX, 0);
        }
    }
    let mut best_move: u64 = 0;
    let mut best_eval: i32 = i32::MIN;
    let mut best_orig_eval: i32 = 0;
    let mut local_alpha = alpha;
    let mut local_beta = beta;
    let mut tmp = outcome;
    while tmp > 0 {
        let candidate = lowest_set_bit(tmp);
        tmp &= !candidate;
        let next_white: u64;
        let next_black: u64;
        let new_pos_opt = apply_move_opt(white, black, candidate, is_white_move);
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
            orig_eval = eval_position(next_white, next_black);
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
            );
            if new_move == 0 {
                continue;
            }
        }
        // Prioritizing shorter win paths
        if orig_eval > 500 {
            orig_eval -= 1;
        } else if orig_eval < -500 {
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

fn generate_opening_book(
    calculation_depth: u32,
    full_depth: u32,
    partial_depth: u32,
    save_path: &str,
) {
    println!("Generating opening book;calc depth: {}, full search depth: {}, partial search depth: {}, path: {}", calculation_depth, full_depth, partial_depth, save_path);
    let black = 0x0000000810000000u64;
    let white = 0x0000001008000000u64;
    let white_to_move: bool = false;
    let mut queue: Vec<Position> = Vec::new();
    let mut book: OpeningBook;
    if Path::new(save_path).exists() {
        book = OpeningBook::load_from_file(save_path).unwrap();
    } else {
        book = OpeningBook::default();
    }

    let starting_pos: Position = Position {
        black: black,
        white: white,
        white_to_move: white_to_move,
    };
    queue.push(starting_pos);
    for depth in 0..partial_depth {
        let mut next_queue: Vec<Position> = Vec::new();
        println!(
            "{:?} Reached depth {} with {} positions",
            chrono::offset::Local::now(),
            depth,
            queue.len()
        );
        for pos in queue {
            println!(
                "{:?} Evaluating new position: b {} w {} wtm: {}",
                chrono::offset::Local::now(),
                pos.black,
                pos.white,
                pos.white_to_move
            );
            let cached_result = book.get(&pos);
            match cached_result {
                Some(_) => {
                    println!("{:?} Cached position found!", chrono::offset::Local::now());
                }
                None => {
                    println!(
                        "{:?} Position absent from cache",
                        chrono::offset::Local::now()
                    );
                    let (best_move, _) = search_moves_par(
                        pos.white,
                        pos.black,
                        pos.white_to_move,
                        calculation_depth,
                        -2000,
                        2000,
                        calculation_depth,
                    );
                    println!(
                        "{:?} Best move found: {}",
                        chrono::offset::Local::now(),
                        best_move
                    );
                    book.insert_all_rotations(pos, best_move);
                    if depth >= full_depth {
                        println!(
                            "{:?} Inserting move for partial search",
                            chrono::offset::Local::now()
                        );
                        let new_pos_opt =
                            apply_move_opt(pos.white, pos.black, best_move, pos.white_to_move);
                        match new_pos_opt {
                            Ok((w, b)) => {
                                next_queue.push(Position {
                                    black: b,
                                    white: w,
                                    white_to_move: !pos.white_to_move,
                                });
                            }
                            Err(_) => {
                                //println!("Move error: {}", s);
                                continue;
                            }
                        }
                    }
                }
            }
            let next_moves = find_legal_moves_alt(pos.white, pos.black, pos.white_to_move);
            if depth >= full_depth {
                continue;
            }
            println!(
                "{} Generating all possible moves",
                chrono::offset::Local::now()
            );
            for next_move in next_moves {
                let new_pos_opt =
                    apply_move_opt(pos.white, pos.black, next_move, pos.white_to_move);
                match new_pos_opt {
                    Ok((w, b)) => {
                        next_queue.push(Position {
                            black: b,
                            white: w,
                            white_to_move: !pos.white_to_move,
                        });
                    }
                    Err(_) => {
                        //println!("Move error: {}", s);
                        continue;
                    }
                }
            }
            let write_res = book.save_to_file(save_path);
            match write_res {
                Ok(_) => {}
                Err(e) => {
                    println!("Error while saving to file: {}", e);
                }
            }
        }
        queue = next_queue;
    }
    let write_res = book.save_to_file(save_path);
    match write_res {
        Ok(_) => {}
        Err(e) => {
            println!("Error while saving to file: {}", e);
        }
    }
}

fn local_game(args: Args) {
    let mut black = 0x0000000810000000u64;
    let mut white = 0x0000001008000000u64;
    let mut white_to_move: bool = false;

    // Ply: 51, Is white: false, Move: a8, Eval: 991, Black pos: 33909430323788925, White pos: 4325574457067520514
    // Ply: 9, Is white: false, Move: h7, Eval: 999, Black pos: 4713330624348249857, White pos: 4474012615487561982
    // Ply: 55, Is white: false, Move: h5, Eval: 995, Black pos: 29361505010844157, White pos: 4330122384527982082
    // Ply: 2, Is white: true, Move: h7, Eval: 996, Black pos: 29362332874211837, White pos: 433012210642042829

    /*let mut black: u64 = 120795966464;
    let mut white: u64 = 36310151199708159;
    let mut white_to_move: bool = false;*/
    let book: OpeningBook;
    if args.book_path.is_empty() {
        book = OpeningBook::default();
    } else {
        book = OpeningBook::load_from_file(args.book_path.as_str()).unwrap();
    }

    print_board(white, black, 0, 0, false);
    let default_depth: u32 = args.search_depth;
    let mut ply = 0;
    loop {
        let piece_count = (white | black).count_ones();
        let depth: u32;
        if (64 - piece_count) > default_depth {
            depth = default_depth;
        } else {
            depth = 64 - piece_count;
        }
        ply += 1;
        let nxt_move: u64;
        let eval: i32;
        if !white_to_move {
            let next_move_opt = book.get(&Position {
                black: black,
                white: white,
                white_to_move: white_to_move,
            });
            match next_move_opt {
                Some(m) => {
                    println!("Book move found!");
                    nxt_move = m.suggested_moves[0];
                    eval = 0;
                }
                None => {
                    (nxt_move, eval) =
                        search_moves_par(white, black, white_to_move, depth, -2000, 2000, depth);
                    if nxt_move == 0 {
                        println!("NO MOVES!");
                        break;
                    }
                }
            }
        } else {
            (nxt_move, eval) =
                search_moves_par(white, black, white_to_move, depth, -2000, 2000, depth);
            if nxt_move == 0 {
                println!("NO MOVES!");
                break;
            }
        }
        if nxt_move != u64::MAX {
            println!(
                "Ply: {}, Is white: {}, Move: {}, Eval: {}, Black pos: {}, White pos: {}",
                ply,
                white_to_move,
                move_to_algebraic(nxt_move).unwrap(),
                eval,
                black,
                white
            );
            let (new_white, new_black) = apply_move(white, black, nxt_move, white_to_move).unwrap();
            //println!("WWW {} {} {}", new_white, new_black, white_to_move);
            let game_status = check_game_status(new_white, new_black);
            if game_status == 3 {
                black = new_black;
                white = new_white;
                white_to_move = !white_to_move;
            } else {
                let black_score = new_black.count_ones();
                let white_score = new_white.count_ones();
                println!("Black score: {}, white score: {}", black_score, white_score);
                if game_status == 1 {
                    println!("White won b {} w {}", new_black, new_white);
                } else if game_status == 2 {
                    println!("Black won b {} w {}", new_black, new_white);
                } else if game_status == 0 {
                    println!("Draw b {} w {}", new_black, new_white);
                }
                break;
            }
        } else {
            println!("Is white: {}; PASS", white_to_move);
            white_to_move = !white_to_move;
        }
    }
}

fn find_games_to_join(args: &Args) -> Result<Vec<String>, ureq::Error> {
    let mut res: Vec<String> = Vec::new();
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/game_list";
    println!("{}", api_endpoint);
    let join_request: NewGameRequest = NewGameRequest {
        player_id: args.player_uuid.clone(),
    };
    let list_games_result: GameListResponse = ureq::post(api_endpoint.as_str())
        .send_json(&join_request)?
        .body_mut()
        .read_json::<GameListResponse>()?;
    for game in list_games_result.result {
        if game.first_player != args.player_uuid {
            res.push(game.game_id);
        }
    }
    Ok(res)
}

fn create_game(args: &Args) -> Result<NewGameResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/create_game";
    let create_request: NewGameRequest = NewGameRequest {
        player_id: args.player_uuid.clone(),
    };
    let created_game: NewGameResponse = ureq::post(api_endpoint.as_str())
        .send_json(&create_request)?
        .body_mut()
        .read_json::<NewGameResponse>()?;
    Ok(created_game.result)
}

fn join_game(args: &Args, game_uuid: String) -> Result<GameJoinResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/join";
    let game_request: GameRequest = GameRequest {
        player_id: args.player_uuid.clone(),
        game_id: game_uuid.clone(),
    };
    let joined_game: GameJoinResponse = ureq::post(api_endpoint.as_str())
        .send_json(&game_request)?
        .body_mut()
        .read_json::<GameJoinResponse>()?;
    Ok(joined_game.result)
}

//make_move(&args, game.clone(), nxt_move_algebraic.clone());
fn make_move(args: &Args, game_uuid: String, our_move: String) -> Result<MoveResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/move";
    let move_request: MoveRequest = MoveRequest {
        player_id: args.player_uuid.clone(),
        game_id: game_uuid.clone(),
        r#move: our_move.clone(),
    };
    let move_response: MoveResponse = ureq::post(api_endpoint.as_str())
        .send_json(&move_request)?
        .body_mut()
        .read_json::<MoveResponse>()?;
    Ok(move_response.result)
}

fn get_game_status(args: &Args, game_uuid: String) -> Result<GameStatusResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/game_status";
    let game_request: GameRequest = GameRequest {
        player_id: args.player_uuid.clone(),
        game_id: game_uuid.clone(),
    };
    let status: GameStatusResponse = ureq::post(api_endpoint.as_str())
        .send_json(&game_request)?
        .body_mut()
        .read_json::<GameStatusResponse>()?;
    Ok(status.result)
}

//GameStatusResult = wait_for_response(&args, game.clone(), my_color.clone());
fn wait_for_response(args: &Args, game_uuid: String, my_color: String) -> GameStatusResult {
    loop {
        let curr_result: GameStatusResult;
        match get_game_status(args, game_uuid.clone()) {
            Ok(g) => {
                curr_result = g;
            }
            Err(e) => {
                println!("Failed to fetch game status, retrying: {}", e);
                thread::sleep(time::Duration::from_millis(1000));
                continue;
            }
        }
        if curr_result.status == my_color
            || curr_result.status == "black_won".to_string()
            || curr_result.status == "white_won".to_string()
        {
            return curr_result;
        }
        thread::sleep(time::Duration::from_millis(500));
    }
}

fn wait_for_joining_player(args: &Args, game_uuid: String) -> GameStatusResult {
    loop {
        let curr_result: GameStatusResult;
        match get_game_status(args, game_uuid.clone()) {
            Ok(g) => {
                curr_result = g;
            }
            Err(e) => {
                println!("Failed to fetch game status, retrying: {}", e);
                thread::sleep(time::Duration::from_millis(1000));
                continue;
            }
        }
        if curr_result.status != "pending".to_string() {
            return curr_result;
        }
        thread::sleep(time::Duration::from_millis(500));
    }
}

fn play_multiplayer(args: Args) {
    println!(
        "{} {} {} {}",
        args.api_url, args.search_depth, args.book_path, args.player_uuid
    );
    let games: Vec<String>;
    loop {
        match find_games_to_join(&args) {
            Ok(g) => {
                games = g;
                break;
            }
            Err(e) => {
                println!("Failed to retrieve game list, retrying: {}", e);
                thread::sleep(time::Duration::from_millis(1000));
            }
        }
    }
    let mut my_game_uuid: String = String::new();
    let mut my_color: String = String::new();
    let mut opp_first_move: u64 = 0;
    if games.len() == 0 {
        println!("No games to join, creating one!");
        let new_game: NewGameResult;
        loop {
            match create_game(&args) {
                Ok(g) => {
                    new_game = g;
                    break;
                }
                Err(e) => {
                    println!("Error while creating a game, retrying: {}", e);
                    thread::sleep(time::Duration::from_millis(1000));
                }
            }
        }
        my_game_uuid = new_game.game_id;
        my_color = new_game.color;
        println!("Waiting for ooponent to join");
        let opp_join_status = wait_for_joining_player(&args, my_game_uuid.clone());
        if opp_join_status.last_move != String::new() {
            opp_first_move = move_to_bitmap(opp_join_status.last_move.as_str()).unwrap();
        }
    } else {
        for game in games {
            let joined_game: GameJoinResult;
            loop {
                match join_game(&args, game.clone()) {
                    Ok(g) => {
                        joined_game = g;
                        break;
                    }
                    Err(e) => {
                        println!("Error while joining a game, retrying: {}", e);
                        thread::sleep(time::Duration::from_millis(1000));
                    }
                }
            }
            if joined_game.result {
                my_game_uuid = game.clone();
                my_color = joined_game.color;
                break;
            }
        }
    }
    if my_game_uuid.is_empty() {
        println!("Failed to create or join game!");
    } else {
        println!("Playing game {} as {}", my_game_uuid, my_color);
        let mut black = 0x0000000810000000u64;
        let mut white = 0x0000001008000000u64;
        let mut white_to_move: bool = false;
        if opp_first_move > 0 {
            println!("Applying opponent's initial move");
            let (new_white, new_black) =
                apply_move(white, black, opp_first_move, white_to_move).unwrap();
            white = new_white;
            black = new_black;
            white_to_move = !white_to_move;
        }
        print_board(white, black, 0, 0, false);
        let book: OpeningBook;
        if args.book_path != String::new() {
            book = OpeningBook::load_from_file(args.book_path.clone().as_str()).unwrap();
        } else {
            book = OpeningBook::default();
        }
        loop {
            if white_to_move == (my_color == "white".to_string()) {
                let nxt_move: u64;
                let eval: i32;
                let next_move_opt = book.get(&Position {
                    black: black,
                    white: white,
                    white_to_move: white_to_move,
                });
                match next_move_opt {
                    Some(m) => {
                        println!("Book move found!");
                        nxt_move = m.suggested_moves[0];
                        eval = 0;
                    }
                    None => {
                        let piece_count = (white | black).count_ones();
                        let depth: u32;
                        if (64 - piece_count) > args.search_depth {
                            depth = args.search_depth;
                        } else {
                            depth = 64 - piece_count;
                        }
                        (nxt_move, eval) = search_moves_par(
                            white,
                            black,
                            white_to_move,
                            depth,
                            -2000,
                            2000,
                            depth,
                        );
                        if nxt_move == 0 {
                            println!("NO MOVES!");
                        }
                    }
                }
                let mut nxt_move_algebraic: String;
                if nxt_move == 0 {
                    nxt_move_algebraic = "resign".to_string();
                    println!("Failed to find a move, we resign!");
                } else if nxt_move == u64::MAX {
                    nxt_move_algebraic = "pass".to_string();
                    println!("No legal moves, we pass!");
                } else {
                    let (new_white, new_black) =
                        apply_move(white, black, nxt_move, white_to_move).unwrap();
                    nxt_move_algebraic = move_to_algebraic(nxt_move).unwrap();
                    println!(
                        "Move {} {}, eval {}, black pos: {}, white pos: {}, white move: {}",
                        nxt_move_algebraic, nxt_move, eval, black, white, white_to_move
                    );
                    white = new_white;
                    black = new_black;
                    let game_status = check_game_status(new_white, new_black);
                    if (game_status == 2 && my_color == "white".to_string())
                        || (game_status == 1 && my_color == "black".to_string())
                    {
                        nxt_move_algebraic = "resign".to_string();
                    }
                }
                let move_result: MoveResult;
                loop {
                    match make_move(&args, my_game_uuid.clone(), nxt_move_algebraic.clone()) {
                        Ok(g) => {
                            move_result = g;
                            break;
                        }
                        Err(e) => {
                            println!("Error while making a move, retrying: {}", e);
                            thread::sleep(time::Duration::from_millis(1000));
                        }
                    }
                }
                if !move_result.r#continue {
                    println!("Game ended, {} won!", move_result.winner);
                    println!(
                        "Black score: {}. white score: {}",
                        black.count_ones(),
                        white.count_ones()
                    );
                    break;
                } else {
                    white_to_move = !white_to_move;
                }
                // Our move!
            } else {
                println!("Patiently waiting for opponent's move");
                let next_status: GameStatusResult =
                    wait_for_response(&args, my_game_uuid.clone(), my_color.clone());
                if next_status.status == "black_won".to_string() {
                    println!("Game ended, black won!");
                    break;
                } else if next_status.status == "white_won".to_string() {
                    println!("Game ended, white won!");
                    break;
                }
                if next_status.last_move == "pass".to_string() {
                    println!("Opponnent passes their move!");
                    white_to_move = !white_to_move;
                    continue;
                }
                let opp_move: u64 = move_to_bitmap(next_status.last_move.as_str()).unwrap();
                println!("Here it is: {} {}!", next_status.last_move, opp_move);
                let (new_white, new_black) =
                    apply_move(white, black, opp_move, white_to_move).unwrap();
                white = new_white;
                black = new_black;
                white_to_move = !white_to_move;
                // Opponent's move!
            }
        }
    }
}

fn main() {
    let args = Args::parse();
    if args.generate_book {
        if args.book_path.as_str() != "" {
            println!(
                "{} {} {} {}",
                args.search_depth, args.full_depth, args.k_partial_depth, args.book_path
            );
            generate_opening_book(
                args.search_depth,
                args.full_depth,
                args.k_partial_depth,
                args.book_path.as_str(),
            );
        } else {
            println!("No opening book save path provided!");
        }
    } else if args.api_url == "".to_string() {
        local_game(args);
    } else {
        play_multiplayer(args);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_to_algebraic() {
        assert_eq!(move_to_algebraic(1).unwrap(), "a1");
        assert_eq!(move_to_algebraic(4).unwrap(), "c1");
        assert_eq!(move_to_algebraic(3), None);
    }

    #[test]
    fn test_move_to_bitmap() {
        assert_eq!(move_to_bitmap("a1").unwrap(), 1);
        assert_eq!(move_to_bitmap("foo"), Err("Invalid move notation"));
    }

    #[test]
    fn test_eval_position() {
        assert_eq!(eval_position(4325574457067520514, 33909430323788925), 7);
    }

    /// Helper bitwise shift functions which do not take board edges into account
    /// unlike similar functions from the base program.

    fn shift_left(x: u64) -> u64 {
        x << 1
    }

    fn shift_right(x: u64) -> u64 {
        x >> 1
    }

    fn shift_up(x: u64) -> u64 {
        x << 8
    }

    fn shift_down(x: u64) -> u64 {
        x >> 8
    }

    #[test]
    fn test_flip_in_dir_basic_single_flip_left() {
        let move_bit = 0b0010_0000;
        let me       = 0b1000_0000;
        let opp      = 0b0100_0000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_left);
        assert_eq!(flipped, 0b0100_0000); // We expect exactly the opp bit to flip.
    }

    #[test]
    fn test_flip_in_dir_no_flip_left() {
        let move_bit = 0b0001_0000;
        let me       = 0b1000_0000;
        let opp      = 0b0010_0000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_left);
        assert_eq!(flipped, 0); // No valid chain, so nothing flips.
    }

    #[test]
    fn test_flip_in_dir_multiple_flips_left() {
        let move_bit = 0b0000_1000;
        let opp      = 0b0111_0000;
        let me       = 0b1000_0000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_left);
        assert_eq!(flipped, 0b0111_0000);
    }

    #[test]
    fn test_flip_in_dir_interrupted_chain_left() {
        let move_bit = 0b0000_1000;
        let opp      = 0b0010_0000;
        let me       = 0b1000_0000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_left);
        assert_eq!(flipped, 0);
    }

    #[test]
    fn test_flip_in_dir_basic_single_flip_right() {
        let move_bit = 0x800;
        let opp      = 0x400;
        let me       = 0x200;

        let flipped = flip_in_dir(move_bit, me, opp, shift_right);
        assert_eq!(flipped, 0x400);
    }

    #[test]
    fn test_flip_in_dir_basic_single_flip_up() {
        let move_bit = 0x0001;
        let opp      = 0x0100;
        let me       = 0x010000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_up);
        assert_eq!(flipped, 0x0100);
    }

    #[test]
    fn test_flip_in_dir_no_flip_up_due_to_gap() {
        let move_bit = 0x0001;
        let opp      = 0x0200;
        let me       = 0x010000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_up);
        assert_eq!(flipped, 0);
    }

    #[test]
    fn test_flip_in_dir_multiple_flips_down() {
        let move_bit = 0x1000000000000000;
        let opp      = 0x0010000000000000 | 0x0000100000000000;
        let me       = 0x0000001000000000;

        let flipped = flip_in_dir(move_bit, me, opp, shift_down);
        assert_eq!(flipped, opp);
    }

    #[test]
    fn test_apply_move_opt() {
        assert_eq!(apply_move_opt(35253361508352, 171935537184, move_to_bitmap("c4").unwrap(), true).unwrap(), (35253562834944, 171801319456));
        assert_eq!(apply_move_opt(35253361508352, 171935537184, move_to_bitmap("a1").unwrap(), true), Err("No flips!"));
        assert_eq!(apply_move_opt(35253361508352, 171935537184, move_to_bitmap("a3").unwrap(), true), Err("Square already occupied"));
    }

    #[test]
    fn test_compute_moves_no_possible_moves() {
        let me  = 0x1;
        let opp = 0x2;

        print_board(me, opp, 0, 0, false);
        let moves = compute_moves(opp, me);
        assert_eq!(moves, 0, "Expected no moves, got some bits set instead.");
    }

    #[test]
    fn test_compute_moves_simple_horizontal() {
        let me  = 1 << 3;
        let opp = (1 << 2) | (1 << 1);

        let moves = compute_moves(me, opp);
        assert_eq!(moves, 1, "Expected bit 0 to be a valid move, but got something else.");
    }

    #[test]
    fn test_compute_moves_standard_othello_black_to_move() {
        // Black pieces (me)
        let me = (1 << 28) | (1 << 35);
        // White pieces (opp)
        let opp = (1 << 27) | (1 << 36);

        let moves = compute_moves(me, opp);

        let expected_moves = (1 << 19) | (1 << 26) | (1 << 37) | (1 << 44);

        assert_eq!(moves, expected_moves,
            "Black's standard opening moves did not match the expected bitmask."
        );
    }

    #[test]
    fn test_compute_moves_standard_othello_white_to_move() {
        // White pieces (me)
        let me = (1 << 27) | (1 << 36);
        // Black pieces (opp)
        let opp = (1 << 28) | (1 << 35);

        let moves = compute_moves(me, opp);

        let expected_moves = (1 << 20) | (1 << 29) | (1 << 34) | (1 << 43);
        assert_eq!(moves, expected_moves,
            "White's standard opening moves did not match the expected bitmask."
        );
    }

    #[test]
    fn test_compute_moves_all_filled_but_one() {
        let empty_bit = 12;
        let all_board = u64::MAX;
        let me  = all_board & !(1 << empty_bit) & !(1 << 10) & !(1 << 11);
        let opp = (1 << 10) | (1 << 11);
        print_board(me, opp, 0, 0, false);
        let moves = compute_moves(opp, me);
        assert_eq!(moves, 0,
            "Expected no valid moves on a nearly full board, got a nonzero mask."
        );
    }

    #[test]
    fn test_check_game_status_alt_current_player_has_moves() {
        let white = (1 << 27) | (1 << 36); // (3,3) and (4,4)
        let black = (1 << 28) | (1 << 35); // (3,4) and (4,3)
        let is_white_move = true;

        let expected_moves = (1 << 20) | (1 << 29) | (1 << 34) | (1 << 43);

        let status = check_game_status_alt(white, black, is_white_move);
        assert_eq!(
            status,
            expected_moves,
            "Expected White's standard opening moves, got something else."
        );
    }

    #[test]
    fn test_check_game_status_alt_current_player_has_no_moves_opponent_does() {
        let white = 0x0000_FFFF_FFFF_F000u64;
        let black = 0x0000_0000_0000_FFFFu64;

        let is_white_move = true;

        let status = check_game_status_alt(white, black, is_white_move);
        assert_eq!(
            status,
            u64::MAX,
            "Expected pass situation (u64::MAX) if current player has no moves but opponent does."
        );
    }

    #[test]
    fn test_check_game_status_alt_both_sides_have_no_moves_white_wins() {
        let white = 14260085270048145407;
        let black = 67108864;
        let is_white_move = true;

        let status = check_game_status_alt(white, black, is_white_move);
        assert_eq!(
            status,
            u64::MAX - 2,
            "Expected White to win => (u64::MAX - 2). Got something else."
        );
    }

    #[test]
    fn test_check_game_status_alt_both_sides_have_no_moves_black_wins() {
        let white = 67108864;
        let black = 14260085270048145407;

        let is_white_move = false;
        print_board(white, black, 0, 0, false);

        let status = check_game_status_alt(white, black, is_white_move);
        assert_eq!(
            status,
            u64::MAX - 1,
            "Expected Black to win => (u64::MAX - 1). Got something else."
        );
    }

    #[test]
    fn test_check_game_status_alt_both_sides_have_no_moves_tie() {
        let white = 0x0000_0000_FFFF_FFFFu64; // exactly 32 bits set
        let black = 0xFFFF_FFFF_0000_0000u64; // exactly 32 bits set

        let is_white_move = true; // or false, same result if no moves remain.

        let status = check_game_status_alt(white, black, is_white_move);
        assert_eq!(
            status,
            u64::MAX - 3,
            "Expected tie => (u64::MAX - 3). Got something else."
        );
    }

    #[test]
    fn test_check_game_status_alt_black_has_moves() {
        let white = (1 << 27) | (1 << 36); // (3,3) and (4,4)
        let black = (1 << 28) | (1 << 35); // (3,4) and (4,3)
        let is_white_move = false;         // black to move

        let expected_moves = (1 << 19) | (1 << 26) | (1 << 37) | (1 << 44);

        let status = check_game_status_alt(white, black, is_white_move);
        assert_eq!(
            status,
            expected_moves,
            "Expected black's moves in the standard opening, got something else."
        );
    }
}
