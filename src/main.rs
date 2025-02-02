use chrono;
use clap::Parser;
use rayon::prelude::*;
use reversi_tools::position::*;
use std::collections::HashMap;
use std::path::Path;

mod openingbook;
use openingbook::*;

mod engine;
use engine::*;

mod utils;
use utils::*;

use reversi_engine::multiplayer::api_client::*;
use reversi_engine::multiplayer::model::*;

use reversi_engine::cli::args::*;

use std::{thread, time};

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
                        -20000,
                        20000,
                        calculation_depth,
                        DEFAULT_CFG,
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
                            apply_move(pos.white, pos.black, best_move, pos.white_to_move);
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
                let new_pos_opt = apply_move(pos.white, pos.black, next_move, pos.white_to_move);
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

fn play_game_from_position(first: EvalCfg, second: EvalCfg, depth: u32, pos: Position) -> i32 {
    let mut white = pos.white;
    let mut black = pos.black;
    let mut white_to_move = pos.white_to_move;
    const BLACK_WON: u64 = u64::MAX - 1;
    const WHITE_WON: u64 = u64::MAX - 2;
    const DRAWN_GAME: u64 = u64::MAX - 3;
    loop {
        match check_game_status(white, black, white_to_move) {
            u64::MAX => {
                white_to_move = !white_to_move;
            }
            BLACK_WON => {
                return 1;
            }
            WHITE_WON => {
                return -1;
            }
            DRAWN_GAME => {
                return 0;
            }
            _ => {
                let curr_cfg;
                if white_to_move {
                    curr_cfg = second;
                } else {
                    curr_cfg = first;
                }
                let (best_move, _) = search_moves_opt(
                    white,
                    black,
                    white_to_move,
                    depth,
                    -20000,
                    20000,
                    depth,
                    curr_cfg,
                );
                match apply_move(white, black, best_move, white_to_move) {
                    Ok((w, b)) => {
                        white = w;
                        black = b;
                        white_to_move = !white_to_move;
                    }
                    Err(_) => {
                        return 0;
                    }
                }
            }
        }
    }
}

fn compare_configs(first: EvalCfg, second: EvalCfg, depth: u32) -> i32 {
    // Generate all positions with a depth of 6 plies
    let black = 0x0000000810000000u64;
    let white = 0x0000001008000000u64;
    let white_to_move: bool = false;
    let starting_pos: Position = Position {
        black: black,
        white: white,
        white_to_move: white_to_move,
    };
    let mut queue: Vec<Position> = Vec::new();
    let mut dedup_cache: HashMap<Position, bool> = HashMap::new();
    queue.push(starting_pos);
    for _ in 0..6 {
        let mut next_queue: Vec<Position> = Vec::new();
        for pos in queue {
            if dedup_cache.contains_key(&pos) {
                continue;
            }
            let next_moves = find_legal_moves_alt(pos.white, pos.black, pos.white_to_move);
            for next_move in next_moves {
                let new_pos_opt = apply_move(pos.white, pos.black, next_move, pos.white_to_move);
                match new_pos_opt {
                    Ok((w, b)) => {
                        let new_pos: Position = Position {
                            black: b,
                            white: w,
                            white_to_move: !pos.white_to_move,
                        };
                        let mut p = pos.clone();
                        for _ in 0..4 {
                            dedup_cache.insert(p, true);
                            dedup_cache.insert(flip_position_vertical(&p), true);
                            dedup_cache.insert(flip_position_horizontal(&p), true);
                            p = rotate_position_90(&p);
                        }
                        next_queue.push(new_pos);
                    }
                    Err(_) => {
                        //println!("Move error: {}", s);
                        continue;
                    }
                }
            }
        }
        queue = next_queue;
    }
    println!("Comparing engines over {} positions", queue.len());
    let outcome = queue
        .into_par_iter()
        .map(|pos| {
            let mut res: i32 = 2 * play_game_from_position(first, second, depth, pos);
            res -= 2 * play_game_from_position(second, first, depth, pos);
            res
        })
        .reduce(|| 0, |curr, x| curr + x);
    outcome
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
    //let default_depth: u32 = args.search_depth;
    let mut ply = 0;
    loop {
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
                    (nxt_move, eval) = search_moves_par(
                        white,
                        black,
                        white_to_move,
                        args.search_depth,
                        -20000,
                        20000,
                        args.search_depth,
                        DEFAULT_CFG,
                    );
                    if nxt_move == 0 {
                        println!("NO MOVES!");
                        break;
                    }
                }
            }
        } else {
            (nxt_move, eval) = search_moves_par(
                white,
                black,
                white_to_move,
                args.search_depth,
                -20000,
                20000,
                args.search_depth,
                DEFAULT_CFG,
            );
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
            let (new_white, new_black) =
                apply_move_verbose(white, black, nxt_move, white_to_move).unwrap();
            //println!("WWW {} {} {}", new_white, new_black, white_to_move);
            let game_status = check_game_status(new_white, new_black, !white_to_move);
            if game_status == u64::MAX || game_status < (u64::MAX - 3) {
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
                apply_move_verbose(white, black, opp_first_move, white_to_move).unwrap();
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
                            -20000,
                            20000,
                            depth,
                            DEFAULT_CFG,
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
                        apply_move_verbose(white, black, nxt_move, white_to_move).unwrap();
                    nxt_move_algebraic = move_to_algebraic(nxt_move).unwrap();
                    println!(
                        "Move {} {}, eval {}, black pos: {}, white pos: {}, white move: {}",
                        nxt_move_algebraic, nxt_move, eval, black, white, white_to_move
                    );
                    white = new_white;
                    black = new_black;
                    let game_status = check_game_status(new_white, new_black, !white_to_move);
                    if (game_status == (u64::MAX - 1) && my_color == "white".to_string())
                        || (game_status == (u64::MAX - 2) && my_color == "black".to_string())
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
                    apply_move_verbose(white, black, opp_move, white_to_move).unwrap();
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
    } else if args.compare_configs {
        let first: EvalCfg = EvalCfg {
            corner_value: 70,
            edge_value: 17,
            antiedge_value: -22,
            anticorner_value: -34,
        };
        let second: EvalCfg = EvalCfg {
            corner_value: 70,
            edge_value: 17,
            antiedge_value: -20,
            anticorner_value: -30,
        };
        println!(
            "The score between first and second configs is {}",
            compare_configs(first, second, args.search_depth)
        );
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
    fn test_eval_position() {
        assert_eq!(eval_position(4325574457067520514, 33909430323788925), 7);
    }
}
