use chrono;
use clap::Parser;
use rayon::prelude::*;
use reversi_tools::position::*;
use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

mod openingbook;
use openingbook::*;

mod tt;

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

fn evaluate_position(depth: u32, pos: Position) -> u64 {
    // Clear the TT so each position is measured from a cold state; this
    // makes the benchmark a faithful per-position comparison.
    tt::tt().clear();
    let mut counter: u64 = 0;
    search_iterative_cntr(
        pos.white,
        pos.black,
        pos.white_to_move,
        depth,
        DEFAULT_CFG,
        &mut counter,
    );
    return counter;
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

fn benchmark(depth: u32) -> i32 {
    let queue: Vec<Position> = vec![
        Position {
            white: 68719476736,
            black: 34762915840,
            white_to_move: true,
        },
        Position {
            white: 68719476736,
            black: 34829500416,
            white_to_move: true,
        },
        Position {
            white: 134217728,
            black: 240786604032,
            white_to_move: true,
        },
        Position {
            white: 134217728,
            black: 17695533694976,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 34628698112,
            white_to_move: false,
        },
        Position {
            white: 68988960768,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 120259084288,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 68853694464,
            black: 34629091328,
            white_to_move: true,
        },
        Position {
            white: 68719738880,
            black: 34830024704,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 240787128320,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 17695534219264,
            white_to_move: true,
        },
        Position {
            white: 68987912192,
            black: 34495537152,
            white_to_move: true,
        },
        Position {
            white: 68719476736,
            black: 34766061568,
            white_to_move: true,
        },
        Position {
            white: 68720525312,
            black: 35299786752,
            white_to_move: true,
        },
        Position {
            white: 1048576,
            black: 240921346048,
            white_to_move: true,
        },
        Position {
            white: 269484032,
            black: 35287586045952,
            white_to_move: true,
        },
        Position {
            white: 103079215104,
            black: 2216606302208,
            white_to_move: true,
        },
        Position {
            white: 85899345920,
            black: 4432809426944,
            white_to_move: true,
        },
        Position {
            white: 85899345920,
            black: 8830855938048,
            white_to_move: true,
        },
        Position {
            white: 51539607552,
            black: 17661308698624,
            white_to_move: true,
        },
        Position {
            white: 51539607552,
            black: 35253494743040,
            white_to_move: true,
        },
        Position {
            white: 68853957120,
            black: 34628829184,
            white_to_move: false,
        },
        Position {
            white: 68854220800,
            black: 34628567040,
            white_to_move: false,
        },
        Position {
            white: 69123178496,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 69659000832,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 120393302016,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 8899306455040,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 68989747200,
            black: 34561064960,
            white_to_move: false,
        },
        Position {
            white: 120326455296,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 240518692864,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 206427389952,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 172067651584,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 17695265783808,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 17626814742528,
            white_to_move: false,
        },
        Position {
            white: 68988437504,
            black: 34495012864,
            white_to_move: false,
        },
        Position {
            white: 68988964864,
            black: 34494488576,
            white_to_move: false,
        },
        Position {
            white: 69122392064,
            black: 34361319424,
            white_to_move: false,
        },
        Position {
            white: 69189238784,
            black: 34361319424,
            white_to_move: false,
        },
        Position {
            white: 120527519744,
            black: 135798784,
            white_to_move: false,
        },
        Position {
            white: 4501394161664,
            black: 135798784,
            white_to_move: false,
        },
        Position {
            white: 68988964864,
            black: 34496577536,
            white_to_move: false,
        },
        Position {
            white: 68853956608,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 120259084288,
            black: 406323200,
            white_to_move: false,
        },
        Position {
            white: 68855529472,
            black: 35165044736,
            white_to_move: false,
        },
        Position {
            white: 69261590528,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 120394350592,
            black: 805830656,
            white_to_move: false,
        },
        Position {
            white: 344135303168,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 1835008,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 17315135488,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 17661175005184,
            black: 171933433856,
            white_to_move: false,
        },
        Position {
            white: 270009344,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 270270464,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 470810624,
            black: 35287451828224,
            white_to_move: false,
        },
        Position {
            white: 17583570944,
            black: 35287451828224,
            white_to_move: false,
        },
        Position {
            white: 4432675733504,
            black: 35253226307584,
            white_to_move: false,
        },
        Position {
            white: 17661175005184,
            black: 35218866569216,
            white_to_move: false,
        },
        Position {
            white: 103213959168,
            black: 2216471560192,
            white_to_move: false,
        },
        Position {
            white: 103213694976,
            black: 2216472084480,
            white_to_move: false,
        },
        Position {
            white: 103348699136,
            black: 2216337866752,
            white_to_move: false,
        },
        Position {
            white: 103349747712,
            black: 2216337866752,
            white_to_move: false,
        },
        Position {
            white: 128849018880,
            black: 2199426433024,
            white_to_move: false,
        },
        Position {
            white: 86033825792,
            black: 4432675209216,
            white_to_move: false,
        },
        Position {
            white: 86303047680,
            black: 4432406773760,
            white_to_move: false,
        },
        Position {
            white: 1130383852699648,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 86033825792,
            black: 8830721720320,
            white_to_move: false,
        },
        Position {
            white: 86303047680,
            black: 8830453284864,
            white_to_move: false,
        },
        Position {
            white: 1134781899210752,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 4512481619738624,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 17661173956608,
            white_to_move: false,
        },
        Position {
            white: 51674873856,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 51810140160,
            black: 17661040263168,
            white_to_move: false,
        },
        Position {
            white: 257698037760,
            black: 17592589221888,
            white_to_move: false,
        },
        Position {
            white: 9024842980392960,
            black: 69122654208,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 35253360001024,
            white_to_move: false,
        },
        Position {
            white: 51674873856,
            black: 35253360525312,
            white_to_move: false,
        },
        Position {
            white: 51810140160,
            black: 35253226307584,
            white_to_move: false,
        },
        Position {
            white: 257698037760,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 68853956608,
            black: 34628829698,
            white_to_move: true,
        },
        Position {
            white: 68719739392,
            black: 34830155776,
            white_to_move: true,
        },
        Position {
            white: 134480384,
            black: 240787259392,
            white_to_move: true,
        },
        Position {
            white: 134480384,
            black: 17695534350336,
            white_to_move: true,
        },
        Position {
            white: 68719476736,
            black: 34763311112,
            white_to_move: true,
        },
        Position {
            white: 68854218752,
            black: 34628569104,
            white_to_move: true,
        },
        Position {
            white: 68853696512,
            black: 34629092352,
            white_to_move: true,
        },
        Position {
            white: 68853696512,
            black: 34630139904,
            white_to_move: true,
        },
        Position {
            white: 68720003072,
            black: 34829893632,
            white_to_move: true,
        },
        Position {
            white: 134744064,
            black: 240786997248,
            white_to_move: true,
        },
        Position {
            white: 134744064,
            black: 17695534088192,
            white_to_move: true,
        },
        Position {
            white: 526336,
            black: 35287854350336,
            white_to_move: true,
        },
        Position {
            white: 68853694464,
            black: 34632237056,
            white_to_move: true,
        },
        Position {
            white: 135266304,
            black: 240787521536,
            white_to_move: true,
        },
        Position {
            white: 269484032,
            black: 35287586439168,
            white_to_move: true,
        },
        Position {
            white: 69390565376,
            black: 34631188480,
            white_to_move: true,
        },
        Position {
            white: 671088640,
            black: 240787521536,
            white_to_move: true,
        },
        Position {
            white: 805306368,
            black: 35287586439168,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 470679552,
            white_to_move: true,
        },
        Position {
            white: 86033563648,
            black: 4432675602432,
            white_to_move: true,
        },
        Position {
            white: 85899345920,
            black: 8830856331264,
            white_to_move: true,
        },
        Position {
            white: 51673825280,
            black: 17661174874112,
            white_to_move: true,
        },
        Position {
            white: 51539607552,
            black: 35253495136256,
            white_to_move: true,
        },
        Position {
            white: 8899172237312,
            black: 470679552,
            white_to_move: true,
        },
        Position {
            white: 8864946716672,
            black: 4432675602432,
            white_to_move: true,
        },
        Position {
            white: 8830586978304,
            black: 17661174874112,
            white_to_move: true,
        },
        Position {
            white: 8830452760576,
            black: 35253495136256,
            white_to_move: true,
        },
        Position {
            white: 68719476736,
            black: 2260630670016512,
            white_to_move: true,
        },
        Position {
            white: 68989485056,
            black: 34561327616,
            white_to_move: true,
        },
        Position {
            white: 68989485056,
            black: 34561328128,
            white_to_move: true,
        },
        Position {
            white: 68989222912,
            black: 34561591296,
            white_to_move: true,
        },
        Position {
            white: 68989222912,
            black: 34561593344,
            white_to_move: true,
        },
        Position {
            white: 68988698624,
            black: 34562121728,
            white_to_move: true,
        },
        Position {
            white: 68721311744,
            black: 34831597568,
            white_to_move: true,
        },
        Position {
            white: 68721311744,
            black: 35366371328,
            white_to_move: true,
        },
        Position {
            white: 270270464,
            black: 240719495168,
            white_to_move: true,
        },
        Position {
            white: 270270464,
            black: 35287652630528,
            white_to_move: true,
        },
        Position {
            white: 262144,
            black: 240921348104,
            white_to_move: true,
        },
        Position {
            white: 526336,
            black: 240921084416,
            white_to_move: true,
        },
        Position {
            white: 134481920,
            black: 240787129344,
            white_to_move: true,
        },
        Position {
            white: 788480,
            black: 240987930624,
            white_to_move: true,
        },
        Position {
            white: 1572864,
            black: 240921084416,
            white_to_move: true,
        },
        Position {
            white: 135528448,
            black: 240787129344,
            white_to_move: true,
        },
        Position {
            white: 1310720,
            black: 240921348096,
            white_to_move: true,
        },
        Position {
            white: 135004160,
            black: 240787656704,
            white_to_move: true,
        },
        Position {
            white: 1835008,
            black: 240987930624,
            white_to_move: true,
        },
        Position {
            white: 805306368,
            black: 240653173248,
            white_to_move: true,
        },
        Position {
            white: 939524096,
            black: 240519086080,
            white_to_move: true,
        },
        Position {
            white: 671350784,
            black: 240788176896,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 241326096384,
            white_to_move: true,
        },
        Position {
            white: 402915328,
            black: 241059758080,
            white_to_move: true,
        },
        Position {
            white: 8830452760576,
            black: 206561870336,
            white_to_move: true,
        },
        Position {
            white: 8830586978304,
            black: 206427783168,
            white_to_move: true,
        },
        Position {
            white: 8830453022720,
            black: 206628716544,
            white_to_move: true,
        },
        Position {
            white: 8796227502080,
            black: 257966997504,
            white_to_move: true,
        },
        Position {
            white: 8796227502080,
            black: 4638833639424,
            white_to_move: true,
        },
        Position {
            white: 34494218240,
            black: 1134902427254784,
            white_to_move: true,
        },
        Position {
            white: 262144,
            black: 2260836828053504,
            white_to_move: true,
        },
        Position {
            white: 35253225783296,
            black: 172068044800,
            white_to_move: true,
        },
        Position {
            white: 35253091827712,
            black: 172268978176,
            white_to_move: true,
        },
        Position {
            white: 35184506568704,
            black: 17832973172736,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 9042555694481408,
            white_to_move: true,
        },
        Position {
            white: 262144,
            black: 17695668439048,
            white_to_move: true,
        },
        Position {
            white: 526336,
            black: 17695668175360,
            white_to_move: true,
        },
        Position {
            white: 134481920,
            black: 17695534220288,
            white_to_move: true,
        },
        Position {
            white: 788480,
            black: 17695735021568,
            white_to_move: true,
        },
        Position {
            white: 1572864,
            black: 17695668175360,
            white_to_move: true,
        },
        Position {
            white: 135528448,
            black: 17695534220288,
            white_to_move: true,
        },
        Position {
            white: 1310720,
            black: 17695668439040,
            white_to_move: true,
        },
        Position {
            white: 135004160,
            black: 17695534747648,
            white_to_move: true,
        },
        Position {
            white: 1835008,
            black: 17695735021568,
            white_to_move: true,
        },
        Position {
            white: 805306368,
            black: 17695400264192,
            white_to_move: true,
        },
        Position {
            white: 939524096,
            black: 17695266177024,
            white_to_move: true,
        },
        Position {
            white: 671350784,
            black: 17695535267840,
            white_to_move: true,
        },
        Position {
            white: 671350784,
            black: 17695536316416,
            white_to_move: true,
        },
        Position {
            white: 402915328,
            black: 17695806849024,
            white_to_move: true,
        },
        Position {
            white: 671350784,
            black: 17832973172736,
            white_to_move: true,
        },
        Position {
            white: 8830452760576,
            black: 17661308961280,
            white_to_move: true,
        },
        Position {
            white: 8830586978304,
            black: 17661174874112,
            white_to_move: true,
        },
        Position {
            white: 8796093284352,
            black: 17695735545856,
            white_to_move: true,
        },
        Position {
            white: 8796227502080,
            black: 17712714088448,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 30889673752576,
            white_to_move: true,
        },
        Position {
            white: 34494218240,
            black: 1152357174345728,
            white_to_move: true,
        },
        Position {
            white: 262144,
            black: 2278291575144448,
            white_to_move: true,
        },
        Position {
            white: 35253225783296,
            black: 17626815135744,
            white_to_move: true,
        },
        Position {
            white: 35253091827712,
            black: 17627016069120,
            white_to_move: true,
        },
        Position {
            white: 35184506568704,
            black: 17832973172736,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 123179931009024,
            white_to_move: true,
        },
        Position {
            white: 68987913216,
            black: 34495539200,
            white_to_move: true,
        },
        Position {
            white: 68987913216,
            black: 34495799296,
            white_to_move: true,
        },
        Position {
            white: 68720002048,
            black: 34765545472,
            white_to_move: true,
        },
        Position {
            white: 68720002048,
            black: 35300319232,
            white_to_move: true,
        },
        Position {
            white: 268960768,
            black: 240653443072,
            white_to_move: true,
        },
        Position {
            white: 525312,
            black: 17695668969472,
            white_to_move: true,
        },
        Position {
            white: 268960768,
            black: 35287586578432,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 34494492704,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 34494494720,
            white_to_move: true,
        },
        Position {
            white: 68719480832,
            black: 34766069760,
            white_to_move: true,
        },
        Position {
            white: 68720529408,
            black: 35299794944,
            white_to_move: true,
        },
        Position {
            white: 1052672,
            black: 240921354240,
            white_to_move: true,
        },
        Position {
            white: 269488128,
            black: 35287586054144,
            white_to_move: true,
        },
        Position {
            white: 69122129920,
            black: 34361712640,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 34631852032,
            white_to_move: true,
        },
        Position {
            white: 68988174336,
            black: 51675406336,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 240788185088,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 17695535276032,
            white_to_move: true,
        },
        Position {
            white: 69122129920,
            black: 34428559360,
            white_to_move: true,
        },
        Position {
            white: 68920803328,
            black: 34631852032,
            white_to_move: true,
        },
        Position {
            white: 69122129920,
            black: 43018362880,
            white_to_move: true,
        },
        Position {
            white: 69055021056,
            black: 51675406336,
            white_to_move: true,
        },
        Position {
            white: 201326592,
            black: 240788185088,
            white_to_move: true,
        },
        Position {
            white: 201326592,
            black: 17695535276032,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 941105152,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 137843187712,
            white_to_move: true,
        },
        Position {
            white: 103347650560,
            black: 2216338923520,
            white_to_move: true,
        },
        Position {
            white: 86167781376,
            black: 8830588559360,
            white_to_move: true,
        },
        Position {
            white: 51539607552,
            black: 17661309755392,
            white_to_move: true,
        },
        Position {
            white: 51808043008,
            black: 35253227364352,
            white_to_move: true,
        },
        Position {
            white: 4501125726208,
            black: 941105152,
            white_to_move: true,
        },
        Position {
            white: 4501125726208,
            black: 137843187712,
            white_to_move: true,
        },
        Position {
            white: 4467034423296,
            black: 8830588559360,
            white_to_move: true,
        },
        Position {
            white: 4432406249472,
            black: 17661309755392,
            white_to_move: true,
        },
        Position {
            white: 4432674684928,
            black: 35253227364352,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 34496581640,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 34496581664,
            white_to_move: true,
        },
        Position {
            white: 68987916288,
            black: 34497634304,
            white_to_move: true,
        },
        Position {
            white: 68720529408,
            black: 35301883904,
            white_to_move: true,
        },
        Position {
            white: 1052672,
            black: 240923443200,
            white_to_move: true,
        },
        Position {
            white: 269488128,
            black: 35287588143104,
            white_to_move: true,
        },
        Position {
            white: 68853694464,
            black: 34632237056,
            white_to_move: true,
        },
        Position {
            white: 68719738880,
            black: 34833170432,
            white_to_move: true,
        },
        Position {
            white: 68719738880,
            black: 51945930752,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 240790274048,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 17695537364992,
            white_to_move: true,
        },
        Position {
            white: 103079215104,
            black: 2216609447936,
            white_to_move: true,
        },
        Position {
            white: 85899345920,
            black: 4432812572672,
            white_to_move: true,
        },
        Position {
            white: 85899345920,
            black: 8830859083776,
            white_to_move: true,
        },
        Position {
            white: 51539607552,
            black: 17661311844352,
            white_to_move: true,
        },
        Position {
            white: 51539607552,
            black: 35253497888768,
            white_to_move: true,
        },
        Position {
            white: 68855005184,
            black: 35165570048,
            white_to_move: true,
        },
        Position {
            white: 68719738880,
            black: 35300837376,
            white_to_move: true,
        },
        Position {
            white: 68854480896,
            black: 35166097408,
            white_to_move: true,
        },
        Position {
            white: 68721311744,
            black: 35366371328,
            white_to_move: true,
        },
        Position {
            white: 136052736,
            black: 241323474944,
            white_to_move: true,
        },
        Position {
            white: 136052736,
            black: 8899977543680,
            white_to_move: true,
        },
        Position {
            white: 136052736,
            black: 17696070565888,
            white_to_move: true,
        },
        Position {
            white: 69260541952,
            black: 34763968512,
            white_to_move: true,
        },
        Position {
            white: 69260541952,
            black: 34763972608,
            white_to_move: true,
        },
        Position {
            white: 69260541952,
            black: 34766061568,
            white_to_move: true,
        },
        Position {
            white: 68724719616,
            black: 36373528576,
            white_to_move: true,
        },
        Position {
            white: 542113792,
            black: 240921346048,
            white_to_move: true,
        },
        Position {
            white: 542113792,
            black: 17695668436992,
            white_to_move: true,
        },
        Position {
            white: 542113792,
            black: 35287854481408,
            white_to_move: true,
        },
        Position {
            white: 120393302016,
            black: 806881280,
            white_to_move: true,
        },
        Position {
            white: 120393302016,
            black: 806883328,
            white_to_move: true,
        },
        Position {
            white: 120393302016,
            black: 808976384,
            white_to_move: true,
        },
        Position {
            white: 120260132864,
            black: 1007157248,
            white_to_move: true,
        },
        Position {
            white: 86034612224,
            black: 4433212080128,
            white_to_move: true,
        },
        Position {
            white: 17180917760,
            black: 8900112285696,
            white_to_move: true,
        },
        Position {
            white: 51674873856,
            black: 17661711351808,
            white_to_move: true,
        },
        Position {
            white: 344134254592,
            black: 34763968512,
            white_to_move: true,
        },
        Position {
            white: 344134254592,
            black: 34763972608,
            white_to_move: true,
        },
        Position {
            white: 344134254592,
            black: 34766061568,
            white_to_move: true,
        },
        Position {
            white: 343598432256,
            black: 36373528576,
            white_to_move: true,
        },
        Position {
            white: 275415826432,
            black: 240921346048,
            white_to_move: true,
        },
        Position {
            white: 275415826432,
            black: 17695668436992,
            white_to_move: true,
        },
        Position {
            white: 275415826432,
            black: 35287854481408,
            white_to_move: true,
        },
        Position {
            white: 1572864,
            black: 240921084416,
            white_to_move: true,
        },
        Position {
            white: 1310720,
            black: 240921347072,
            white_to_move: true,
        },
        Position {
            white: 1310720,
            black: 240921348096,
            white_to_move: true,
        },
        Position {
            white: 786432,
            black: 240921874432,
            white_to_move: true,
        },
        Position {
            white: 786432,
            black: 240921878528,
            white_to_move: true,
        },
        Position {
            white: 17314086912,
            black: 240788180992,
            white_to_move: true,
        },
        Position {
            white: 17180917760,
            black: 240921608192,
            white_to_move: true,
        },
        Position {
            white: 17314086912,
            black: 240790274048,
            white_to_move: true,
        },
        Position {
            white: 17180917760,
            black: 240988454912,
            white_to_move: true,
        },
        Position {
            white: 135266304,
            black: 266556932096,
            white_to_move: true,
        },
        Position {
            white: 17661173956608,
            black: 171934490624,
            white_to_move: true,
        },
        Position {
            white: 17660905521152,
            black: 172205015040,
            white_to_move: true,
        },
        Position {
            white: 17660906569728,
            black: 172738740224,
            white_to_move: true,
        },
        Position {
            white: 17592455528448,
            black: 35425024999424,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 2269563933163520,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 9024963374219264,
            white_to_move: true,
        },
        Position {
            white: 269485056,
            black: 35287586048000,
            white_to_move: true,
        },
        Position {
            white: 525312,
            black: 35287855009792,
            white_to_move: true,
        },
        Position {
            white: 268960768,
            black: 35287586578432,
            white_to_move: true,
        },
        Position {
            white: 1573888,
            black: 35287856054272,
            white_to_move: true,
        },
        Position {
            white: 1573888,
            black: 35288390828032,
            white_to_move: true,
        },
        Position {
            white: 270008320,
            black: 35287585784320,
            white_to_move: true,
        },
        Position {
            white: 269746176,
            black: 35287586048000,
            white_to_move: true,
        },
        Position {
            white: 786432,
            black: 35287855009792,
            white_to_move: true,
        },
        Position {
            white: 269221888,
            black: 35287586578432,
            white_to_move: true,
        },
        Position {
            white: 1835008,
            black: 35287856054272,
            white_to_move: true,
        },
        Position {
            white: 1835008,
            black: 35288390828032,
            white_to_move: true,
        },
        Position {
            white: 201326592,
            black: 35287721316352,
            white_to_move: true,
        },
        Position {
            white: 403701760,
            black: 35287519068160,
            white_to_move: true,
        },
        Position {
            white: 336592896,
            black: 35287586308096,
            white_to_move: true,
        },
        Position {
            white: 201326592,
            black: 35287723409408,
            white_to_move: true,
        },
        Position {
            white: 403701760,
            black: 35296108871680,
            white_to_move: true,
        },
        Position {
            white: 202375168,
            black: 35425159217152,
            white_to_move: true,
        },
        Position {
            white: 17314086912,
            black: 35287721316352,
            white_to_move: true,
        },
        Position {
            white: 17449353216,
            black: 35287586308096,
            white_to_move: true,
        },
        Position {
            white: 17314086912,
            black: 35287723409408,
            white_to_move: true,
        },
        Position {
            white: 403701760,
            black: 35313221632000,
            white_to_move: true,
        },
        Position {
            white: 17315135488,
            black: 35425159217152,
            white_to_move: true,
        },
        Position {
            white: 4432406249472,
            black: 35253495795712,
            white_to_move: true,
        },
        Position {
            white: 4432674684928,
            black: 35253227364352,
            white_to_move: true,
        },
        Position {
            white: 4432674684928,
            black: 35253229453312,
            white_to_move: true,
        },
        Position {
            white: 4432407298048,
            black: 35254031613952,
            white_to_move: true,
        },
        Position {
            white: 4398315995136,
            black: 35304765915136,
            white_to_move: true,
        },
        Position {
            white: 4432407298048,
            black: 35390933696512,
            white_to_move: true,
        },
        Position {
            white: 4398315995136,
            black: 44083679068160,
            white_to_move: true,
        },
        Position {
            white: 17661173956608,
            black: 35218867625984,
            white_to_move: true,
        },
        Position {
            white: 17660905521152,
            black: 35219138150400,
            white_to_move: true,
        },
        Position {
            white: 17660906569728,
            black: 35219671875584,
            white_to_move: true,
        },
        Position {
            white: 17592187092992,
            black: 35425293434880,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 61607145635840,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 9060010307354624,
            white_to_move: true,
        },
        Position {
            white: 103213434880,
            black: 2216472085504,
            white_to_move: true,
        },
        Position {
            white: 103079741440,
            black: 2216606826496,
            white_to_move: true,
        },
        Position {
            white: 103079741440,
            black: 2216672886784,
            white_to_move: true,
        },
        Position {
            white: 134744064,
            black: 2456989728768,
            white_to_move: true,
        },
        Position {
            white: 68854220800,
            black: 6648877809664,
            white_to_move: true,
        },
        Position {
            white: 34494482432,
            black: 19877377081344,
            white_to_move: true,
        },
        Position {
            white: 103213432832,
            black: 2216472477696,
            white_to_move: true,
        },
        Position {
            white: 103079477248,
            black: 2216607350784,
            white_to_move: true,
        },
        Position {
            white: 103079477248,
            black: 2216673411072,
            white_to_move: true,
        },
        Position {
            white: 134479872,
            black: 2456990253056,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 6648878333952,
            white_to_move: true,
        },
        Position {
            white: 68719738880,
            black: 11047059062784,
            white_to_move: true,
        },
        Position {
            white: 34494218240,
            black: 19877377605632,
            white_to_move: true,
        },
        Position {
            white: 103347650560,
            black: 2216338923520,
            white_to_move: true,
        },
        Position {
            white: 103347650560,
            black: 2216341012480,
            white_to_move: true,
        },
        Position {
            white: 103080263680,
            black: 2217143173120,
            white_to_move: true,
        },
        Position {
            white: 1048576,
            black: 2457124470784,
            white_to_move: true,
        },
        Position {
            white: 68988960768,
            black: 11046790627328,
            white_to_move: true,
        },
        Position {
            white: 34629222400,
            black: 37469429432320,
            white_to_move: true,
        },
        Position {
            white: 103081312256,
            black: 2217143173120,
            white_to_move: true,
        },
        Position {
            white: 2097152,
            black: 2457124470784,
            white_to_move: true,
        },
        Position {
            white: 68990009344,
            black: 11046790627328,
            white_to_move: true,
        },
        Position {
            white: 34630270976,
            black: 37469429432320,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 2208049922048,
            white_to_move: true,
        },
        Position {
            white: 94489280512,
            black: 6631832682496,
            white_to_move: true,
        },
        Position {
            white: 94489280512,
            black: 11029879193600,
            white_to_move: true,
        },
        Position {
            white: 60129542144,
            black: 19860331954176,
            white_to_move: true,
        },
        Position {
            white: 60129542144,
            black: 37452517998592,
            white_to_move: true,
        },
        Position {
            white: 86033563648,
            black: 4432675602432,
            white_to_move: true,
        },
        Position {
            white: 68719738880,
            black: 4450056404992,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 4458445012992,
            white_to_move: true,
        },
        Position {
            white: 17314349056,
            black: 4638833639424,
            white_to_move: true,
        },
        Position {
            white: 17314349056,
            black: 22093580730368,
            white_to_move: true,
        },
        Position {
            white: 86033563648,
            black: 4432678354944,
            white_to_move: true,
        },
        Position {
            white: 69123178496,
            black: 4449653751808,
            white_to_move: true,
        },
        Position {
            white: 69123178496,
            black: 4458176577536,
            white_to_move: true,
        },
        Position {
            white: 17315135488,
            black: 4638833639424,
            white_to_move: true,
        },
        Position {
            white: 1130366672830464,
            black: 60532719616,
            white_to_move: true,
        },
        Position {
            white: 1130315133222912,
            black: 240921346048,
            white_to_move: true,
        },
        Position {
            white: 1130366672830464,
            black: 2250966040576,
            white_to_move: true,
        },
        Position {
            white: 1130315133222912,
            black: 17695668436992,
            white_to_move: true,
        },
        Position {
            white: 1130315133222912,
            black: 35287854481408,
            white_to_move: true,
        },
        Position {
            white: 1125985806188544,
            black: 567382762848256,
            white_to_move: true,
        },
        Position {
            white: 86033563648,
            black: 8830722113536,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 8847935143936,
            white_to_move: true,
        },
        Position {
            white: 85899608064,
            black: 8830923046912,
            white_to_move: true,
        },
        Position {
            white: 17314349056,
            black: 8899978067968,
            white_to_move: true,
        },
        Position {
            white: 68853956608,
            black: 8856491524096,
            white_to_move: true,
        },
        Position {
            white: 17314349056,
            black: 9036880150528,
            white_to_move: true,
        },
        Position {
            white: 17314349056,
            black: 26491627241472,
            white_to_move: true,
        },
        Position {
            white: 86033563648,
            black: 8830724866048,
            white_to_move: true,
        },
        Position {
            white: 69123178496,
            black: 8847666708480,
            white_to_move: true,
        },
        Position {
            white: 17583570944,
            black: 8899709632512,
            white_to_move: true,
        },
        Position {
            white: 69123178496,
            black: 8856223088640,
            white_to_move: true,
        },
        Position {
            white: 17315135488,
            black: 9036880150528,
            white_to_move: true,
        },
        Position {
            white: 1134764719341568,
            black: 60532719616,
            white_to_move: true,
        },
        Position {
            white: 1134713179734016,
            black: 240921346048,
            white_to_move: true,
        },
        Position {
            white: 1134764719341568,
            black: 2250966040576,
            white_to_move: true,
        },
        Position {
            white: 1134713179734016,
            black: 17695668436992,
            white_to_move: true,
        },
        Position {
            white: 1134713179734016,
            black: 35287854481408,
            white_to_move: true,
        },
        Position {
            white: 1125985806188544,
            black: 2260630669623296,
            white_to_move: true,
        },
        Position {
            white: 4512464439869440,
            black: 60532719616,
            white_to_move: true,
        },
        Position {
            white: 4512412900261888,
            black: 240921346048,
            white_to_move: true,
        },
        Position {
            white: 4512464439869440,
            black: 2250966040576,
            white_to_move: true,
        },
        Position {
            white: 4512412900261888,
            black: 17695668436992,
            white_to_move: true,
        },
        Position {
            white: 4512412900261888,
            black: 35287854481408,
            white_to_move: true,
        },
        Position {
            white: 4503685526716416,
            black: 2260630669623296,
            white_to_move: true,
        },
        Position {
            white: 51673827328,
            black: 17661174481920,
            white_to_move: true,
        },
        Position {
            white: 51540133888,
            black: 17661308436480,
            white_to_move: true,
        },
        Position {
            white: 17180395520,
            black: 17695735021568,
            white_to_move: true,
        },
        Position {
            white: 134744064,
            black: 17721303498752,
            white_to_move: true,
        },
        Position {
            white: 17314613248,
            black: 22093580206080,
            white_to_move: true,
        },
        Position {
            white: 51673825280,
            black: 17661175533568,
            white_to_move: true,
        },
        Position {
            white: 51540656128,
            black: 17661308960768,
            white_to_move: true,
        },
        Position {
            white: 51673825280,
            black: 17661177626624,
            white_to_move: true,
        },
        Position {
            white: 17180917760,
            black: 17695735545856,
            white_to_move: true,
        },
        Position {
            white: 135266304,
            black: 17721304023040,
            white_to_move: true,
        },
        Position {
            white: 17315135488,
            black: 22093580730368,
            white_to_move: true,
        },
        Position {
            white: 17180917760,
            black: 26491761459200,
            white_to_move: true,
        },
        Position {
            white: 51541704704,
            black: 17661309747200,
            white_to_move: true,
        },
        Position {
            white: 17450401792,
            black: 17695467110400,
            white_to_move: true,
        },
        Position {
            white: 51541704704,
            black: 17661845569536,
            white_to_move: true,
        },
        Position {
            white: 270532608,
            black: 17721169805312,
            white_to_move: true,
        },
        Position {
            white: 51541704704,
            black: 17798747652096,
            white_to_move: true,
        },
        Position {
            white: 34630270976,
            black: 19877243387904,
            white_to_move: true,
        },
        Position {
            white: 17450401792,
            black: 26491493023744,
            white_to_move: true,
        },
        Position {
            white: 223338299392,
            black: 17627016069120,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 17731101917184,
            white_to_move: true,
        },
        Position {
            white: 240518168576,
            black: 19808792346624,
            white_to_move: true,
        },
        Position {
            white: 223338299392,
            black: 22024995471360,
            white_to_move: true,
        },
        Position {
            white: 223338299392,
            black: 26423041982464,
            white_to_move: true,
        },
        Position {
            white: 188978561024,
            black: 52845680787456,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 88098772353024,
            white_to_move: true,
        },
        Position {
            white: 9024791440785408,
            black: 129252196352,
            white_to_move: true,
        },
        Position {
            white: 9024825800523776,
            black: 2285325778944,
            white_to_move: true,
        },
        Position {
            white: 9024808620654592,
            black: 4501528903680,
            white_to_move: true,
        },
        Position {
            white: 9024808620654592,
            black: 8899575414784,
            white_to_move: true,
        },
        Position {
            white: 9007250794348544,
            black: 4521260936069120,
            white_to_move: true,
        },
        Position {
            white: 51673827328,
            black: 35253360526336,
            white_to_move: true,
        },
        Position {
            white: 51540133888,
            black: 35253494480896,
            white_to_move: true,
        },
        Position {
            white: 51540133888,
            black: 35253561327616,
            white_to_move: true,
        },
        Position {
            white: 134744064,
            black: 35313489543168,
            white_to_move: true,
        },
        Position {
            white: 17314613248,
            black: 39685766250496,
            white_to_move: true,
        },
        Position {
            white: 51673825280,
            black: 35253361577984,
            white_to_move: true,
        },
        Position {
            white: 51540656128,
            black: 35253495005184,
            white_to_move: true,
        },
        Position {
            white: 51673825280,
            black: 35253363671040,
            white_to_move: true,
        },
        Position {
            white: 51540656128,
            black: 35253561851904,
            white_to_move: true,
        },
        Position {
            white: 135266304,
            black: 35313490067456,
            white_to_move: true,
        },
        Position {
            white: 17315135488,
            black: 39685766774784,
            white_to_move: true,
        },
        Position {
            white: 17180917760,
            black: 44083947503616,
            white_to_move: true,
        },
        Position {
            white: 51541704704,
            black: 35253495791616,
            white_to_move: true,
        },
        Position {
            white: 51541704704,
            black: 35254031613952,
            white_to_move: true,
        },
        Position {
            white: 270532608,
            black: 35313355849728,
            white_to_move: true,
        },
        Position {
            white: 51541704704,
            black: 35390933696512,
            white_to_move: true,
        },
        Position {
            white: 34630270976,
            black: 37469429432320,
            white_to_move: true,
        },
        Position {
            white: 17450401792,
            black: 44083679068160,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 35322751090688,
            white_to_move: true,
        },
        Position {
            white: 240518168576,
            black: 37400978391040,
            white_to_move: true,
        },
        Position {
            white: 223338299392,
            black: 39617181515776,
            white_to_move: true,
        },
        Position {
            white: 223338299392,
            black: 44015228026880,
            white_to_move: true,
        },
        Position {
            white: 188978561024,
            black: 52845680787456,
            white_to_move: true,
        },
        Position {
            white: 120259084288,
            black: 105690958397440,
            white_to_move: true,
        },
        Position {
            white: 68853957121,
            black: 34628829186,
            white_to_move: false,
        },
        Position {
            white: 68854482944,
            black: 34628305410,
            white_to_move: false,
        },
        Position {
            white: 68854153216,
            black: 34628698626,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 34359869954,
            white_to_move: false,
        },
        Position {
            white: 69659262976,
            black: 34360394242,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 269091330,
            white_to_move: false,
        },
        Position {
            white: 8899306717184,
            black: 269091330,
            white_to_move: false,
        },
        Position {
            white: 68719936000,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 68989747712,
            black: 34561196032,
            white_to_move: false,
        },
        Position {
            white: 68753424896,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 120326455808,
            black: 403308544,
            white_to_move: false,
        },
        Position {
            white: 135006720,
            black: 240786735104,
            white_to_move: false,
        },
        Position {
            white: 134676992,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 136053248,
            black: 240786735104,
            white_to_move: false,
        },
        Position {
            white: 168165888,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 939786752,
            black: 240518823936,
            white_to_move: false,
        },
        Position {
            white: 8830587240960,
            black: 206427521024,
            white_to_move: false,
        },
        Position {
            white: 35253226045952,
            black: 172067782656,
            white_to_move: false,
        },
        Position {
            white: 135006720,
            black: 17695533826048,
            white_to_move: false,
        },
        Position {
            white: 134676992,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 136053248,
            black: 17695533826048,
            white_to_move: false,
        },
        Position {
            white: 168165888,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 939786752,
            black: 17695265914880,
            white_to_move: false,
        },
        Position {
            white: 8830587240960,
            black: 17661174611968,
            white_to_move: false,
        },
        Position {
            white: 35253226045952,
            black: 17626814873600,
            white_to_move: false,
        },
        Position {
            white: 68853957120,
            black: 34628831240,
            white_to_move: false,
        },
        Position {
            white: 68988960768,
            black: 34494875656,
            white_to_move: false,
        },
        Position {
            white: 120259084288,
            black: 403572744,
            white_to_move: false,
        },
        Position {
            white: 68854220808,
            black: 34628567056,
            white_to_move: false,
        },
        Position {
            white: 68854481408,
            black: 34628306960,
            white_to_move: false,
        },
        Position {
            white: 68854677504,
            black: 34628175888,
            white_to_move: false,
        },
        Position {
            white: 69123702784,
            black: 34360133648,
            white_to_move: false,
        },
        Position {
            white: 69659525120,
            black: 34360133648,
            white_to_move: false,
        },
        Position {
            white: 120393826304,
            black: 268830736,
            white_to_move: false,
        },
        Position {
            white: 206561607680,
            black: 34360133648,
            white_to_move: false,
        },
        Position {
            white: 8899306979328,
            black: 268830736,
            white_to_move: false,
        },
        Position {
            white: 68853960192,
            black: 34628829184,
            white_to_move: false,
        },
        Position {
            white: 69123180544,
            black: 34360656896,
            white_to_move: false,
        },
        Position {
            white: 68887513088,
            black: 34628830208,
            white_to_move: false,
        },
        Position {
            white: 69659002880,
            black: 34360656896,
            white_to_move: false,
        },
        Position {
            white: 120393304064,
            black: 269353984,
            white_to_move: false,
        },
        Position {
            white: 8899306457088,
            black: 269353984,
            white_to_move: false,
        },
        Position {
            white: 68853959168,
            black: 34629877760,
            white_to_move: false,
        },
        Position {
            white: 69123184640,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 68854753280,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 68887513088,
            black: 34629877760,
            white_to_move: false,
        },
        Position {
            white: 69660051456,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 120393304064,
            black: 270401536,
            white_to_move: false,
        },
        Position {
            white: 8899306457088,
            black: 270401536,
            white_to_move: false,
        },
        Position {
            white: 68854483456,
            black: 34695413760,
            white_to_move: false,
        },
        Position {
            white: 68720461824,
            black: 34829500416,
            white_to_move: false,
        },
        Position {
            white: 68989487104,
            black: 34561458176,
            white_to_move: false,
        },
        Position {
            white: 68753819648,
            black: 34829631488,
            white_to_move: false,
        },
        Position {
            white: 77377046528,
            black: 34762784768,
            white_to_move: false,
        },
        Position {
            white: 120259610624,
            black: 470155264,
            white_to_move: false,
        },
        Position {
            white: 206427392000,
            black: 34561458176,
            white_to_move: false,
        },
        Position {
            white: 8899306981376,
            black: 335937536,
            white_to_move: false,
        },
        Position {
            white: 135006720,
            black: 240786735104,
            white_to_move: false,
        },
        Position {
            white: 135202816,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 168560640,
            black: 240786735104,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 240518561792,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 206427258880,
            white_to_move: false,
        },
        Position {
            white: 35253226309632,
            black: 172067520512,
            white_to_move: false,
        },
        Position {
            white: 70506586310656,
            black: 103079608320,
            white_to_move: false,
        },
        Position {
            white: 135006720,
            black: 17695533826048,
            white_to_move: false,
        },
        Position {
            white: 135202816,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 168560640,
            black: 17695533826048,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 17695265652736,
            white_to_move: false,
        },
        Position {
            white: 137842132992,
            black: 17695265652736,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 17661174349824,
            white_to_move: false,
        },
        Position {
            white: 35253226309632,
            black: 17626814611456,
            white_to_move: false,
        },
        Position {
            white: 985088,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 34342912,
            black: 35287854088192,
            white_to_move: false,
        },
        Position {
            white: 137707915264,
            black: 35287585914880,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 35253360394240,
            white_to_move: false,
        },
        Position {
            white: 68853957120,
            black: 34631974912,
            white_to_move: false,
        },
        Position {
            white: 68854220800,
            black: 34631712768,
            white_to_move: false,
        },
        Position {
            white: 69123182592,
            black: 34362753024,
            white_to_move: false,
        },
        Position {
            white: 68854751232,
            black: 34631188480,
            white_to_move: false,
        },
        Position {
            white: 69659000832,
            black: 34363801600,
            white_to_move: false,
        },
        Position {
            white: 120393302016,
            black: 272498688,
            white_to_move: false,
        },
        Position {
            white: 8899306455040,
            black: 272498688,
            white_to_move: false,
        },
        Position {
            white: 135528960,
            black: 240787259392,
            white_to_move: false,
        },
        Position {
            white: 135792640,
            black: 240786997248,
            white_to_move: false,
        },
        Position {
            white: 136249344,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 940572672,
            black: 240519086080,
            white_to_move: false,
        },
        Position {
            white: 8830588026880,
            black: 206427783168,
            white_to_move: false,
        },
        Position {
            white: 17661309222912,
            black: 171799609344,
            white_to_move: false,
        },
        Position {
            white: 35253226831872,
            black: 172068044800,
            white_to_move: false,
        },
        Position {
            white: 270009344,
            black: 35287585914880,
            white_to_move: false,
        },
        Position {
            white: 270467072,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 470810624,
            black: 35287452221440,
            white_to_move: false,
        },
        Position {
            white: 17583570944,
            black: 35287452221440,
            white_to_move: false,
        },
        Position {
            white: 4432675733504,
            black: 35253226700800,
            white_to_move: false,
        },
        Position {
            white: 17661175005184,
            black: 35218866962432,
            white_to_move: false,
        },
        Position {
            white: 69390828032,
            black: 34630926336,
            white_to_move: false,
        },
        Position {
            white: 69391091712,
            black: 34630664192,
            white_to_move: false,
        },
        Position {
            white: 69392670720,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 69660049408,
            black: 34362753024,
            white_to_move: false,
        },
        Position {
            white: 120930172928,
            black: 271450112,
            white_to_move: false,
        },
        Position {
            white: 8899843325952,
            black: 271450112,
            white_to_move: false,
        },
        Position {
            white: 671351296,
            black: 240787259392,
            white_to_move: false,
        },
        Position {
            white: 671614976,
            black: 240786997248,
            white_to_move: false,
        },
        Position {
            white: 8899843325952,
            black: 137708306432,
            white_to_move: false,
        },
        Position {
            white: 35391201607680,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 805831680,
            black: 35287585914880,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 35287452221440,
            white_to_move: false,
        },
        Position {
            white: 4433211555840,
            black: 35253226700800,
            white_to_move: false,
        },
        Position {
            white: 8865617805312,
            black: 35218866962432,
            white_to_move: false,
        },
        Position {
            white: 17661710827520,
            black: 35218866962432,
            white_to_move: false,
        },
        Position {
            white: 120326324480,
            black: 403439616,
            white_to_move: false,
        },
        Position {
            white: 120393564672,
            black: 336199680,
            white_to_move: false,
        },
        Position {
            white: 120326456320,
            black: 403308544,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 335937536,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 68026368,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 202244096,
            white_to_move: false,
        },
        Position {
            white: 86033826304,
            black: 4432675340288,
            white_to_move: false,
        },
        Position {
            white: 86034089984,
            black: 4432675078144,
            white_to_move: false,
        },
        Position {
            white: 86303047680,
            black: 4432407166976,
            white_to_move: false,
        },
        Position {
            white: 86838870016,
            black: 4432407166976,
            white_to_move: false,
        },
        Position {
            white: 8916486324224,
            black: 4398315864064,
            white_to_move: false,
        },
        Position {
            white: 1130383986917376,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 86033826304,
            black: 8830721851392,
            white_to_move: false,
        },
        Position {
            white: 86303047680,
            black: 8830453678080,
            white_to_move: false,
        },
        Position {
            white: 1134781899210752,
            black: 34763309056,
            white_to_move: false,
        },
        Position {
            white: 4512481619738624,
            black: 34763309056,
            white_to_move: false,
        },
        Position {
            white: 51674087936,
            black: 17661174611968,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 17661174349824,
            white_to_move: false,
        },
        Position {
            white: 51944357888,
            black: 17660906438656,
            white_to_move: false,
        },
        Position {
            white: 52479131648,
            black: 17660906438656,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 17592455397376,
            white_to_move: false,
        },
        Position {
            white: 35304765390848,
            black: 17592455397376,
            white_to_move: false,
        },
        Position {
            white: 9024843114610688,
            black: 68988829696,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 35253360394240,
            white_to_move: false,
        },
        Position {
            white: 51674873856,
            black: 35253360918528,
            white_to_move: false,
        },
        Position {
            white: 51810140160,
            black: 35253226700800,
            white_to_move: false,
        },
        Position {
            white: 257698037760,
            black: 35184775659520,
            white_to_move: false,
        },
        Position {
            white: 8899239477504,
            black: 403439616,
            white_to_move: false,
        },
        Position {
            white: 8899306717696,
            black: 336199680,
            white_to_move: false,
        },
        Position {
            white: 8899306981376,
            black: 335937536,
            white_to_move: false,
        },
        Position {
            white: 8899441721344,
            black: 202244096,
            white_to_move: false,
        },
        Position {
            white: 8899442769920,
            black: 202244096,
            white_to_move: false,
        },
        Position {
            white: 8864946979328,
            black: 4432675340288,
            white_to_move: false,
        },
        Position {
            white: 8864947243008,
            black: 4432675078144,
            white_to_move: false,
        },
        Position {
            white: 8865216200704,
            black: 4432407166976,
            white_to_move: false,
        },
        Position {
            white: 8865752023040,
            black: 4432407166976,
            white_to_move: false,
        },
        Position {
            white: 8916486324224,
            black: 4398315864064,
            white_to_move: false,
        },
        Position {
            white: 15462016483328,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 8830587240960,
            black: 17661174611968,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 17661174349824,
            white_to_move: false,
        },
        Position {
            white: 8830857510912,
            black: 17660906438656,
            white_to_move: false,
        },
        Position {
            white: 8900111761408,
            black: 17592186961920,
            white_to_move: false,
        },
        Position {
            white: 9036745408512,
            black: 17592455397376,
            white_to_move: false,
        },
        Position {
            white: 61675864588288,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 9033622027763712,
            black: 68988829696,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 35253360394240,
            white_to_move: false,
        },
        Position {
            white: 8830723293184,
            black: 35253226700800,
            white_to_move: false,
        },
        Position {
            white: 8899709108224,
            black: 35184775659520,
            white_to_move: false,
        },
        Position {
            white: 9036611190784,
            black: 35184775659520,
            white_to_move: false,
        },
        Position {
            white: 68853957120,
            black: 2260630535536640,
            white_to_move: false,
        },
        Position {
            white: 68988960768,
            black: 2260630401581056,
            white_to_move: false,
        },
        Position {
            white: 120259084288,
            black: 2260596310278144,
            white_to_move: false,
        },
        Position {
            white: 1134764719341568,
            black: 2251834576994304,
            white_to_move: false,
        },
        Position {
            white: 69123965441,
            black: 34426847232,
            white_to_move: false,
        },
        Position {
            white: 68989878272,
            black: 34561065472,
            white_to_move: false,
        },
        Position {
            white: 69224366080,
            black: 34360001024,
            white_to_move: false,
        },
        Position {
            white: 77646528512,
            black: 34494218752,
            white_to_move: false,
        },
        Position {
            white: 120663310336,
            black: 67371520,
            white_to_move: false,
        },
        Position {
            white: 4501395734528,
            black: 201589248,
            white_to_move: false,
        },
        Position {
            white: 8899576463360,
            black: 67371520,
            white_to_move: false,
        },
        Position {
            white: 68989486082,
            black: 34561327104,
            white_to_move: false,
        },
        Position {
            white: 69123965440,
            black: 34426848256,
            white_to_move: false,
        },
        Position {
            white: 68989878272,
            black: 34561065984,
            white_to_move: false,
        },
        Position {
            white: 69224366080,
            black: 34360001536,
            white_to_move: false,
        },
        Position {
            white: 77646528512,
            black: 34494219264,
            white_to_move: false,
        },
        Position {
            white: 120663310336,
            black: 67372032,
            white_to_move: false,
        },
        Position {
            white: 4501395734528,
            black: 201589760,
            white_to_move: false,
        },
        Position {
            white: 8899576463360,
            black: 67372032,
            white_to_move: false,
        },
        Position {
            white: 68989224964,
            black: 34561589248,
            white_to_move: false,
        },
        Position {
            white: 68989224976,
            black: 34561589248,
            white_to_move: false,
        },
        Position {
            white: 68989748224,
            black: 34561067008,
            white_to_move: false,
        },
        Position {
            white: 69224103936,
            black: 34360264704,
            white_to_move: false,
        },
        Position {
            white: 120730157056,
            black: 526336,
            white_to_move: false,
        },
        Position {
            white: 4501395472384,
            black: 201852928,
            white_to_move: false,
        },
        Position {
            white: 68989227024,
            black: 34561589248,
            white_to_move: false,
        },
        Position {
            white: 68989748224,
            black: 34561069056,
            white_to_move: false,
        },
        Position {
            white: 69224103936,
            black: 34360266752,
            white_to_move: false,
        },
        Position {
            white: 120730157056,
            black: 528384,
            white_to_move: false,
        },
        Position {
            white: 4501395472384,
            black: 201854976,
            white_to_move: false,
        },
        Position {
            white: 68989751296,
            black: 34561073152,
            white_to_move: false,
        },
        Position {
            white: 68991844352,
            black: 34561073152,
            white_to_move: false,
        },
        Position {
            white: 69223579648,
            black: 34360795136,
            white_to_move: false,
        },
        Position {
            white: 77645742080,
            black: 34495012864,
            white_to_move: false,
        },
        Position {
            white: 120595415040,
            black: 135274496,
            white_to_move: false,
        },
        Position {
            white: 4501394948096,
            black: 202383360,
            white_to_move: false,
        },
        Position {
            white: 8899575676928,
            black: 68165632,
            white_to_move: false,
        },
        Position {
            white: 68727603200,
            black: 34829500416,
            white_to_move: false,
        },
        Position {
            white: 77378355200,
            black: 34764488704,
            white_to_move: false,
        },
        Position {
            white: 120462245888,
            black: 270532608,
            white_to_move: false,
        },
        Position {
            white: 206428700672,
            black: 34563162112,
            white_to_move: false,
        },
        Position {
            white: 8899308290048,
            black: 337641472,
            white_to_move: false,
        },
        Position {
            white: 69262376960,
            black: 34829500416,
            white_to_move: false,
        },
        Position {
            white: 77378355200,
            black: 35299262464,
            white_to_move: false,
        },
        Position {
            white: 120462245888,
            black: 805306368,
            white_to_move: false,
        },
        Position {
            white: 206428700672,
            black: 35097935872,
            white_to_move: false,
        },
        Position {
            white: 344136089600,
            black: 34829500416,
            white_to_move: false,
        },
        Position {
            white: 8899308290048,
            black: 872415232,
            white_to_move: false,
        },
        Position {
            white: 505151488,
            black: 240518168576,
            white_to_move: false,
        },
        Position {
            white: 8927313920,
            black: 240652386304,
            white_to_move: false,
        },
        Position {
            white: 17651466240,
            black: 240518168576,
            white_to_move: false,
        },
        Position {
            white: 4432676519936,
            black: 206359756800,
            white_to_move: false,
        },
        Position {
            white: 8830857248768,
            black: 206225539072,
            white_to_move: false,
        },
        Position {
            white: 17661175791616,
            black: 172000018432,
            white_to_move: false,
        },
        Position {
            white: 35253496053760,
            black: 171865800704,
            white_to_move: false,
        },
        Position {
            white: 70506453401600,
            black: 103280541696,
            white_to_move: false,
        },
        Position {
            white: 505151488,
            black: 35287451303936,
            white_to_move: false,
        },
        Position {
            white: 8927313920,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 17651466240,
            black: 35287451303936,
            white_to_move: false,
        },
        Position {
            white: 4432676519936,
            black: 35253292892160,
            white_to_move: false,
        },
        Position {
            white: 8830857248768,
            black: 35253158674432,
            white_to_move: false,
        },
        Position {
            white: 17661175791616,
            black: 35218933153792,
            white_to_move: false,
        },
        Position {
            white: 18049652005535744,
            black: 34426847232,
            white_to_move: false,
        },
        Position {
            white: 264208,
            black: 240921346056,
            white_to_move: false,
        },
        Position {
            white: 1835008,
            black: 240920823816,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 172067653640,
            white_to_move: false,
        },
        Position {
            white: 919552,
            black: 240920822272,
            white_to_move: false,
        },
        Position {
            white: 34342912,
            black: 240920822272,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 206427128320,
            white_to_move: false,
        },
        Position {
            white: 70506452092928,
            black: 103213695488,
            white_to_move: false,
        },
        Position {
            white: 134482948,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 134483456,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 136054784,
            black: 240786605056,
            white_to_move: false,
        },
        Position {
            white: 939788288,
            black: 240518693888,
            white_to_move: false,
        },
        Position {
            white: 8830587242496,
            black: 206427390976,
            white_to_move: false,
        },
        Position {
            white: 35253226047488,
            black: 172067652608,
            white_to_move: false,
        },
        Position {
            white: 8657831936,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 17247766528,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 8830587766784,
            black: 206493974528,
            white_to_move: false,
        },
        Position {
            white: 35253226571776,
            black: 172134236160,
            white_to_move: false,
        },
        Position {
            white: 70506452355072,
            black: 103280541696,
            white_to_move: false,
        },
        Position {
            white: 1966080,
            black: 240920822272,
            white_to_move: false,
        },
        Position {
            white: 17315659776,
            black: 240786866688,
            white_to_move: false,
        },
        Position {
            white: 8830588551168,
            black: 206427128320,
            white_to_move: false,
        },
        Position {
            white: 17661175529472,
            black: 171933172224,
            white_to_move: false,
        },
        Position {
            white: 70506453139456,
            black: 103213695488,
            white_to_move: false,
        },
        Position {
            white: 135529476,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 136054784,
            black: 240786605056,
            white_to_move: false,
        },
        Position {
            white: 940834816,
            black: 240518693888,
            white_to_move: false,
        },
        Position {
            white: 8830588289024,
            black: 206427390976,
            white_to_move: false,
        },
        Position {
            white: 17661309485056,
            black: 171799217152,
            white_to_move: false,
        },
        Position {
            white: 35253227094016,
            black: 172067652608,
            white_to_move: false,
        },
        Position {
            white: 1312772,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 1312784,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 17315397632,
            black: 240787130368,
            white_to_move: false,
        },
        Position {
            white: 17661175267328,
            black: 171933435904,
            white_to_move: false,
        },
        Position {
            white: 35253227094016,
            black: 172067653632,
            white_to_move: false,
        },
        Position {
            white: 135008288,
            black: 240787652608,
            white_to_move: false,
        },
        Position {
            white: 136060928,
            black: 240786608128,
            white_to_move: false,
        },
        Position {
            white: 138149888,
            black: 240786608128,
            white_to_move: false,
        },
        Position {
            white: 940310528,
            black: 240519221248,
            white_to_move: false,
        },
        Position {
            white: 8830587764736,
            black: 206427918336,
            white_to_move: false,
        },
        Position {
            white: 35253226569728,
            black: 172068179968,
            white_to_move: false,
        },
        Position {
            white: 70506586570752,
            black: 103080267776,
            white_to_move: false,
        },
        Position {
            white: 8658878464,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 17383030784,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 8830588813312,
            black: 206493974528,
            white_to_move: false,
        },
        Position {
            white: 17661175791616,
            black: 172000018432,
            white_to_move: false,
        },
        Position {
            white: 35253227618304,
            black: 172134236160,
            white_to_move: false,
        },
        Position {
            white: 70506453401600,
            black: 103280541696,
            white_to_move: false,
        },
        Position {
            white: 805831680,
            black: 240652648960,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 240518955520,
            white_to_move: false,
        },
        Position {
            white: 4433211555840,
            black: 206293434880,
            white_to_move: false,
        },
        Position {
            white: 8865617805312,
            black: 171933696512,
            white_to_move: false,
        },
        Position {
            white: 17661710827520,
            black: 171933696512,
            white_to_move: false,
        },
        Position {
            white: 35322616348672,
            black: 103214219776,
            white_to_move: false,
        },
        Position {
            white: 70506988437504,
            black: 103214219776,
            white_to_move: false,
        },
        Position {
            white: 939786752,
            black: 240518823936,
            white_to_move: false,
        },
        Position {
            white: 940049408,
            black: 240518561792,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 240518561792,
            white_to_move: false,
        },
        Position {
            white: 4433345773568,
            black: 206159347712,
            white_to_move: false,
        },
        Position {
            white: 8900111761408,
            black: 137439870976,
            white_to_move: false,
        },
        Position {
            white: 17661845045248,
            black: 171799609344,
            white_to_move: false,
        },
        Position {
            white: 35391470043136,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 70507122655232,
            black: 103080132608,
            white_to_move: false,
        },
        Position {
            white: 672925696,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 672407552,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 675020800,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 8899843588096,
            black: 137708961792,
            white_to_move: false,
        },
        Position {
            white: 35391201869824,
            black: 34629746688,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 241325572096,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 241325572096,
            white_to_move: false,
        },
        Position {
            white: 2013528064,
            black: 240520790016,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 206966358016,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 172606619648,
            white_to_move: false,
        },
        Position {
            white: 403440640,
            black: 241059233792,
            white_to_move: false,
        },
        Position {
            white: 403441664,
            black: 241059233792,
            white_to_move: false,
        },
        Position {
            white: 404488192,
            black: 241059233792,
            white_to_move: false,
        },
        Position {
            white: 2013528064,
            black: 240522887168,
            white_to_move: false,
        },
        Position {
            white: 4432809164800,
            black: 206700019712,
            white_to_move: false,
        },
        Position {
            white: 8830855675904,
            black: 206700019712,
            white_to_move: false,
        },
        Position {
            white: 17661308436480,
            black: 172340281344,
            white_to_move: false,
        },
        Position {
            white: 35253494480896,
            black: 172340281344,
            white_to_move: false,
        },
        Position {
            white: 70506586046464,
            black: 103620804608,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 206427128320,
            white_to_move: false,
        },
        Position {
            white: 8830723293184,
            black: 206293434880,
            white_to_move: false,
        },
        Position {
            white: 8899709108224,
            black: 137842393600,
            white_to_move: false,
        },
        Position {
            white: 9311489097728,
            black: 403440128,
            white_to_move: false,
        },
        Position {
            white: 8830587240960,
            black: 206427521024,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 206427258880,
            white_to_move: false,
        },
        Position {
            white: 8830857510912,
            black: 206159347712,
            white_to_move: false,
        },
        Position {
            white: 8900111761408,
            black: 137439870976,
            white_to_move: false,
        },
        Position {
            white: 9311623315456,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 44083678543872,
            black: 137708306432,
            white_to_move: false,
        },
        Position {
            white: 8830587766784,
            black: 206493974528,
            white_to_move: false,
        },
        Position {
            white: 8830520262656,
            black: 206561607680,
            white_to_move: false,
        },
        Position {
            white: 8830454595584,
            black: 206628192256,
            white_to_move: false,
        },
        Position {
            white: 8830723555328,
            black: 206360281088,
            white_to_move: false,
        },
        Position {
            white: 8899709370368,
            black: 137909239808,
            white_to_move: false,
        },
        Position {
            white: 8847700000768,
            black: 206561607680,
            white_to_move: false,
        },
        Position {
            white: 9311489359872,
            black: 470286336,
            white_to_move: false,
        },
        Position {
            white: 44083678806016,
            black: 137775022080,
            white_to_move: false,
        },
        Position {
            white: 8796228028416,
            black: 257966473216,
            white_to_move: false,
        },
        Position {
            white: 8796229074944,
            black: 257966473216,
            white_to_move: false,
        },
        Position {
            white: 8813440925696,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 8865752285184,
            black: 188979085312,
            white_to_move: false,
        },
        Position {
            white: 11012430626816,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 44049319067648,
            black: 189247520768,
            white_to_move: false,
        },
        Position {
            white: 8796228028416,
            black: 4638833115136,
            white_to_move: false,
        },
        Position {
            white: 8796229074944,
            black: 4638833115136,
            white_to_move: false,
        },
        Position {
            white: 8865752285184,
            black: 4569845727232,
            white_to_move: false,
        },
        Position {
            white: 15393297268736,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 44049319067648,
            black: 4570114162688,
            white_to_move: false,
        },
        Position {
            white: 34494744576,
            black: 1134902426730496,
            white_to_move: false,
        },
        Position {
            white: 34495791104,
            black: 1134902426730496,
            white_to_move: false,
        },
        Position {
            white: 34764750848,
            black: 1134902158819328,
            white_to_move: false,
        },
        Position {
            white: 35299524608,
            black: 1134902158819328,
            white_to_move: false,
        },
        Position {
            white: 515530555392,
            black: 1134696268824576,
            white_to_move: false,
        },
        Position {
            white: 35287585783808,
            black: 1134833707778048,
            white_to_move: false,
        },
        Position {
            white: 2260630400925696,
            black: 1126106334232576,
            white_to_move: false,
        },
        Position {
            white: 1835008,
            black: 2260836827529216,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 2260767974359040,
            white_to_move: false,
        },
        Position {
            white: 35253226045952,
            black: 172067782656,
            white_to_move: false,
        },
        Position {
            white: 35253226309632,
            black: 172067520512,
            white_to_move: false,
        },
        Position {
            white: 35253495267328,
            black: 171799609344,
            white_to_move: false,
        },
        Position {
            white: 35391470043136,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 35304765390848,
            black: 137708306432,
            white_to_move: false,
        },
        Position {
            white: 35665542643712,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 44083678543872,
            black: 137708306432,
            white_to_move: false,
        },
        Position {
            white: 35253361836032,
            black: 172000018432,
            white_to_move: false,
        },
        Position {
            white: 35391067652096,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 35304698544128,
            black: 137842130944,
            white_to_move: false,
        },
        Position {
            white: 35665408688128,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 264208,
            black: 17695668437000,
            white_to_move: false,
        },
        Position {
            white: 1835008,
            black: 17695667914760,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 17626814744584,
            white_to_move: false,
        },
        Position {
            white: 919552,
            black: 17695667913216,
            white_to_move: false,
        },
        Position {
            white: 34342912,
            black: 17695667913216,
            white_to_move: false,
        },
        Position {
            white: 137707915264,
            black: 17695399739904,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 17661174219264,
            white_to_move: false,
        },
        Position {
            white: 134482948,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 134483456,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 136054784,
            black: 17695533696000,
            white_to_move: false,
        },
        Position {
            white: 939788288,
            black: 17695265784832,
            white_to_move: false,
        },
        Position {
            white: 8830587242496,
            black: 17661174481920,
            white_to_move: false,
        },
        Position {
            white: 35253226047488,
            black: 17626814743552,
            white_to_move: false,
        },
        Position {
            white: 8657831936,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 17247766528,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 137708177408,
            black: 17695466586112,
            white_to_move: false,
        },
        Position {
            white: 8830587766784,
            black: 17661241065472,
            white_to_move: false,
        },
        Position {
            white: 35253226571776,
            black: 17626881327104,
            white_to_move: false,
        },
        Position {
            white: 1966080,
            black: 17695667913216,
            white_to_move: false,
        },
        Position {
            white: 17315659776,
            black: 17695533957632,
            white_to_move: false,
        },
        Position {
            white: 137708961792,
            black: 17695399739904,
            white_to_move: false,
        },
        Position {
            white: 8830588551168,
            black: 17661174219264,
            white_to_move: false,
        },
        Position {
            white: 4521260802899968,
            black: 34494218752,
            white_to_move: false,
        },
        Position {
            white: 135529476,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 136054784,
            black: 17695533696000,
            white_to_move: false,
        },
        Position {
            white: 940834816,
            black: 17695265784832,
            white_to_move: false,
        },
        Position {
            white: 8830588289024,
            black: 17661174481920,
            white_to_move: false,
        },
        Position {
            white: 35253227094016,
            black: 17626814743552,
            white_to_move: false,
        },
        Position {
            white: 4521260936855552,
            black: 34360263680,
            white_to_move: false,
        },
        Position {
            white: 1312772,
            black: 17695668436992,
            white_to_move: false,
        },
        Position {
            white: 1312784,
            black: 17695668436992,
            white_to_move: false,
        },
        Position {
            white: 17315397632,
            black: 17695534221312,
            white_to_move: false,
        },
        Position {
            white: 35253227094016,
            black: 17626814744576,
            white_to_move: false,
        },
        Position {
            white: 4521260802637824,
            black: 34494482432,
            white_to_move: false,
        },
        Position {
            white: 135008288,
            black: 17695534743552,
            white_to_move: false,
        },
        Position {
            white: 136060928,
            black: 17695533699072,
            white_to_move: false,
        },
        Position {
            white: 138149888,
            black: 17695533699072,
            white_to_move: false,
        },
        Position {
            white: 940310528,
            black: 17695266312192,
            white_to_move: false,
        },
        Position {
            white: 137842393088,
            black: 17695266312192,
            white_to_move: false,
        },
        Position {
            white: 8830587764736,
            black: 17661175009280,
            white_to_move: false,
        },
        Position {
            white: 35253226569728,
            black: 17626815270912,
            white_to_move: false,
        },
        Position {
            white: 8658878464,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 17383030784,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 137709223936,
            black: 17695466586112,
            white_to_move: false,
        },
        Position {
            white: 8830588813312,
            black: 17661241065472,
            white_to_move: false,
        },
        Position {
            white: 35253227618304,
            black: 17626881327104,
            white_to_move: false,
        },
        Position {
            white: 4521260803162112,
            black: 34561064960,
            white_to_move: false,
        },
        Position {
            white: 805831680,
            black: 17695399739904,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 17695266046464,
            white_to_move: false,
        },
        Position {
            white: 4433211555840,
            black: 17661040525824,
            white_to_move: false,
        },
        Position {
            white: 8865617805312,
            black: 17626680787456,
            white_to_move: false,
        },
        Position {
            white: 4521261338198016,
            black: 34494743040,
            white_to_move: false,
        },
        Position {
            white: 939786752,
            black: 17695265914880,
            white_to_move: false,
        },
        Position {
            white: 940049408,
            black: 17695265652736,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 17695265652736,
            white_to_move: false,
        },
        Position {
            white: 4433345773568,
            black: 17660906438656,
            white_to_move: false,
        },
        Position {
            white: 8900111761408,
            black: 17592186961920,
            white_to_move: false,
        },
        Position {
            white: 35254031089664,
            black: 17626546700288,
            white_to_move: false,
        },
        Position {
            white: 4521261472415744,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 672925696,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 672407552,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 675020800,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 8899843588096,
            black: 17592456052736,
            white_to_move: false,
        },
        Position {
            white: 35253762916352,
            black: 17626815791104,
            white_to_move: false,
        },
        Position {
            white: 671877120,
            black: 17695535792128,
            white_to_move: false,
        },
        Position {
            white: 673456128,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 672923648,
            black: 17695535792128,
            white_to_move: false,
        },
        Position {
            white: 8899843588096,
            black: 17592457101312,
            white_to_move: false,
        },
        Position {
            white: 35253762916352,
            black: 17626816839680,
            white_to_move: false,
        },
        Position {
            white: 403440640,
            black: 17695806324736,
            white_to_move: false,
        },
        Position {
            white: 403441664,
            black: 17695806324736,
            white_to_move: false,
        },
        Position {
            white: 404488192,
            black: 17695806324736,
            white_to_move: false,
        },
        Position {
            white: 2013528064,
            black: 17695269978112,
            white_to_move: false,
        },
        Position {
            white: 4432809164800,
            black: 17661447110656,
            white_to_move: false,
        },
        Position {
            white: 8830855675904,
            black: 17661447110656,
            white_to_move: false,
        },
        Position {
            white: 35253494480896,
            black: 17627087372288,
            white_to_move: false,
        },
        Position {
            white: 4521260935806976,
            black: 34901327872,
            white_to_move: false,
        },
        Position {
            white: 671877120,
            black: 17832972648448,
            white_to_move: false,
        },
        Position {
            white: 672923648,
            black: 17832972648448,
            white_to_move: false,
        },
        Position {
            white: 8899843588096,
            black: 17729893957632,
            white_to_move: false,
        },
        Position {
            white: 35391201869824,
            black: 17626814742528,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 17661174219264,
            white_to_move: false,
        },
        Position {
            white: 8830723293184,
            black: 17661040525824,
            white_to_move: false,
        },
        Position {
            white: 8899709108224,
            black: 17592589484544,
            white_to_move: false,
        },
        Position {
            white: 9036611190784,
            black: 17592589484544,
            white_to_move: false,
        },
        Position {
            white: 61607010893824,
            black: 69122916864,
            white_to_move: false,
        },
        Position {
            white: 9033621893545984,
            black: 69122916864,
            white_to_move: false,
        },
        Position {
            white: 8830587766784,
            black: 17661241065472,
            white_to_move: false,
        },
        Position {
            white: 8796094857216,
            black: 17695735021568,
            white_to_move: false,
        },
        Position {
            white: 8865349632000,
            black: 17627016069120,
            white_to_move: false,
        },
        Position {
            white: 8813340262400,
            black: 17695668436992,
            white_to_move: false,
        },
        Position {
            white: 61641505112064,
            black: 34695806976,
            white_to_move: false,
        },
        Position {
            white: 8796228028416,
            black: 17712713564160,
            white_to_move: false,
        },
        Position {
            white: 8796229074944,
            black: 17712713564160,
            white_to_move: false,
        },
        Position {
            white: 8813440925696,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 8865752285184,
            black: 17643726176256,
            white_to_move: false,
        },
        Position {
            white: 11012430626816,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 61641505112064,
            black: 51808567296,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 30889673228288,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 30889673228288,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 30889405317120,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 30820954275840,
            white_to_move: false,
        },
        Position {
            white: 2260630400925696,
            black: 22059220992000,
            white_to_move: false,
        },
        Position {
            white: 34494744576,
            black: 1152357173821440,
            white_to_move: false,
        },
        Position {
            white: 34495791104,
            black: 1152357173821440,
            white_to_move: false,
        },
        Position {
            white: 34764750848,
            black: 1152356905910272,
            white_to_move: false,
        },
        Position {
            white: 35299524608,
            black: 1152356905910272,
            white_to_move: false,
        },
        Position {
            white: 240652648448,
            black: 1152288454868992,
            white_to_move: false,
        },
        Position {
            white: 35287585783808,
            black: 1152288454868992,
            white_to_move: false,
        },
        Position {
            white: 2260630400925696,
            black: 1143561081323520,
            white_to_move: false,
        },
        Position {
            white: 9024825935003648,
            black: 1134764988301312,
            white_to_move: false,
        },
        Position {
            white: 1835008,
            black: 2278291574620160,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 2278222721449984,
            white_to_move: false,
        },
        Position {
            white: 35253226045952,
            black: 17626814873600,
            white_to_move: false,
        },
        Position {
            white: 35253226309632,
            black: 17626814611456,
            white_to_move: false,
        },
        Position {
            white: 35253495267328,
            black: 17626546700288,
            white_to_move: false,
        },
        Position {
            white: 35254031089664,
            black: 17626546700288,
            white_to_move: false,
        },
        Position {
            white: 35304765390848,
            black: 17592455397376,
            white_to_move: false,
        },
        Position {
            white: 61675864588288,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 4556445039198208,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 68988967936,
            black: 34494488576,
            white_to_move: false,
        },
        Position {
            white: 69122393088,
            black: 34361321472,
            white_to_move: false,
        },
        Position {
            white: 69189239808,
            black: 34361321472,
            white_to_move: false,
        },
        Position {
            white: 120527520768,
            black: 135800832,
            white_to_move: false,
        },
        Position {
            white: 4501394162688,
            black: 135800832,
            white_to_move: false,
        },
        Position {
            white: 69122393600,
            black: 34361319424,
            white_to_move: false,
        },
        Position {
            white: 68988965888,
            black: 34494750720,
            white_to_move: false,
        },
        Position {
            white: 69189501952,
            black: 34361319424,
            white_to_move: false,
        },
        Position {
            white: 120527520768,
            black: 136060928,
            white_to_move: false,
        },
        Position {
            white: 4501394162688,
            black: 136060928,
            white_to_move: false,
        },
        Position {
            white: 68989490176,
            black: 34496061440,
            white_to_move: false,
        },
        Position {
            white: 68854481920,
            black: 34631327744,
            white_to_move: false,
        },
        Position {
            white: 68727342080,
            black: 34762399744,
            white_to_move: false,
        },
        Position {
            white: 120259609600,
            black: 405807104,
            white_to_move: false,
        },
        Position {
            white: 206427390976,
            black: 34497110016,
            white_to_move: false,
        },
        Position {
            white: 8899306980352,
            black: 271589376,
            white_to_move: false,
        },
        Position {
            white: 68989490176,
            black: 35030835200,
            white_to_move: false,
        },
        Position {
            white: 68854481920,
            black: 35166101504,
            white_to_move: false,
        },
        Position {
            white: 68723147776,
            black: 35299270656,
            white_to_move: false,
        },
        Position {
            white: 69261067264,
            black: 34763448320,
            white_to_move: false,
        },
        Position {
            white: 120259609600,
            black: 940580864,
            white_to_move: false,
        },
        Position {
            white: 206427390976,
            black: 35031883776,
            white_to_move: false,
        },
        Position {
            white: 8899306980352,
            black: 806363136,
            white_to_move: false,
        },
        Position {
            white: 270013440,
            black: 240652394496,
            white_to_move: false,
        },
        Position {
            white: 272106496,
            black: 240652394496,
            white_to_move: false,
        },
        Position {
            white: 470287360,
            black: 240519225344,
            white_to_move: false,
        },
        Position {
            white: 4432675210240,
            black: 206293704704,
            white_to_move: false,
        },
        Position {
            white: 8830855939072,
            black: 206159486976,
            white_to_move: false,
        },
        Position {
            white: 17661174481920,
            black: 171933966336,
            white_to_move: false,
        },
        Position {
            white: 70506452091904,
            black: 103214489600,
            white_to_move: false,
        },
        Position {
            white: 3671040,
            black: 17695667920896,
            white_to_move: false,
        },
        Position {
            white: 137707914240,
            black: 17695400534016,
            white_to_move: false,
        },
        Position {
            white: 8830587503616,
            black: 17661175013376,
            white_to_move: false,
        },
        Position {
            white: 270013440,
            black: 35287585529856,
            white_to_move: false,
        },
        Position {
            white: 272106496,
            black: 35287585529856,
            white_to_move: false,
        },
        Position {
            white: 470287360,
            black: 35287452360704,
            white_to_move: false,
        },
        Position {
            white: 4432675210240,
            black: 35253226840064,
            white_to_move: false,
        },
        Position {
            white: 8830855939072,
            black: 35253092622336,
            white_to_move: false,
        },
        Position {
            white: 17661174481920,
            black: 35218867101696,
            white_to_move: false,
        },
        Position {
            white: 68988964880,
            black: 34494488608,
            white_to_move: false,
        },
        Position {
            white: 68988969024,
            black: 34494484512,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 34493968416,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 34359750688,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 34360274976,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 536608,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 134754336,
            white_to_move: false,
        },
        Position {
            white: 68988962820,
            black: 34494492672,
            white_to_move: false,
        },
        Position {
            white: 68988964880,
            black: 34494490624,
            white_to_move: false,
        },
        Position {
            white: 68988969024,
            black: 34494486528,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 34493970432,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 34359752704,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 34360276992,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 538624,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 134756352,
            white_to_move: false,
        },
        Position {
            white: 68719505408,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 68853960704,
            black: 34631852032,
            white_to_move: false,
        },
        Position {
            white: 68787113984,
            black: 34765545472,
            white_to_move: false,
        },
        Position {
            white: 69795319808,
            black: 34763972608,
            white_to_move: false,
        },
        Position {
            white: 120259088384,
            black: 406331392,
            white_to_move: false,
        },
        Position {
            white: 68720537664,
            black: 35299786752,
            white_to_move: false,
        },
        Position {
            white: 68720553984,
            black: 35299786752,
            white_to_move: false,
        },
        Position {
            white: 68855533568,
            black: 35165052928,
            white_to_move: false,
        },
        Position {
            white: 69261594624,
            black: 34762924032,
            white_to_move: false,
        },
        Position {
            white: 68788162560,
            black: 35299270656,
            white_to_move: false,
        },
        Position {
            white: 120394354688,
            black: 805838848,
            white_to_move: false,
        },
        Position {
            white: 344135307264,
            black: 34762924032,
            white_to_move: false,
        },
        Position {
            white: 1060928,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 1077248,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 1839104,
            black: 240920829952,
            white_to_move: false,
        },
        Position {
            white: 68685824,
            black: 240920829952,
            white_to_move: false,
        },
        Position {
            white: 17315139584,
            black: 240787136512,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 171933442048,
            white_to_move: false,
        },
        Position {
            white: 269496384,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 270013440,
            black: 35287585529856,
            white_to_move: false,
        },
        Position {
            white: 269512704,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 270274560,
            black: 35287585529856,
            white_to_move: false,
        },
        Position {
            white: 471339008,
            black: 35287451312128,
            white_to_move: false,
        },
        Position {
            white: 17583575040,
            black: 35287451836416,
            white_to_move: false,
        },
        Position {
            white: 4432675737600,
            black: 35253226315776,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 35218866577408,
            white_to_move: false,
        },
        Position {
            white: 69123186752,
            black: 34360655872,
            white_to_move: false,
        },
        Position {
            white: 69122392576,
            black: 34361450496,
            white_to_move: false,
        },
        Position {
            white: 69122655232,
            black: 34361188352,
            white_to_move: false,
        },
        Position {
            white: 69122656256,
            black: 34361188352,
            white_to_move: false,
        },
        Position {
            white: 69123182592,
            black: 34360664064,
            white_to_move: false,
        },
        Position {
            white: 120661737472,
            black: 1974272,
            white_to_move: false,
        },
        Position {
            white: 4501528379392,
            black: 1974272,
            white_to_move: false,
        },
        Position {
            white: 8899574890496,
            black: 1974272,
            white_to_move: false,
        },
        Position {
            white: 68855013440,
            black: 34630795264,
            white_to_move: false,
        },
        Position {
            white: 68854482944,
            black: 34631327744,
            white_to_move: false,
        },
        Position {
            white: 69123444736,
            black: 34362368000,
            white_to_move: false,
        },
        Position {
            white: 68861820928,
            black: 34628182016,
            white_to_move: false,
        },
        Position {
            white: 69659262976,
            black: 34363416576,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 272113664,
            white_to_move: false,
        },
        Position {
            white: 8899306717184,
            black: 272113664,
            white_to_move: false,
        },
        Position {
            white: 68988699648,
            black: 51674882048,
            white_to_move: false,
        },
        Position {
            white: 68989227008,
            black: 51674357760,
            white_to_move: false,
        },
        Position {
            white: 68991844352,
            black: 51673833472,
            white_to_move: false,
        },
        Position {
            white: 69189500928,
            black: 51541188608,
            white_to_move: false,
        },
        Position {
            white: 129117716480,
            black: 135798784,
            white_to_move: false,
        },
        Position {
            white: 4501394423808,
            black: 17315667968,
            white_to_move: false,
        },
        Position {
            white: 135536704,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 240787660800,
            white_to_move: false,
        },
        Position {
            white: 138149888,
            black: 240786612224,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 240519749632,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 206428446720,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 172068708352,
            white_to_move: false,
        },
        Position {
            white: 135536704,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 17695534751744,
            white_to_move: false,
        },
        Position {
            white: 138149888,
            black: 17695533703168,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 17695266840576,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 17661175537664,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 17626815799296,
            white_to_move: false,
        },
        Position {
            white: 69123186752,
            black: 34427502592,
            white_to_move: false,
        },
        Position {
            white: 69122655232,
            black: 34428035072,
            white_to_move: false,
        },
        Position {
            white: 69122656256,
            black: 34428035072,
            white_to_move: false,
        },
        Position {
            white: 69123182592,
            black: 34427510784,
            white_to_move: false,
        },
        Position {
            white: 69222793216,
            black: 34361450496,
            white_to_move: false,
        },
        Position {
            white: 120661737472,
            black: 68820992,
            white_to_move: false,
        },
        Position {
            white: 4501528379392,
            black: 68820992,
            white_to_move: false,
        },
        Position {
            white: 8899574890496,
            black: 68820992,
            white_to_move: false,
        },
        Position {
            white: 68921860160,
            black: 34630795264,
            white_to_move: false,
        },
        Position {
            white: 68921329664,
            black: 34631327744,
            white_to_move: false,
        },
        Position {
            white: 69190815744,
            black: 34361843712,
            white_to_move: false,
        },
        Position {
            white: 69726109696,
            black: 34363416576,
            white_to_move: false,
        },
        Position {
            white: 120460410880,
            black: 272113664,
            white_to_move: false,
        },
        Position {
            white: 8899373563904,
            black: 272113664,
            white_to_move: false,
        },
        Position {
            white: 17695466586112,
            black: 272113664,
            white_to_move: false,
        },
        Position {
            white: 69123186752,
            black: 43017306112,
            white_to_move: false,
        },
        Position {
            white: 69122655232,
            black: 43017838592,
            white_to_move: false,
        },
        Position {
            white: 69122656256,
            black: 43017838592,
            white_to_move: false,
        },
        Position {
            white: 69123182592,
            black: 43017314304,
            white_to_move: false,
        },
        Position {
            white: 69222793216,
            black: 42951254016,
            white_to_move: false,
        },
        Position {
            white: 120661737472,
            black: 8658624512,
            white_to_move: false,
        },
        Position {
            white: 4501528379392,
            black: 8658624512,
            white_to_move: false,
        },
        Position {
            white: 8899574890496,
            black: 8658624512,
            white_to_move: false,
        },
        Position {
            white: 69055546368,
            black: 51674882048,
            white_to_move: false,
        },
        Position {
            white: 69056598016,
            black: 51673833472,
            white_to_move: false,
        },
        Position {
            white: 69189500928,
            black: 51541188608,
            white_to_move: false,
        },
        Position {
            white: 129184563200,
            black: 135798784,
            white_to_move: false,
        },
        Position {
            white: 4518641139712,
            black: 135798784,
            white_to_move: false,
        },
        Position {
            white: 17695600803840,
            black: 17315667968,
            white_to_move: false,
        },
        Position {
            white: 202383424,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 201852928,
            black: 240787660800,
            white_to_move: false,
        },
        Position {
            white: 201854976,
            black: 240787660800,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 240519749632,
            white_to_move: false,
        },
        Position {
            white: 8830654087168,
            black: 206428446720,
            white_to_move: false,
        },
        Position {
            white: 17626747109376,
            black: 206428446720,
            white_to_move: false,
        },
        Position {
            white: 35253292892160,
            black: 172068708352,
            white_to_move: false,
        },
        Position {
            white: 202383424,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 201852928,
            black: 17695534751744,
            white_to_move: false,
        },
        Position {
            white: 201854976,
            black: 17695534751744,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 17695266840576,
            white_to_move: false,
        },
        Position {
            white: 8830654087168,
            black: 17661175537664,
            white_to_move: false,
        },
        Position {
            white: 35253292892160,
            black: 17626815799296,
            white_to_move: false,
        },
        Position {
            white: 9024826001850368,
            black: 68989493248,
            white_to_move: false,
        },
        Position {
            white: 120394358848,
            black: 805830656,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 806363136,
            white_to_move: false,
        },
        Position {
            white: 120528572416,
            black: 671621120,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 806887424,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 672669696,
            white_to_move: false,
        },
        Position {
            white: 120800149504,
            black: 404234240,
            white_to_move: false,
        },
        Position {
            white: 120394358848,
            black: 137707913216,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 137708445696,
            white_to_move: false,
        },
        Position {
            white: 120528572416,
            black: 137573703680,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 137708969984,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 137574752256,
            white_to_move: false,
        },
        Position {
            white: 532575944704,
            black: 404234240,
            white_to_move: false,
        },
        Position {
            white: 103348175872,
            black: 2216338399232,
            white_to_move: false,
        },
        Position {
            white: 103482394624,
            black: 2216204181504,
            white_to_move: false,
        },
        Position {
            white: 103348703232,
            black: 2216337874944,
            white_to_move: false,
        },
        Position {
            white: 103482130432,
            black: 2216204705792,
            white_to_move: false,
        },
        Position {
            white: 103548977152,
            black: 2216204705792,
            white_to_move: false,
        },
        Position {
            white: 129117454336,
            black: 2199159054336,
            white_to_move: false,
        },
        Position {
            white: 86303055936,
            black: 8830453284864,
            white_to_move: false,
        },
        Position {
            white: 86168306688,
            black: 8830588035072,
            white_to_move: false,
        },
        Position {
            white: 86168834048,
            black: 8830587510784,
            white_to_move: false,
        },
        Position {
            white: 86302261248,
            black: 8830454341632,
            white_to_move: false,
        },
        Position {
            white: 86369107968,
            black: 8830454341632,
            white_to_move: false,
        },
        Position {
            white: 4518574030848,
            black: 8796228820992,
            white_to_move: false,
        },
        Position {
            white: 1134782167646208,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 4512481888174080,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 51674882112,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 17661175013376,
            white_to_move: false,
        },
        Position {
            white: 51810140160,
            black: 17661041319936,
            white_to_move: false,
        },
        Position {
            white: 257698037760,
            black: 17592590278656,
            white_to_move: false,
        },
        Position {
            white: 9024842980392960,
            black: 69123710976,
            white_to_move: false,
        },
        Position {
            white: 51943317568,
            black: 35253092089856,
            white_to_move: false,
        },
        Position {
            white: 51808568320,
            black: 35253226840064,
            white_to_move: false,
        },
        Position {
            white: 51942787072,
            black: 35253092622336,
            white_to_move: false,
        },
        Position {
            white: 51809095680,
            black: 35253226315776,
            white_to_move: false,
        },
        Position {
            white: 52009369600,
            black: 35253093146624,
            white_to_move: false,
        },
        Position {
            white: 257966473216,
            black: 35184507887616,
            white_to_move: false,
        },
        Position {
            white: 17712713564160,
            black: 35184507887616,
            white_to_move: false,
        },
        Position {
            white: 4501260470272,
            black: 806363136,
            white_to_move: false,
        },
        Position {
            white: 4501395214336,
            black: 671621120,
            white_to_move: false,
        },
        Position {
            white: 4501260206080,
            black: 806887424,
            white_to_move: false,
        },
        Position {
            white: 4501396258816,
            black: 672669696,
            white_to_move: false,
        },
        Position {
            white: 4501666791424,
            black: 404234240,
            white_to_move: false,
        },
        Position {
            white: 4501260470272,
            black: 137708445696,
            white_to_move: false,
        },
        Position {
            white: 4501395214336,
            black: 137573703680,
            white_to_move: false,
        },
        Position {
            white: 4501260206080,
            black: 137708969984,
            white_to_move: false,
        },
        Position {
            white: 4501396258816,
            black: 137574752256,
            white_to_move: false,
        },
        Position {
            white: 4913442586624,
            black: 404234240,
            white_to_move: false,
        },
        Position {
            white: 4467034948608,
            black: 8830588035072,
            white_to_move: false,
        },
        Position {
            white: 4467035475968,
            black: 8830587510784,
            white_to_move: false,
        },
        Position {
            white: 4467168903168,
            black: 8830454341632,
            white_to_move: false,
        },
        Position {
            white: 4467235749888,
            black: 8830454341632,
            white_to_move: false,
        },
        Position {
            white: 4518574030848,
            black: 8796228820992,
            white_to_move: false,
        },
        Position {
            white: 30855313489920,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 1139163034288128,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 4432540993536,
            black: 17661175013376,
            white_to_move: false,
        },
        Position {
            white: 4432676782080,
            black: 17661041319936,
            white_to_move: false,
        },
        Position {
            white: 4638564679680,
            black: 17592590278656,
            white_to_move: false,
        },
        Position {
            white: 9029223847034880,
            black: 69123710976,
            white_to_move: false,
        },
        Position {
            white: 4432675210240,
            black: 35253226840064,
            white_to_move: false,
        },
        Position {
            white: 4432809428992,
            black: 35253092622336,
            white_to_move: false,
        },
        Position {
            white: 4432675737600,
            black: 35253226315776,
            white_to_move: false,
        },
        Position {
            white: 4432876011520,
            black: 35253093146624,
            white_to_move: false,
        },
        Position {
            white: 4638833115136,
            black: 35184507887616,
            white_to_move: false,
        },
        Position {
            white: 22093580206080,
            black: 35184507887616,
            white_to_move: false,
        },
        Position {
            white: 68988964880,
            black: 34496577544,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 34496057352,
            white_to_move: false,
        },
        Position {
            white: 68991074304,
            black: 34494484488,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 34361839624,
            white_to_move: false,
        },
        Position {
            white: 68995252224,
            black: 34494484488,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 34362363912,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 2625544,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 136843272,
            white_to_move: false,
        },
        Position {
            white: 68988964880,
            black: 34496577568,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 34496057376,
            white_to_move: false,
        },
        Position {
            white: 68991074304,
            black: 34494484512,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 34361839648,
            white_to_move: false,
        },
        Position {
            white: 68995252224,
            black: 34494484512,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 34362363936,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 2625568,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 136843296,
            white_to_move: false,
        },
        Position {
            white: 68988441600,
            black: 34497110016,
            white_to_move: false,
        },
        Position {
            white: 68990038016,
            black: 34495528960,
            white_to_move: false,
        },
        Position {
            white: 69122396160,
            black: 34363416576,
            white_to_move: false,
        },
        Position {
            white: 69189767168,
            black: 34362892288,
            white_to_move: false,
        },
        Position {
            white: 70063755264,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 120527523840,
            black: 137895936,
            white_to_move: false,
        },
        Position {
            white: 4501394165760,
            black: 137895936,
            white_to_move: false,
        },
        Position {
            white: 68855533568,
            black: 35167141888,
            white_to_move: false,
        },
        Position {
            white: 69263691776,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 68788162560,
            black: 35301359616,
            white_to_move: false,
        },
        Position {
            white: 69796368384,
            black: 35299786752,
            white_to_move: false,
        },
        Position {
            white: 120394354688,
            black: 807927808,
            white_to_move: false,
        },
        Position {
            white: 344135307264,
            black: 34765012992,
            white_to_move: false,
        },
        Position {
            white: 1839104,
            black: 240922918912,
            white_to_move: false,
        },
        Position {
            white: 7344128,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 68685824,
            black: 240922918912,
            white_to_move: false,
        },
        Position {
            white: 1076891648,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 17315139584,
            black: 240789225472,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 171935531008,
            white_to_move: false,
        },
        Position {
            white: 270013440,
            black: 35287587618816,
            white_to_move: false,
        },
        Position {
            white: 271601664,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 270274560,
            black: 35287587618816,
            white_to_move: false,
        },
        Position {
            white: 275779584,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 471339008,
            black: 35287453401088,
            white_to_move: false,
        },
        Position {
            white: 1345327104,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 17583575040,
            black: 35287453925376,
            white_to_move: false,
        },
        Position {
            white: 4432675737600,
            black: 35253228404736,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 35218868666368,
            white_to_move: false,
        },
        Position {
            white: 68989227008,
            black: 34563686400,
            white_to_move: false,
        },
        Position {
            white: 68727603200,
            black: 34829500416,
            white_to_move: false,
        },
        Position {
            white: 120326455296,
            black: 406323200,
            white_to_move: false,
        },
        Position {
            white: 68989227008,
            black: 51676446720,
            white_to_move: false,
        },
        Position {
            white: 68727603200,
            black: 51942260736,
            white_to_move: false,
        },
        Position {
            white: 128849281024,
            black: 406323200,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 240789749760,
            white_to_move: false,
        },
        Position {
            white: 135536640,
            black: 240789225472,
            white_to_move: false,
        },
        Position {
            white: 142344192,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 240521838592,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 206430535680,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 172070797312,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 17695536840704,
            white_to_move: false,
        },
        Position {
            white: 135536640,
            black: 17695536316416,
            white_to_move: false,
        },
        Position {
            white: 142344192,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 17695268929536,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 17661177626624,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 17626817888256,
            white_to_move: false,
        },
        Position {
            white: 103213959168,
            black: 2216474705920,
            white_to_move: false,
        },
        Position {
            white: 103348703232,
            black: 2216339963904,
            white_to_move: false,
        },
        Position {
            white: 103349764096,
            black: 2216338915328,
            white_to_move: false,
        },
        Position {
            white: 103213694976,
            black: 2216475230208,
            white_to_move: false,
        },
        Position {
            white: 128849018880,
            black: 2199429578752,
            white_to_move: false,
        },
        Position {
            white: 86168834048,
            black: 4432543088640,
            white_to_move: false,
        },
        Position {
            white: 86034620416,
            black: 4432677306368,
            white_to_move: false,
        },
        Position {
            white: 86033825792,
            black: 4432678354944,
            white_to_move: false,
        },
        Position {
            white: 1130383852699648,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 86168834048,
            black: 8830589599744,
            white_to_move: false,
        },
        Position {
            white: 86034620416,
            black: 8830723817472,
            white_to_move: false,
        },
        Position {
            white: 86033825792,
            black: 8830724866048,
            white_to_move: false,
        },
        Position {
            white: 1134781899210752,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 4512481619738624,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 17661177102336,
            white_to_move: false,
        },
        Position {
            white: 51674882048,
            black: 17661176578048,
            white_to_move: false,
        },
        Position {
            white: 51810156544,
            black: 17661041311744,
            white_to_move: false,
        },
        Position {
            white: 257698037760,
            black: 17592592367616,
            white_to_move: false,
        },
        Position {
            white: 9024842980392960,
            black: 69125799936,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 35253363146752,
            white_to_move: false,
        },
        Position {
            white: 51674882048,
            black: 35253362622464,
            white_to_move: false,
        },
        Position {
            white: 51810156544,
            black: 35253227356160,
            white_to_move: false,
        },
        Position {
            white: 257698037760,
            black: 35184778412032,
            white_to_move: false,
        },
        Position {
            white: 68855006212,
            black: 35165569024,
            white_to_move: false,
        },
        Position {
            white: 68855531520,
            black: 35165045760,
            white_to_move: false,
        },
        Position {
            white: 69396070400,
            black: 34628699136,
            white_to_move: false,
        },
        Position {
            white: 70734053376,
            black: 34360263680,
            white_to_move: false,
        },
        Position {
            white: 120394612736,
            black: 805831680,
            white_to_move: false,
        },
        Position {
            white: 344269783040,
            black: 34628699136,
            white_to_move: false,
        },
        Position {
            white: 8899307765760,
            black: 805831680,
            white_to_move: false,
        },
        Position {
            white: 68719740944,
            black: 35300835328,
            white_to_move: false,
        },
        Position {
            white: 68989227008,
            black: 35031353344,
            white_to_move: false,
        },
        Position {
            white: 68723408896,
            black: 35299264512,
            white_to_move: false,
        },
        Position {
            white: 69260804096,
            black: 34763966464,
            white_to_move: false,
        },
        Position {
            white: 120259346432,
            black: 941099008,
            white_to_move: false,
        },
        Position {
            white: 69123969040,
            black: 34896609280,
            white_to_move: false,
        },
        Position {
            white: 68854485024,
            black: 35166093312,
            white_to_move: false,
        },
        Position {
            white: 68855537664,
            black: 35165048832,
            white_to_move: false,
        },
        Position {
            white: 68857626624,
            black: 35165048832,
            white_to_move: false,
        },
        Position {
            white: 69395546112,
            black: 34629226496,
            white_to_move: false,
        },
        Position {
            white: 70733529088,
            black: 34360791040,
            white_to_move: false,
        },
        Position {
            white: 120394088448,
            black: 806359040,
            white_to_move: false,
        },
        Position {
            white: 206561869824,
            black: 34897661952,
            white_to_move: false,
        },
        Position {
            white: 8899307241472,
            black: 806359040,
            white_to_move: false,
        },
        Position {
            white: 2015100928,
            black: 240518168576,
            white_to_move: false,
        },
        Position {
            white: 275550830592,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 8830588813312,
            black: 206963736576,
            white_to_move: false,
        },
        Position {
            white: 17661310009344,
            black: 172335562752,
            white_to_move: false,
        },
        Position {
            white: 35253227618304,
            black: 172603998208,
            white_to_move: false,
        },
        Position {
            white: 70506587619328,
            black: 103616086016,
            white_to_move: false,
        },
        Position {
            white: 2015100928,
            black: 8899172237312,
            white_to_move: false,
        },
        Position {
            white: 137843441664,
            black: 8899709108224,
            white_to_move: false,
        },
        Position {
            white: 275550830592,
            black: 8899440672768,
            white_to_move: false,
        },
        Position {
            white: 17661310009344,
            black: 8830989631488,
            white_to_move: false,
        },
        Position {
            white: 35253227618304,
            black: 8831258066944,
            white_to_move: false,
        },
        Position {
            white: 2260630402498560,
            black: 69524783104,
            white_to_move: false,
        },
        Position {
            white: 2015100928,
            black: 17695265259520,
            white_to_move: false,
        },
        Position {
            white: 137843441664,
            black: 17695802130432,
            white_to_move: false,
        },
        Position {
            white: 275550830592,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 8830588813312,
            black: 17661710827520,
            white_to_move: false,
        },
        Position {
            white: 35253227618304,
            black: 17627351089152,
            white_to_move: false,
        },
        Position {
            white: 4521260937379840,
            black: 34896609280,
            white_to_move: false,
        },
        Position {
            white: 69530030096,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 69261592576,
            black: 34762919936,
            white_to_move: false,
        },
        Position {
            white: 69395021824,
            black: 34629750784,
            white_to_move: false,
        },
        Position {
            white: 69730304000,
            black: 34361315328,
            white_to_move: false,
        },
        Position {
            white: 120800149504,
            black: 404230144,
            white_to_move: false,
        },
        Position {
            white: 69260550160,
            black: 34763964416,
            white_to_move: false,
        },
        Position {
            white: 69261592576,
            black: 34762924032,
            white_to_move: false,
        },
        Position {
            white: 69530030080,
            black: 34494488576,
            white_to_move: false,
        },
        Position {
            white: 69395021824,
            black: 34629754880,
            white_to_move: false,
        },
        Position {
            white: 69730304000,
            black: 34361319424,
            white_to_move: false,
        },
        Position {
            white: 120800149504,
            black: 404234240,
            white_to_move: false,
        },
        Position {
            white: 69261592576,
            black: 34765012992,
            white_to_move: false,
        },
        Position {
            white: 69530030080,
            black: 34496577536,
            white_to_move: false,
        },
        Position {
            white: 69262647296,
            black: 34763964416,
            white_to_move: false,
        },
        Position {
            white: 69398691840,
            black: 34628173824,
            white_to_move: false,
        },
        Position {
            white: 69730304000,
            black: 34363408384,
            white_to_move: false,
        },
        Position {
            white: 120800149504,
            black: 406323200,
            white_to_move: false,
        },
        Position {
            white: 68859723776,
            black: 36238786560,
            white_to_move: false,
        },
        Position {
            white: 120398544896,
            black: 1879572480,
            white_to_move: false,
        },
        Position {
            white: 345213239296,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 542900224,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 1011875840,
            black: 240518692864,
            white_to_move: false,
        },
        Position {
            white: 17856200704,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 8865354612736,
            black: 172201869312,
            white_to_move: false,
        },
        Position {
            white: 17661716070400,
            black: 171933433856,
            white_to_move: false,
        },
        Position {
            white: 35322353156096,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 542900224,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 1011875840,
            black: 17695265783808,
            white_to_move: false,
        },
        Position {
            white: 17856200704,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 8865354612736,
            black: 17626948960256,
            white_to_move: false,
        },
        Position {
            white: 4521261343440896,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 542900224,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 1011875840,
            black: 35287451828224,
            white_to_move: false,
        },
        Position {
            white: 17856200704,
            black: 35287720263680,
            white_to_move: false,
        },
        Position {
            white: 8865354612736,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 17661716070400,
            black: 35218866569216,
            white_to_move: false,
        },
        Position {
            white: 120393828360,
            black: 806354944,
            white_to_move: false,
        },
        Position {
            white: 120662790144,
            black: 537397248,
            white_to_move: false,
        },
        Position {
            white: 120394358784,
            black: 805832704,
            white_to_move: false,
        },
        Position {
            white: 120663834624,
            black: 538445824,
            white_to_move: false,
        },
        Position {
            white: 120934367232,
            black: 270010368,
            white_to_move: false,
        },
        Position {
            white: 122272350208,
            black: 1574912,
            white_to_move: false,
        },
        Position {
            white: 120662790160,
            black: 537395200,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 806359040,
            white_to_move: false,
        },
        Position {
            white: 120394358784,
            black: 805834752,
            white_to_move: false,
        },
        Position {
            white: 120663834624,
            black: 538447872,
            white_to_move: false,
        },
        Position {
            white: 120934367232,
            black: 270012416,
            white_to_move: false,
        },
        Position {
            white: 122272350208,
            black: 1576960,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 808452096,
            white_to_move: false,
        },
        Position {
            white: 120662790144,
            black: 539492352,
            white_to_move: false,
        },
        Position {
            white: 120394358784,
            black: 807927808,
            white_to_move: false,
        },
        Position {
            white: 120663851008,
            black: 538443776,
            white_to_move: false,
        },
        Position {
            white: 120934367232,
            black: 272105472,
            white_to_move: false,
        },
        Position {
            white: 122272350208,
            black: 3670016,
            white_to_move: false,
        },
        Position {
            white: 120394876928,
            black: 872415232,
            white_to_move: false,
        },
        Position {
            white: 120327372800,
            black: 940048384,
            white_to_move: false,
        },
        Position {
            white: 120462245888,
            black: 805306368,
            white_to_move: false,
        },
        Position {
            white: 120530665472,
            black: 738721792,
            white_to_move: false,
        },
        Position {
            white: 120801198080,
            black: 470286336,
            white_to_move: false,
        },
        Position {
            white: 395674910720,
            black: 470286336,
            white_to_move: false,
        },
        Position {
            white: 86035138560,
            black: 4433211555840,
            white_to_move: false,
        },
        Position {
            white: 86035398656,
            black: 4433211555840,
            white_to_move: false,
        },
        Position {
            white: 86575677440,
            black: 4432675209216,
            white_to_move: false,
        },
        Position {
            white: 87913660416,
            black: 4432406773760,
            white_to_move: false,
        },
        Position {
            white: 361449390080,
            black: 4432675209216,
            white_to_move: false,
        },
        Position {
            white: 8916487372800,
            black: 4398852341760,
            white_to_move: false,
        },
        Position {
            white: 1130383987965952,
            black: 35165569024,
            white_to_move: false,
        },
        Position {
            white: 17181704192,
            black: 8900111761408,
            white_to_move: false,
        },
        Position {
            white: 257699086336,
            black: 8797033070592,
            white_to_move: false,
        },
        Position {
            white: 292595695616,
            black: 8899575414784,
            white_to_move: false,
        },
        Position {
            white: 17678354874368,
            black: 8831124373504,
            white_to_move: false,
        },
        Position {
            white: 4512412901310464,
            black: 104019263488,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 17661710827520,
            white_to_move: false,
        },
        Position {
            white: 51675660288,
            black: 17661710827520,
            white_to_move: false,
        },
        Position {
            white: 51945406464,
            black: 17661442916352,
            white_to_move: false,
        },
        Position {
            white: 53553922048,
            black: 17660906045440,
            white_to_move: false,
        },
        Position {
            white: 257833304064,
            black: 17592991875072,
            white_to_move: false,
        },
        Position {
            white: 327089651712,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 35304766439424,
            black: 17592991875072,
            white_to_move: false,
        },
        Position {
            white: 4521312476200960,
            black: 537395200,
            white_to_move: false,
        },
        Position {
            white: 9024843115659264,
            black: 69525307392,
            white_to_move: false,
        },
        Position {
            white: 344403742736,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 344135305216,
            black: 34762919936,
            white_to_move: false,
        },
        Position {
            white: 344268734464,
            black: 34629750784,
            white_to_move: false,
        },
        Position {
            white: 344604016640,
            black: 34361315328,
            white_to_move: false,
        },
        Position {
            white: 395673862144,
            black: 404230144,
            white_to_move: false,
        },
        Position {
            white: 344135305216,
            black: 34762924032,
            white_to_move: false,
        },
        Position {
            white: 344403742720,
            black: 34494488576,
            white_to_move: false,
        },
        Position {
            white: 344268734464,
            black: 34629754880,
            white_to_move: false,
        },
        Position {
            white: 344604016640,
            black: 34361319424,
            white_to_move: false,
        },
        Position {
            white: 395673862144,
            black: 404234240,
            white_to_move: false,
        },
        Position {
            white: 344135305216,
            black: 34765012992,
            white_to_move: false,
        },
        Position {
            white: 344403742720,
            black: 34496577536,
            white_to_move: false,
        },
        Position {
            white: 344136359936,
            black: 34763964416,
            white_to_move: false,
        },
        Position {
            white: 344268734464,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 344604016640,
            black: 34363408384,
            white_to_move: false,
        },
        Position {
            white: 395673862144,
            black: 406323200,
            white_to_move: false,
        },
        Position {
            white: 343733436416,
            black: 36238786560,
            white_to_move: false,
        },
        Position {
            white: 345213239296,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 395272257536,
            black: 1879572480,
            white_to_move: false,
        },
        Position {
            white: 275416612864,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 275885588480,
            black: 240518692864,
            white_to_move: false,
        },
        Position {
            white: 533248081920,
            black: 268959744,
            white_to_move: false,
        },
        Position {
            white: 9140228325376,
            black: 172201869312,
            white_to_move: false,
        },
        Position {
            white: 17936589783040,
            black: 171933433856,
            white_to_move: false,
        },
        Position {
            white: 35597226868736,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 275416612864,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 275885588480,
            black: 17695265783808,
            white_to_move: false,
        },
        Position {
            white: 292729913344,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 9140228325376,
            black: 17626948960256,
            white_to_move: false,
        },
        Position {
            white: 4521536217153536,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 275416612864,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 275885588480,
            black: 35287451828224,
            white_to_move: false,
        },
        Position {
            white: 292729913344,
            black: 35287720263680,
            white_to_move: false,
        },
        Position {
            white: 9140228325376,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 17936589783040,
            black: 35218866569216,
            white_to_move: false,
        },
        Position {
            white: 4539059415285760,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 1311748,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 17315397632,
            black: 240787129344,
            white_to_move: false,
        },
        Position {
            white: 17661175267328,
            black: 171933434880,
            white_to_move: false,
        },
        Position {
            white: 35253227094016,
            black: 172067652608,
            white_to_move: false,
        },
        Position {
            white: 790560,
            black: 240921870336,
            white_to_move: false,
        },
        Position {
            white: 3932160,
            black: 240920825856,
            white_to_move: false,
        },
        Position {
            white: 8830587764736,
            black: 206427918336,
            white_to_move: false,
        },
        Position {
            white: 35253226569728,
            black: 172068179968,
            white_to_move: false,
        },
        Position {
            white: 70506452353024,
            black: 103214485504,
            white_to_move: false,
        },
        Position {
            white: 3932160,
            black: 240920829952,
            white_to_move: false,
        },
        Position {
            white: 8830587764736,
            black: 206427922432,
            white_to_move: false,
        },
        Position {
            white: 35253226569728,
            black: 172068184064,
            white_to_move: false,
        },
        Position {
            white: 70506452353024,
            black: 103214489600,
            white_to_move: false,
        },
        Position {
            white: 17314613248,
            black: 240787656704,
            white_to_move: false,
        },
        Position {
            white: 17315143680,
            black: 240787132416,
            white_to_move: false,
        },
        Position {
            white: 18119393280,
            black: 240519745536,
            white_to_move: false,
        },
        Position {
            white: 532710162432,
            black: 270012416,
            white_to_move: false,
        },
        Position {
            white: 8847766847488,
            black: 206428442624,
            white_to_move: false,
        },
        Position {
            white: 35270405652480,
            black: 172068704256,
            white_to_move: false,
        },
        Position {
            white: 17181835264,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 532576993280,
            black: 403439616,
            white_to_move: false,
        },
        Position {
            white: 17678354874368,
            black: 171933696000,
            white_to_move: false,
        },
        Position {
            white: 17314613248,
            black: 240789749760,
            white_to_move: false,
        },
        Position {
            white: 17315143680,
            black: 240789225472,
            white_to_move: false,
        },
        Position {
            white: 18119393280,
            black: 240521838592,
            white_to_move: false,
        },
        Position {
            white: 532710162432,
            black: 272105472,
            white_to_move: false,
        },
        Position {
            white: 8847766847488,
            black: 206430535680,
            white_to_move: false,
        },
        Position {
            white: 35270405652480,
            black: 172070797312,
            white_to_move: false,
        },
        Position {
            white: 17248813056,
            black: 240920821760,
            white_to_move: false,
        },
        Position {
            white: 532576993280,
            black: 470286336,
            white_to_move: false,
        },
        Position {
            white: 17678354874368,
            black: 172000542720,
            white_to_move: false,
        },
        Position {
            white: 135792640,
            black: 266556407808,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 266556407808,
            white_to_move: false,
        },
        Position {
            white: 940572672,
            black: 266288496640,
            white_to_move: false,
        },
        Position {
            white: 2216338391040,
            black: 249377062912,
            white_to_move: false,
        },
        Position {
            white: 8830588026880,
            black: 232197193728,
            white_to_move: false,
        },
        Position {
            white: 17661309222912,
            black: 197569019904,
            white_to_move: false,
        },
        Position {
            white: 35253226831872,
            black: 197837455360,
            white_to_move: false,
        },
        Position {
            white: 17661174481920,
            black: 171933966336,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 171933442048,
            white_to_move: false,
        },
        Position {
            white: 17661308436480,
            black: 171800272896,
            white_to_move: false,
        },
        Position {
            white: 17695735021568,
            black: 137440534528,
            white_to_move: false,
        },
        Position {
            white: 17799686651904,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 17712713564160,
            black: 137574752256,
            white_to_move: false,
        },
        Position {
            white: 18073490817024,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 22093580206080,
            black: 137574752256,
            white_to_move: false,
        },
        Position {
            white: 88167357087744,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 171935531008,
            white_to_move: false,
        },
        Position {
            white: 17661040001024,
            black: 172070797312,
            white_to_move: false,
        },
        Position {
            white: 17695332368384,
            black: 137845276672,
            white_to_move: false,
        },
        Position {
            white: 17799418216448,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 17712445128704,
            black: 137845276672,
            white_to_move: false,
        },
        Position {
            white: 18073222381568,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 17661041573888,
            black: 172603998208,
            white_to_move: false,
        },
        Position {
            white: 17661447634944,
            black: 172201869312,
            white_to_move: false,
        },
        Position {
            white: 17695333416960,
            black: 138379001856,
            white_to_move: false,
        },
        Position {
            white: 17799419265024,
            black: 35299786752,
            white_to_move: false,
        },
        Position {
            white: 17712580395008,
            black: 138244784128,
            white_to_move: false,
        },
        Position {
            white: 18073760301056,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 17592456053760,
            black: 35425024475136,
            white_to_move: false,
        },
        Position {
            white: 17592456314880,
            black: 35425024475136,
            white_to_move: false,
        },
        Position {
            white: 17627016593408,
            black: 35390531043328,
            white_to_move: false,
        },
        Position {
            white: 17730968223744,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 17609769615360,
            black: 35424890781696,
            white_to_move: false,
        },
        Position {
            white: 22024861777920,
            black: 35390665261056,
            white_to_move: false,
        },
        Position {
            white: 123283010748416,
            black: 103213957120,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 2269563932639232,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 2269563798421504,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 2269563798945792,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 2269529439207424,
            white_to_move: false,
        },
        Position {
            white: 481305821184,
            black: 2269426494210048,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 2269529573425152,
            white_to_move: false,
        },
        Position {
            white: 70575172091904,
            black: 2269426494210048,
            white_to_move: false,
        },
        Position {
            white: 4521260802375680,
            black: 2251971747119104,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 9024963373694976,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 9024963239477248,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 9024963240001536,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 9024928880263168,
            white_to_move: false,
        },
        Position {
            white: 481305821184,
            black: 9024825935265792,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 9024929014480896,
            white_to_move: false,
        },
        Position {
            white: 70575172091904,
            black: 9024825935265792,
            white_to_move: false,
        },
        Position {
            white: 4521260802375680,
            black: 9007371188174848,
            white_to_move: false,
        },
        Position {
            white: 269487108,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 269491200,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 270271488,
            black: 35287585523712,
            white_to_move: false,
        },
        Position {
            white: 470811648,
            black: 35287451830272,
            white_to_move: false,
        },
        Position {
            white: 17583571968,
            black: 35287451830272,
            white_to_move: false,
        },
        Position {
            white: 4432675734528,
            black: 35253226309632,
            white_to_move: false,
        },
        Position {
            white: 17661175006208,
            black: 35218866571264,
            white_to_move: false,
        },
        Position {
            white: 529440,
            black: 35287855005696,
            white_to_move: false,
        },
        Position {
            white: 3671040,
            black: 35287853961216,
            white_to_move: false,
        },
        Position {
            white: 137707914240,
            black: 35287586574336,
            white_to_move: false,
        },
        Position {
            white: 8830587503616,
            black: 35253361053696,
            white_to_move: false,
        },
        Position {
            white: 7865344,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 17315660800,
            black: 35287721836544,
            white_to_move: false,
        },
        Position {
            white: 137708962816,
            black: 35287587618816,
            white_to_move: false,
        },
        Position {
            white: 8830588552192,
            black: 35253362098176,
            white_to_move: false,
        },
        Position {
            white: 17661175530496,
            black: 35218868142080,
            white_to_move: false,
        },
        Position {
            white: 17315660800,
            black: 35288256610304,
            white_to_move: false,
        },
        Position {
            white: 137708962816,
            black: 35288122392576,
            white_to_move: false,
        },
        Position {
            white: 275416351744,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 8830588552192,
            black: 35253896871936,
            white_to_move: false,
        },
        Position {
            white: 17661175530496,
            black: 35219402915840,
            white_to_move: false,
        },
        Position {
            white: 270401536,
            black: 35287585522176,
            white_to_move: false,
        },
        Position {
            white: 471334912,
            black: 35287451566592,
            white_to_move: false,
        },
        Position {
            white: 17584095232,
            black: 35287451566592,
            white_to_move: false,
        },
        Position {
            white: 4432676257792,
            black: 35253226045952,
            white_to_move: false,
        },
        Position {
            white: 8830856986624,
            black: 35253091828224,
            white_to_move: false,
        },
        Position {
            white: 17661175529472,
            black: 35218866307584,
            white_to_move: false,
        },
        Position {
            white: 269748228,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 269748240,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 270271488,
            black: 35287585523712,
            white_to_move: false,
        },
        Position {
            white: 471072768,
            black: 35287451830272,
            white_to_move: false,
        },
        Position {
            white: 17583833088,
            black: 35287451830272,
            white_to_move: false,
        },
        Position {
            white: 4432675995648,
            black: 35253226309632,
            white_to_move: false,
        },
        Position {
            white: 17661175267328,
            black: 35218866571264,
            white_to_move: false,
        },
        Position {
            white: 18049652005011456,
            black: 34360264704,
            white_to_move: false,
        },
        Position {
            white: 790560,
            black: 35287855005696,
            white_to_move: false,
        },
        Position {
            white: 3932160,
            black: 35287853961216,
            white_to_move: false,
        },
        Position {
            white: 137708175360,
            black: 35287586574336,
            white_to_move: false,
        },
        Position {
            white: 8830587764736,
            black: 35253361053696,
            white_to_move: false,
        },
        Position {
            white: 18049651736051712,
            black: 34629226496,
            white_to_move: false,
        },
        Position {
            white: 270274560,
            black: 35287585529856,
            white_to_move: false,
        },
        Position {
            white: 272367616,
            black: 35287585529856,
            white_to_move: false,
        },
        Position {
            white: 470548480,
            black: 35287452360704,
            white_to_move: false,
        },
        Position {
            white: 4432675471360,
            black: 35253226840064,
            white_to_move: false,
        },
        Position {
            white: 8830856200192,
            black: 35253092622336,
            white_to_move: false,
        },
        Position {
            white: 17661174743040,
            black: 35218867101696,
            white_to_move: false,
        },
        Position {
            white: 18049652004487168,
            black: 34360795136,
            white_to_move: false,
        },
        Position {
            white: 8126464,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 35287721836544,
            white_to_move: false,
        },
        Position {
            white: 137709223936,
            black: 35287587618816,
            white_to_move: false,
        },
        Position {
            white: 8830588813312,
            black: 35253362098176,
            white_to_move: false,
        },
        Position {
            white: 17661175791616,
            black: 35218868142080,
            white_to_move: false,
        },
        Position {
            white: 18049651737100288,
            black: 34630270976,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 35288256610304,
            white_to_move: false,
        },
        Position {
            white: 137709223936,
            black: 35288122392576,
            white_to_move: false,
        },
        Position {
            white: 275416612864,
            black: 35287853957120,
            white_to_move: false,
        },
        Position {
            white: 8830588813312,
            black: 35253896871936,
            white_to_move: false,
        },
        Position {
            white: 17661175791616,
            black: 35219402915840,
            white_to_move: false,
        },
        Position {
            white: 18049651737100288,
            black: 35165044736,
            white_to_move: false,
        },
        Position {
            white: 201855008,
            black: 35287720787968,
            white_to_move: false,
        },
        Position {
            white: 201852928,
            black: 35287720792064,
            white_to_move: false,
        },
        Position {
            white: 202383360,
            black: 35287720267776,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 35287452880896,
            white_to_move: false,
        },
        Position {
            white: 8830654087168,
            black: 35253361577984,
            white_to_move: false,
        },
        Position {
            white: 17626747109376,
            black: 35253361577984,
            white_to_move: false,
        },
        Position {
            white: 18049651802374144,
            black: 34629750784,
            white_to_move: false,
        },
        Position {
            white: 404227072,
            black: 35287518543872,
            white_to_move: false,
        },
        Position {
            white: 404228096,
            black: 35287518543872,
            white_to_move: false,
        },
        Position {
            white: 404488192,
            black: 35287518543872,
            white_to_move: false,
        },
        Position {
            white: 504365056,
            black: 35287451959296,
            white_to_move: false,
        },
        Position {
            white: 4432809951232,
            black: 35253159329792,
            white_to_move: false,
        },
        Position {
            white: 8830856462336,
            black: 35253159329792,
            white_to_move: false,
        },
        Position {
            white: 17661309222912,
            black: 35218799591424,
            white_to_move: false,
        },
        Position {
            white: 18049652004749312,
            black: 34427502592,
            white_to_move: false,
        },
        Position {
            white: 337380352,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 337121280,
            black: 35287585783808,
            white_to_move: false,
        },
        Position {
            white: 337510400,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 17650679808,
            black: 35287452090368,
            white_to_move: false,
        },
        Position {
            white: 4432742842368,
            black: 35253226569728,
            white_to_move: false,
        },
        Position {
            white: 17695601852416,
            black: 35184507092992,
            white_to_move: false,
        },
        Position {
            white: 201852928,
            black: 35287722885120,
            white_to_move: false,
        },
        Position {
            white: 201854976,
            black: 35287722885120,
            white_to_move: false,
        },
        Position {
            white: 202383360,
            black: 35287722360832,
            white_to_move: false,
        },
        Position {
            white: 1006632960,
            black: 35287454973952,
            white_to_move: false,
        },
        Position {
            white: 8830654087168,
            black: 35253363671040,
            white_to_move: false,
        },
        Position {
            white: 17626747109376,
            black: 35253363671040,
            white_to_move: false,
        },
        Position {
            white: 18049651802374144,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 404227072,
            black: 35296108347392,
            white_to_move: false,
        },
        Position {
            white: 404228096,
            black: 35296108347392,
            white_to_move: false,
        },
        Position {
            white: 404488192,
            black: 35296108347392,
            white_to_move: false,
        },
        Position {
            white: 504365056,
            black: 35296041762816,
            white_to_move: false,
        },
        Position {
            white: 4432809951232,
            black: 35261749133312,
            white_to_move: false,
        },
        Position {
            white: 8830856462336,
            black: 35261749133312,
            white_to_move: false,
        },
        Position {
            white: 17661309222912,
            black: 35227389394944,
            white_to_move: false,
        },
        Position {
            white: 18049652004749312,
            black: 43017306112,
            white_to_move: false,
        },
        Position {
            white: 202901504,
            black: 35425158692864,
            white_to_move: false,
        },
        Position {
            white: 202903552,
            black: 35425158692864,
            white_to_move: false,
        },
        Position {
            white: 203161600,
            black: 35425158692864,
            white_to_move: false,
        },
        Position {
            white: 1007681536,
            black: 35424890781696,
            white_to_move: false,
        },
        Position {
            white: 8830655135744,
            black: 35390799478784,
            white_to_move: false,
        },
        Position {
            white: 17695736070144,
            black: 35321811566592,
            white_to_move: false,
        },
        Position {
            white: 18049651803422720,
            black: 172067651584,
            white_to_move: false,
        },
        Position {
            white: 17314613248,
            black: 35287720792064,
            white_to_move: false,
        },
        Position {
            white: 17315143680,
            black: 35287720267776,
            white_to_move: false,
        },
        Position {
            white: 18119393280,
            black: 35287452880896,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 35184642101248,
            white_to_move: false,
        },
        Position {
            white: 8847766847488,
            black: 35253361577984,
            white_to_move: false,
        },
        Position {
            white: 18049668915134464,
            black: 34629750784,
            white_to_move: false,
        },
        Position {
            white: 17449878528,
            black: 35287585783808,
            white_to_move: false,
        },
        Position {
            white: 17450270720,
            black: 35287585521664,
            white_to_move: false,
        },
        Position {
            white: 17650679808,
            black: 35287452090368,
            white_to_move: false,
        },
        Position {
            white: 257967521792,
            black: 35184507092992,
            white_to_move: false,
        },
        Position {
            white: 4449855602688,
            black: 35253226569728,
            white_to_move: false,
        },
        Position {
            white: 17678354874368,
            black: 35218866831360,
            white_to_move: false,
        },
        Position {
            white: 17314613248,
            black: 35287722885120,
            white_to_move: false,
        },
        Position {
            white: 17315143680,
            black: 35287722360832,
            white_to_move: false,
        },
        Position {
            white: 18119393280,
            black: 35287454973952,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 35184644194304,
            white_to_move: false,
        },
        Position {
            white: 8847766847488,
            black: 35253363671040,
            white_to_move: false,
        },
        Position {
            white: 18049668915134464,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 404227072,
            black: 35313221107712,
            white_to_move: false,
        },
        Position {
            white: 404228096,
            black: 35313221107712,
            white_to_move: false,
        },
        Position {
            white: 404488192,
            black: 35313221107712,
            white_to_move: false,
        },
        Position {
            white: 2216606826496,
            black: 35296041762816,
            white_to_move: false,
        },
        Position {
            white: 4432809951232,
            black: 35278861893632,
            white_to_move: false,
        },
        Position {
            white: 8830856462336,
            black: 35278861893632,
            white_to_move: false,
        },
        Position {
            white: 17661309222912,
            black: 35244502155264,
            white_to_move: false,
        },
        Position {
            white: 18049652004749312,
            black: 60130066432,
            white_to_move: false,
        },
        Position {
            white: 17315661824,
            black: 35425158692864,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 35425158692864,
            white_to_move: false,
        },
        Position {
            white: 18120441856,
            black: 35424890781696,
            white_to_move: false,
        },
        Position {
            white: 532711211008,
            black: 35184641048576,
            white_to_move: false,
        },
        Position {
            white: 8847767896064,
            black: 35390799478784,
            white_to_move: false,
        },
        Position {
            white: 17678489092096,
            black: 35356171304960,
            white_to_move: false,
        },
        Position {
            white: 18049668916183040,
            black: 172067651584,
            white_to_move: false,
        },
        Position {
            white: 4432540993536,
            black: 35253361053696,
            white_to_move: false,
        },
        Position {
            white: 4432676782080,
            black: 35253227360256,
            white_to_move: false,
        },
        Position {
            white: 4638564679680,
            black: 35184776318976,
            white_to_move: false,
        },
        Position {
            white: 4432675210240,
            black: 35253228929024,
            white_to_move: false,
        },
        Position {
            white: 4432809428992,
            black: 35253094711296,
            white_to_move: false,
        },
        Position {
            white: 4432675737600,
            black: 35253228404736,
            white_to_move: false,
        },
        Position {
            white: 4432676798464,
            black: 35253227356160,
            white_to_move: false,
        },
        Position {
            white: 4432876011520,
            black: 35253095235584,
            white_to_move: false,
        },
        Position {
            white: 4638833115136,
            black: 35184509976576,
            white_to_move: false,
        },
        Position {
            white: 22093580206080,
            black: 35184509976576,
            white_to_move: false,
        },
        Position {
            white: 4432542042112,
            black: 35253896871936,
            white_to_move: false,
        },
        Position {
            white: 4432408084480,
            black: 35254031089664,
            white_to_move: false,
        },
        Position {
            white: 4432677830656,
            black: 35253763178496,
            white_to_move: false,
        },
        Position {
            white: 4449721384960,
            black: 35253897396224,
            white_to_move: false,
        },
        Position {
            white: 4638565728256,
            black: 35185312137216,
            white_to_move: false,
        },
        Position {
            white: 4707822075904,
            black: 35253494743040,
            white_to_move: false,
        },
        Position {
            white: 22093581254656,
            black: 35185043701760,
            white_to_move: false,
        },
        Position {
            white: 4398316520448,
            black: 35304765390848,
            white_to_move: false,
        },
        Position {
            white: 4398316781568,
            black: 35304765390848,
            white_to_move: false,
        },
        Position {
            white: 4415697190912,
            black: 35287451828224,
            white_to_move: false,
        },
        Position {
            white: 6614653337600,
            black: 35287451828224,
            white_to_move: false,
        },
        Position {
            white: 22059221516288,
            black: 35236046438400,
            white_to_move: false,
        },
        Position {
            white: 4432542042112,
            black: 35390798954496,
            white_to_move: false,
        },
        Position {
            white: 4432408084480,
            black: 35390933172224,
            white_to_move: false,
        },
        Position {
            white: 4432677830656,
            black: 35390665261056,
            white_to_move: false,
        },
        Position {
            white: 4449721384960,
            black: 35390799478784,
            white_to_move: false,
        },
        Position {
            white: 4913443635200,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 22093581254656,
            black: 35321945784320,
            white_to_move: false,
        },
        Position {
            white: 4398316520448,
            black: 44083678543872,
            white_to_move: false,
        },
        Position {
            white: 4398316781568,
            black: 44083678543872,
            white_to_move: false,
        },
        Position {
            white: 4398517321728,
            black: 44083544850432,
            white_to_move: false,
        },
        Position {
            white: 4415630082048,
            black: 44083544850432,
            white_to_move: false,
        },
        Position {
            white: 30855314538496,
            black: 35218866569216,
            white_to_move: false,
        },
        Position {
            white: 17661174481920,
            black: 35218867101696,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 35218866577408,
            white_to_move: false,
        },
        Position {
            white: 17661308436480,
            black: 35218733408256,
            white_to_move: false,
        },
        Position {
            white: 17695735021568,
            black: 35184373669888,
            white_to_move: false,
        },
        Position {
            white: 17712713564160,
            black: 35184507887616,
            white_to_move: false,
        },
        Position {
            white: 22093580206080,
            black: 35184507887616,
            white_to_move: false,
        },
        Position {
            white: 123214290223104,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 18067244055527424,
            black: 34495537152,
            white_to_move: false,
        },
        Position {
            white: 17661175009280,
            black: 35218868666368,
            white_to_move: false,
        },
        Position {
            white: 17661040001024,
            black: 35219003932672,
            white_to_move: false,
        },
        Position {
            white: 17695332368384,
            black: 35184778412032,
            white_to_move: false,
        },
        Position {
            white: 17712445128704,
            black: 35184778412032,
            white_to_move: false,
        },
        Position {
            white: 123214021787648,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 18067243787091968,
            black: 34766061568,
            white_to_move: false,
        },
        Position {
            white: 17661041573888,
            black: 35219537133568,
            white_to_move: false,
        },
        Position {
            white: 17661447634944,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 17695333416960,
            black: 35185312137216,
            white_to_move: false,
        },
        Position {
            white: 17712580395008,
            black: 35185177919488,
            white_to_move: false,
        },
        Position {
            white: 17936321347584,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 123214022836224,
            black: 35299786752,
            white_to_move: false,
        },
        Position {
            white: 18067243788140544,
            black: 35299786752,
            white_to_move: false,
        },
        Position {
            white: 17592187879424,
            black: 35425292910592,
            white_to_move: false,
        },
        Position {
            white: 17626613940224,
            black: 35390933696512,
            white_to_move: false,
        },
        Position {
            white: 17730699788288,
            black: 35287854481408,
            white_to_move: false,
        },
        Position {
            white: 17609501179904,
            black: 35425159217152,
            white_to_move: false,
        },
        Position {
            white: 123145303359488,
            black: 240921346048,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 61607145111552,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 61607010893824,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 61607011418112,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 61572651679744,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 61572785897472,
            white_to_move: false,
        },
        Position {
            white: 1134764988825600,
            black: 52811052613632,
            white_to_move: false,
        },
        Position {
            white: 4521260802375680,
            black: 44014959591424,
            white_to_move: false,
        },
        Position {
            white: 18049651870531584,
            black: 26422773547008,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 9060010306830336,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 9060010172612608,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 9060010173136896,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 9059975813398528,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 9059975947616256,
            white_to_move: false,
        },
        Position {
            white: 4521260802375680,
            black: 9042418121310208,
            white_to_move: false,
        },
        Position {
            white: 18049651870531584,
            black: 9024825935265792,
            white_to_move: false,
        },
        Position {
            white: 103213436416,
            black: 2216472084480,
            white_to_move: false,
        },
        Position {
            white: 103482918912,
            black: 2216203650048,
            white_to_move: false,
        },
        Position {
            white: 103483967488,
            black: 2216203650048,
            white_to_move: false,
        },
        Position {
            white: 104018741248,
            black: 2216203650048,
            white_to_move: false,
        },
        Position {
            white: 128983238656,
            black: 2199292216320,
            white_to_move: false,
        },
        Position {
            white: 283794393270272,
            black: 268960768,
            white_to_move: false,
        },
        Position {
            white: 103349229568,
            black: 2216337342464,
            white_to_move: false,
        },
        Position {
            white: 103214221312,
            black: 2216472608768,
            white_to_move: false,
        },
        Position {
            white: 103351322624,
            black: 2216337342464,
            white_to_move: false,
        },
        Position {
            white: 103617660928,
            black: 2216605777920,
            white_to_move: false,
        },
        Position {
            white: 128849545216,
            black: 2199426957312,
            white_to_move: false,
        },
        Position {
            white: 240787130368,
            black: 2216338391040,
            white_to_move: false,
        },
        Position {
            white: 103146981376,
            black: 2216605777920,
            white_to_move: false,
        },
        Position {
            white: 103214221312,
            black: 2216538669056,
            white_to_move: false,
        },
        Position {
            white: 103349225472,
            black: 2216404451328,
            white_to_move: false,
        },
        Position {
            white: 103350274048,
            black: 2216404451328,
            white_to_move: false,
        },
        Position {
            white: 128916654080,
            black: 2199425908736,
            white_to_move: false,
        },
        Position {
            white: 240787130368,
            black: 2216404451328,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 2456721293312,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 2422629990400,
            white_to_move: false,
        },
        Position {
            white: 35253226309632,
            black: 2388270252032,
            white_to_move: false,
        },
        Position {
            white: 70506586310656,
            black: 2319282339840,
            white_to_move: false,
        },
        Position {
            white: 283691314579456,
            black: 240786604032,
            white_to_move: false,
        },
        Position {
            white: 69123704832,
            black: 6648609374208,
            white_to_move: false,
        },
        Position {
            white: 69659527168,
            black: 6648609374208,
            white_to_move: false,
        },
        Position {
            white: 128983762944,
            black: 6597338202112,
            white_to_move: false,
        },
        Position {
            white: 206561609728,
            black: 6648609374208,
            white_to_move: false,
        },
        Position {
            white: 8899306981376,
            black: 6614518071296,
            white_to_move: false,
        },
        Position {
            white: 283760034056192,
            black: 4432674684928,
            white_to_move: false,
        },
        Position {
            white: 34765015040,
            black: 19877108645888,
            white_to_move: false,
        },
        Position {
            white: 35299788800,
            black: 19877108645888,
            white_to_move: false,
        },
        Position {
            white: 60264286208,
            black: 19860197212160,
            white_to_move: false,
        },
        Position {
            white: 240921348096,
            black: 19808389169152,
            white_to_move: false,
        },
        Position {
            white: 35287586048000,
            black: 19808657604608,
            white_to_move: false,
        },
        Position {
            white: 283725674317824,
            black: 17661173956608,
            white_to_move: false,
        },
        Position {
            white: 9024825935267840,
            black: 2285191036928,
            white_to_move: false,
        },
        Position {
            white: 103213695488,
            black: 2216472215552,
            white_to_move: false,
        },
        Position {
            white: 103213959168,
            black: 2216471953408,
            white_to_move: false,
        },
        Position {
            white: 103482916864,
            black: 2216204042240,
            white_to_move: false,
        },
        Position {
            white: 103483965440,
            black: 2216204042240,
            white_to_move: false,
        },
        Position {
            white: 104018739200,
            black: 2216204042240,
            white_to_move: false,
        },
        Position {
            white: 128983236608,
            black: 2199292608512,
            white_to_move: false,
        },
        Position {
            white: 283794393268224,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 103214221312,
            black: 2216472608768,
            white_to_move: false,
        },
        Position {
            white: 103348965376,
            black: 2216337866752,
            white_to_move: false,
        },
        Position {
            white: 103351582720,
            black: 2216337342464,
            white_to_move: false,
        },
        Position {
            white: 128849281024,
            black: 2199427481600,
            white_to_move: false,
        },
        Position {
            white: 103214221312,
            black: 2216538669056,
            white_to_move: false,
        },
        Position {
            white: 103146717184,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 103349485568,
            black: 2216404451328,
            white_to_move: false,
        },
        Position {
            white: 103350009856,
            black: 2216404975616,
            white_to_move: false,
        },
        Position {
            white: 128849281024,
            black: 2199493541888,
            white_to_move: false,
        },
        Position {
            white: 4518372966400,
            black: 2199426433024,
            white_to_move: false,
        },
        Position {
            white: 135006208,
            black: 2456989728768,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 2456989728768,
            white_to_move: false,
        },
        Position {
            white: 939786240,
            black: 2456721817600,
            white_to_move: false,
        },
        Position {
            white: 8830587240448,
            black: 2422630514688,
            white_to_move: false,
        },
        Position {
            white: 35253226045440,
            black: 2388270776320,
            white_to_move: false,
        },
        Position {
            white: 283691314315264,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 68854482944,
            black: 6648877809664,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 6648609374208,
            white_to_move: false,
        },
        Position {
            white: 69659262976,
            black: 6648609898496,
            white_to_move: false,
        },
        Position {
            white: 128983498752,
            black: 6597338726400,
            white_to_move: false,
        },
        Position {
            white: 8899306717184,
            black: 6614518595584,
            white_to_move: false,
        },
        Position {
            white: 283760033792000,
            black: 4432675209216,
            white_to_move: false,
        },
        Position {
            white: 68989747200,
            black: 11046790103040,
            white_to_move: false,
        },
        Position {
            white: 128849281024,
            black: 10995519455232,
            white_to_move: false,
        },
        Position {
            white: 1134764719603712,
            black: 2250966040576,
            white_to_move: false,
        },
        Position {
            white: 34494744576,
            black: 19877377081344,
            white_to_move: false,
        },
        Position {
            white: 34495791104,
            black: 19877377081344,
            white_to_move: false,
        },
        Position {
            white: 34764750848,
            black: 19877109170176,
            white_to_move: false,
        },
        Position {
            white: 35299524608,
            black: 19877109170176,
            white_to_move: false,
        },
        Position {
            white: 60264022016,
            black: 19860197736448,
            white_to_move: false,
        },
        Position {
            white: 240652648448,
            black: 19808658128896,
            white_to_move: false,
        },
        Position {
            white: 35287585783808,
            black: 19808658128896,
            white_to_move: false,
        },
        Position {
            white: 283725674053632,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 9024825935003648,
            black: 2285191561216,
            white_to_move: false,
        },
        Position {
            white: 103348175872,
            black: 2216340488192,
            white_to_move: false,
        },
        Position {
            white: 103482394624,
            black: 2216206270464,
            white_to_move: false,
        },
        Position {
            white: 103348703232,
            black: 2216339963904,
            white_to_move: false,
        },
        Position {
            white: 103349764096,
            black: 2216338915328,
            white_to_move: false,
        },
        Position {
            white: 103482130432,
            black: 2216206794752,
            white_to_move: false,
        },
        Position {
            white: 103548977152,
            black: 2216206794752,
            white_to_move: false,
        },
        Position {
            white: 129117454336,
            black: 2199161143296,
            white_to_move: false,
        },
        Position {
            white: 103215007744,
            black: 2217008431104,
            white_to_move: false,
        },
        Position {
            white: 103215267840,
            black: 2217008431104,
            white_to_move: false,
        },
        Position {
            white: 103350796288,
            black: 2216874737664,
            white_to_move: false,
        },
        Position {
            white: 103621328896,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 128850067456,
            black: 2199963303936,
            white_to_move: false,
        },
        Position {
            white: 378495041536,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 283794394316800,
            black: 805830656,
            white_to_move: false,
        },
        Position {
            white: 1835008,
            black: 2457123946496,
            white_to_move: false,
        },
        Position {
            white: 17661175005184,
            black: 2388136558592,
            white_to_move: false,
        },
        Position {
            white: 283691315101696,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 68989486080,
            black: 11046790103040,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 11046655885312,
            white_to_move: false,
        },
        Position {
            white: 69190287360,
            black: 11046656409600,
            white_to_move: false,
        },
        Position {
            white: 129118502912,
            black: 10995251019776,
            white_to_move: false,
        },
        Position {
            white: 4501395210240,
            black: 11012430888960,
            white_to_move: false,
        },
        Position {
            white: 283760303013888,
            black: 8830453284864,
            white_to_move: false,
        },
        Position {
            white: 1134764988825600,
            black: 2250697605120,
            white_to_move: false,
        },
        Position {
            white: 34629747712,
            black: 37469428908032,
            white_to_move: false,
        },
        Position {
            white: 34763966464,
            black: 37469294690304,
            white_to_move: false,
        },
        Position {
            white: 34630008832,
            black: 37469428908032,
            white_to_move: false,
        },
        Position {
            white: 34830548992,
            black: 37469295214592,
            white_to_move: false,
        },
        Position {
            white: 60399026176,
            black: 37452249563136,
            white_to_move: false,
        },
        Position {
            white: 240787652608,
            black: 37400709955584,
            white_to_move: false,
        },
        Position {
            white: 17695534743552,
            black: 37400709955584,
            white_to_move: false,
        },
        Position {
            white: 283725943275520,
            black: 35253092089856,
            white_to_move: false,
        },
        Position {
            white: 103216056320,
            black: 2217008431104,
            white_to_move: false,
        },
        Position {
            white: 103215792128,
            black: 2217008955392,
            white_to_move: false,
        },
        Position {
            white: 103350796288,
            black: 2216874737664,
            white_to_move: false,
        },
        Position {
            white: 103622377472,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 128851116032,
            black: 2199963303936,
            white_to_move: false,
        },
        Position {
            white: 241057136640,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 4432676782080,
            black: 2422496296960,
            white_to_move: false,
        },
        Position {
            white: 68990534656,
            black: 11046790103040,
            white_to_move: false,
        },
        Position {
            white: 69124489216,
            black: 11046656409600,
            white_to_move: false,
        },
        Position {
            white: 69191335936,
            black: 11046656409600,
            white_to_move: false,
        },
        Position {
            white: 129119551488,
            black: 10995251019776,
            white_to_move: false,
        },
        Position {
            white: 4501396258816,
            black: 11012430888960,
            white_to_move: false,
        },
        Position {
            white: 1134764989874176,
            black: 2250697605120,
            white_to_move: false,
        },
        Position {
            white: 34630796288,
            black: 37469428908032,
            white_to_move: false,
        },
        Position {
            white: 34765015040,
            black: 37469294690304,
            white_to_move: false,
        },
        Position {
            white: 34831597568,
            black: 37469295214592,
            white_to_move: false,
        },
        Position {
            white: 60400074752,
            black: 37452249563136,
            white_to_move: false,
        },
        Position {
            white: 240788701184,
            black: 37400709955584,
            white_to_move: false,
        },
        Position {
            white: 17695535792128,
            black: 37400709955584,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 2207915180032,
            white_to_move: false,
        },
        Position {
            white: 120292704256,
            black: 2208016367616,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 2207915704320,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 2207647268864,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 2207781486592,
            white_to_move: false,
        },
        Position {
            white: 133143986176,
            black: 2199459987456,
            white_to_move: false,
        },
        Position {
            white: 283794259050496,
            black: 9026666496,
            white_to_move: false,
        },
        Position {
            white: 94623760384,
            black: 6631698464768,
            white_to_move: false,
        },
        Position {
            white: 94892982272,
            black: 6631430029312,
            white_to_move: false,
        },
        Position {
            white: 283768489246720,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 565243465957376,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 1130392442634240,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 2256292349476864,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 94623760384,
            black: 11029744975872,
            white_to_move: false,
        },
        Position {
            white: 94892982272,
            black: 11029476540416,
            white_to_move: false,
        },
        Position {
            white: 283768489246720,
            black: 8830855938048,
            white_to_move: false,
        },
        Position {
            white: 565243465957376,
            black: 8830855938048,
            white_to_move: false,
        },
        Position {
            white: 1134790489145344,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 4512490209673216,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 60264286208,
            black: 19860197212160,
            white_to_move: false,
        },
        Position {
            white: 60264808448,
            black: 19860197736448,
            white_to_move: false,
        },
        Position {
            white: 60400074752,
            black: 19860063518720,
            white_to_move: false,
        },
        Position {
            white: 266287972352,
            black: 19791612477440,
            white_to_move: false,
        },
        Position {
            white: 283734129508352,
            black: 17661308698624,
            white_to_move: false,
        },
        Position {
            white: 565209106219008,
            black: 17661308698624,
            white_to_move: false,
        },
        Position {
            white: 9024851570327552,
            black: 2268145909760,
            white_to_move: false,
        },
        Position {
            white: 60264286208,
            black: 37452383256576,
            white_to_move: false,
        },
        Position {
            white: 60264808448,
            black: 37452383780864,
            white_to_move: false,
        },
        Position {
            white: 60400074752,
            black: 37452249563136,
            white_to_move: false,
        },
        Position {
            white: 266287972352,
            black: 37383798521856,
            white_to_move: false,
        },
        Position {
            white: 283734129508352,
            black: 35253494743040,
            white_to_move: false,
        },
        Position {
            white: 565209106219008,
            black: 35253494743040,
            white_to_move: false,
        },
        Position {
            white: 68854482944,
            black: 4458444488704,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 4458176053248,
            white_to_move: false,
        },
        Position {
            white: 69659262976,
            black: 4458176577536,
            white_to_move: false,
        },
        Position {
            white: 133278466048,
            black: 4398315470848,
            white_to_move: false,
        },
        Position {
            white: 2285057081344,
            black: 4441265143808,
            white_to_move: false,
        },
        Position {
            white: 8899306717184,
            black: 4424085274624,
            white_to_move: false,
        },
        Position {
            white: 17314875392,
            black: 4638833115136,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 4638833115136,
            white_to_move: false,
        },
        Position {
            white: 18119655424,
            black: 4638565203968,
            white_to_move: false,
        },
        Position {
            white: 532710424576,
            black: 4398315470848,
            white_to_move: false,
        },
        Position {
            white: 8847767109632,
            black: 4604473901056,
            white_to_move: false,
        },
        Position {
            white: 35270405914624,
            black: 4570114162688,
            white_to_move: false,
        },
        Position {
            white: 1130315267702784,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 17314875392,
            black: 22093580206080,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 22093580206080,
            white_to_move: false,
        },
        Position {
            white: 18119655424,
            black: 22093312294912,
            white_to_move: false,
        },
        Position {
            white: 257832517632,
            black: 21990501515264,
            white_to_move: false,
        },
        Position {
            white: 8847767109632,
            black: 22059220992000,
            white_to_move: false,
        },
        Position {
            white: 35270405914624,
            black: 22024861253632,
            white_to_move: false,
        },
        Position {
            white: 1130315267702784,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 86034089984,
            black: 4432677830656,
            white_to_move: false,
        },
        Position {
            white: 86303051776,
            black: 4432408870912,
            white_to_move: false,
        },
        Position {
            white: 86034620416,
            black: 4432677306368,
            white_to_move: false,
        },
        Position {
            white: 86838870016,
            black: 4432409919488,
            white_to_move: false,
        },
        Position {
            white: 8916486324224,
            black: 4398318616576,
            white_to_move: false,
        },
        Position {
            white: 1130383986917376,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 69123703808,
            black: 4449653227520,
            white_to_move: false,
        },
        Position {
            white: 69123704832,
            black: 4449653227520,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 4449653227520,
            white_to_move: false,
        },
        Position {
            white: 69223841792,
            black: 4449586642944,
            white_to_move: false,
        },
        Position {
            white: 129252720640,
            black: 4398114144256,
            white_to_move: false,
        },
        Position {
            white: 2285326303232,
            black: 4432473882624,
            white_to_move: false,
        },
        Position {
            white: 8899575939072,
            black: 4415294013440,
            white_to_move: false,
        },
        Position {
            white: 567451482849280,
            black: 17247502336,
            white_to_move: false,
        },
        Position {
            white: 69123703808,
            black: 4458176053248,
            white_to_move: false,
        },
        Position {
            white: 69123704832,
            black: 4458176053248,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 4458176053248,
            white_to_move: false,
        },
        Position {
            white: 133547687936,
            black: 4398047035392,
            white_to_move: false,
        },
        Position {
            white: 2285326303232,
            black: 4440996708352,
            white_to_move: false,
        },
        Position {
            white: 8899575939072,
            black: 4423816839168,
            white_to_move: false,
        },
        Position {
            white: 567451482849280,
            black: 25770328064,
            white_to_move: false,
        },
        Position {
            white: 17315661824,
            black: 4638833115136,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 4638833115136,
            white_to_move: false,
        },
        Position {
            white: 18120441856,
            black: 4638565203968,
            white_to_move: false,
        },
        Position {
            white: 532711211008,
            black: 4398315470848,
            white_to_move: false,
        },
        Position {
            white: 8847767896064,
            black: 4604473901056,
            white_to_move: false,
        },
        Position {
            white: 17678489092096,
            black: 4569845727232,
            white_to_move: false,
        },
        Position {
            white: 35270406701056,
            black: 4570114162688,
            white_to_move: false,
        },
        Position {
            white: 1130315268489216,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 1130366807310336,
            black: 60398501888,
            white_to_move: false,
        },
        Position {
            white: 1130366942314496,
            black: 60264284160,
            white_to_move: false,
        },
        Position {
            white: 1130401303101440,
            black: 25904545792,
            white_to_move: false,
        },
        Position {
            white: 1130375279542272,
            black: 51942785024,
            white_to_move: false,
        },
        Position {
            white: 1130383919808512,
            black: 43352850432,
            white_to_move: false,
        },
        Position {
            white: 1130431097339904,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 1130315268489216,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 1130349763493888,
            black: 206293172224,
            white_to_move: false,
        },
        Position {
            white: 1130830529298432,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 1130366807310336,
            black: 2250831822848,
            white_to_move: false,
        },
        Position {
            white: 1130366942314496,
            black: 2250697605120,
            white_to_move: false,
        },
        Position {
            white: 1130401303101440,
            black: 2216337866752,
            white_to_move: false,
        },
        Position {
            white: 1130383919808512,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 1132569991053312,
            black: 51942785024,
            white_to_move: false,
        },
        Position {
            white: 1130426802372608,
            black: 2199426433024,
            white_to_move: false,
        },
        Position {
            white: 1133665207713792,
            black: 51942785024,
            white_to_move: false,
        },
        Position {
            white: 1130315268489216,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 1130349763493888,
            black: 17661040263168,
            white_to_move: false,
        },
        Position {
            white: 1130555651391488,
            black: 17592589221888,
            white_to_move: false,
        },
        Position {
            white: 1130315268489216,
            black: 35287720263680,
            white_to_move: false,
        },
        Position {
            white: 1130349763493888,
            black: 35253226307584,
            white_to_move: false,
        },
        Position {
            white: 1130555651391488,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 1125985940668416,
            black: 567382628630528,
            white_to_move: false,
        },
        Position {
            white: 1125986209890304,
            black: 567382360195072,
            white_to_move: false,
        },
        Position {
            white: 1970410736320512,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 86033826304,
            black: 8830721851392,
            white_to_move: false,
        },
        Position {
            white: 86034089984,
            black: 8830721589248,
            white_to_move: false,
        },
        Position {
            white: 86303047680,
            black: 8830453678080,
            white_to_move: false,
        },
        Position {
            white: 86838870016,
            black: 8830453678080,
            white_to_move: false,
        },
        Position {
            white: 1134782033428480,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 2260716300009472,
            black: 269352960,
            white_to_move: false,
        },
        Position {
            white: 4512481753956352,
            black: 34629091328,
            white_to_move: false,
        },
        Position {
            white: 68854482944,
            black: 8847934619648,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 8847666184192,
            white_to_move: false,
        },
        Position {
            white: 69659262976,
            black: 8847666708480,
            white_to_move: false,
        },
        Position {
            white: 73182478336,
            black: 8847901589504,
            white_to_move: false,
        },
        Position {
            white: 128983498752,
            black: 8796395536384,
            white_to_move: false,
        },
        Position {
            white: 2285057081344,
            black: 8830755274752,
            white_to_move: false,
        },
        Position {
            white: 1134764853821440,
            black: 51842121728,
            white_to_move: false,
        },
        Position {
            white: 2260699120402432,
            black: 17482383360,
            white_to_move: false,
        },
        Position {
            white: 86303834112,
            black: 8830519869440,
            white_to_move: false,
        },
        Position {
            white: 1134781899472896,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 4512481620000768,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 17314875392,
            black: 8899977543680,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 8899977543680,
            white_to_move: false,
        },
        Position {
            white: 19193397248,
            black: 8899172761600,
            white_to_move: false,
        },
        Position {
            white: 257832517632,
            black: 8796898852864,
            white_to_move: false,
        },
        Position {
            white: 35270405914624,
            black: 8831258591232,
            white_to_move: false,
        },
        Position {
            white: 2260647580794880,
            black: 69525307392,
            white_to_move: false,
        },
        Position {
            white: 4512413034741760,
            black: 103885045760,
            white_to_move: false,
        },
        Position {
            white: 68854482944,
            black: 8856490999808,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 8856222564352,
            white_to_move: false,
        },
        Position {
            white: 69659262976,
            black: 8856223088640,
            white_to_move: false,
        },
        Position {
            white: 133278466048,
            black: 8796361981952,
            white_to_move: false,
        },
        Position {
            white: 2285057081344,
            black: 8839311654912,
            white_to_move: false,
        },
        Position {
            white: 1134764853821440,
            black: 60398501888,
            white_to_move: false,
        },
        Position {
            white: 2260699120402432,
            black: 26038763520,
            white_to_move: false,
        },
        Position {
            white: 17314875392,
            black: 9036879626240,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 9036879626240,
            white_to_move: false,
        },
        Position {
            white: 18119655424,
            black: 9036611715072,
            white_to_move: false,
        },
        Position {
            white: 532710424576,
            black: 8796361981952,
            white_to_move: false,
        },
        Position {
            white: 35270405914624,
            black: 8968160673792,
            white_to_move: false,
        },
        Position {
            white: 2260647580794880,
            black: 206427389952,
            white_to_move: false,
        },
        Position {
            white: 4512413034741760,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 17314875392,
            black: 26491626717184,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 26491626717184,
            white_to_move: false,
        },
        Position {
            white: 18119655424,
            black: 26491358806016,
            white_to_move: false,
        },
        Position {
            white: 257832517632,
            black: 26388548026368,
            white_to_move: false,
        },
        Position {
            white: 35270405914624,
            black: 26422907764736,
            white_to_move: false,
        },
        Position {
            white: 2260647580794880,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 4512413034741760,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 86034089984,
            black: 8830724341760,
            white_to_move: false,
        },
        Position {
            white: 86303051776,
            black: 8830455382016,
            white_to_move: false,
        },
        Position {
            white: 86034620416,
            black: 8830723817472,
            white_to_move: false,
        },
        Position {
            white: 86838870016,
            black: 8830456430592,
            white_to_move: false,
        },
        Position {
            white: 1134782033428480,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 2260716300009472,
            black: 272105472,
            white_to_move: false,
        },
        Position {
            white: 4512481753956352,
            black: 34631843840,
            white_to_move: false,
        },
        Position {
            white: 69123703808,
            black: 8847666184192,
            white_to_move: false,
        },
        Position {
            white: 69123704832,
            black: 8847666184192,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 8847666184192,
            white_to_move: false,
        },
        Position {
            white: 129252720640,
            black: 8796127100928,
            white_to_move: false,
        },
        Position {
            white: 2285326303232,
            black: 8830486839296,
            white_to_move: false,
        },
        Position {
            white: 4501529427968,
            black: 8813306970112,
            white_to_move: false,
        },
        Position {
            white: 1134765123043328,
            black: 51573686272,
            white_to_move: false,
        },
        Position {
            white: 2260699389624320,
            black: 17213947904,
            white_to_move: false,
        },
        Position {
            white: 17584096256,
            black: 8899709108224,
            white_to_move: false,
        },
        Position {
            white: 17584097280,
            black: 8899709108224,
            white_to_move: false,
        },
        Position {
            white: 17584357376,
            black: 8899709108224,
            white_to_move: false,
        },
        Position {
            white: 19194183680,
            black: 8899172761600,
            white_to_move: false,
        },
        Position {
            white: 258101739520,
            black: 8796630417408,
            white_to_move: false,
        },
        Position {
            white: 292998348800,
            black: 8899172761600,
            white_to_move: false,
        },
        Position {
            white: 4449989820416,
            black: 8865349894144,
            white_to_move: false,
        },
        Position {
            white: 17678489092096,
            black: 8830990155776,
            white_to_move: false,
        },
        Position {
            white: 35270675136512,
            black: 8830990155776,
            white_to_move: false,
        },
        Position {
            white: 2260647850016768,
            black: 69256871936,
            white_to_move: false,
        },
        Position {
            white: 4512413303963648,
            black: 103616610304,
            white_to_move: false,
        },
        Position {
            white: 69123703808,
            black: 8856222564352,
            white_to_move: false,
        },
        Position {
            white: 69123704832,
            black: 8856222564352,
            white_to_move: false,
        },
        Position {
            white: 69123964928,
            black: 8856222564352,
            white_to_move: false,
        },
        Position {
            white: 133547687936,
            black: 8796093546496,
            white_to_move: false,
        },
        Position {
            white: 2285326303232,
            black: 8839043219456,
            white_to_move: false,
        },
        Position {
            white: 4501529427968,
            black: 8821863350272,
            white_to_move: false,
        },
        Position {
            white: 1134765123043328,
            black: 60130066432,
            white_to_move: false,
        },
        Position {
            white: 2260699389624320,
            black: 25770328064,
            white_to_move: false,
        },
        Position {
            white: 17315661824,
            black: 9036879626240,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 9036879626240,
            white_to_move: false,
        },
        Position {
            white: 18120441856,
            black: 9036611715072,
            white_to_move: false,
        },
        Position {
            white: 532711211008,
            black: 8796361981952,
            white_to_move: false,
        },
        Position {
            white: 17678489092096,
            black: 8967892238336,
            white_to_move: false,
        },
        Position {
            white: 35270406701056,
            black: 8968160673792,
            white_to_move: false,
        },
        Position {
            white: 2260647581581312,
            black: 206427389952,
            white_to_move: false,
        },
        Position {
            white: 4512413035528192,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 1134799213824000,
            black: 26038239232,
            white_to_move: false,
        },
        Position {
            white: 1134764853821440,
            black: 60398501888,
            white_to_move: false,
        },
        Position {
            white: 1134764988825600,
            black: 60264284160,
            white_to_move: false,
        },
        Position {
            white: 1134781932765184,
            black: 43352850432,
            white_to_move: false,
        },
        Position {
            white: 1134829143851008,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 1134747674216448,
            black: 206426865664,
            white_to_move: false,
        },
        Position {
            white: 1134713315000320,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 1134782436081664,
            black: 172201869312,
            white_to_move: false,
        },
        Position {
            white: 1135228575809536,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 1134799213824000,
            black: 2216471560192,
            white_to_move: false,
        },
        Position {
            white: 1134764853821440,
            black: 2250831822848,
            white_to_move: false,
        },
        Position {
            white: 1134764988825600,
            black: 2250697605120,
            white_to_move: false,
        },
        Position {
            white: 1134781932765184,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 1136968037564416,
            black: 51942785024,
            white_to_move: false,
        },
        Position {
            white: 1134824848883712,
            black: 2199426433024,
            white_to_move: false,
        },
        Position {
            white: 1134747674216448,
            black: 17661173956608,
            white_to_move: false,
        },
        Position {
            white: 1134713315000320,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 1134782436081664,
            black: 17626948960256,
            white_to_move: false,
        },
        Position {
            white: 1134953697902592,
            black: 17592589221888,
            white_to_move: false,
        },
        Position {
            white: 1187489737867264,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 1134747674216448,
            black: 35253360001024,
            white_to_move: false,
        },
        Position {
            white: 1134713315000320,
            black: 35287720263680,
            white_to_move: false,
        },
        Position {
            white: 1134782436081664,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 1134953697902592,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 1125985940668416,
            black: 2260630535405568,
            white_to_move: false,
        },
        Position {
            white: 1125986209890304,
            black: 2260630266970112,
            white_to_move: false,
        },
        Position {
            white: 7890181340266496,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 4512498934351872,
            black: 26038239232,
            white_to_move: false,
        },
        Position {
            white: 4512464574349312,
            black: 60398501888,
            white_to_move: false,
        },
        Position {
            white: 4512464709353472,
            black: 60264284160,
            white_to_move: false,
        },
        Position {
            white: 4512481653293056,
            black: 43352850432,
            white_to_move: false,
        },
        Position {
            white: 4512528864378880,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 4512447394744320,
            black: 206426865664,
            white_to_move: false,
        },
        Position {
            white: 4512413035528192,
            black: 240787128320,
            white_to_move: false,
        },
        Position {
            white: 4512482156609536,
            black: 172201869312,
            white_to_move: false,
        },
        Position {
            white: 4512928296337408,
            black: 403177472,
            white_to_move: false,
        },
        Position {
            white: 4512498934351872,
            black: 2216471560192,
            white_to_move: false,
        },
        Position {
            white: 4512464574349312,
            black: 2250831822848,
            white_to_move: false,
        },
        Position {
            white: 4512464709353472,
            black: 2250697605120,
            white_to_move: false,
        },
        Position {
            white: 4512481653293056,
            black: 2233786171392,
            white_to_move: false,
        },
        Position {
            white: 4512524569411584,
            black: 2199426433024,
            white_to_move: false,
        },
        Position {
            white: 4512447394744320,
            black: 17661173956608,
            white_to_move: false,
        },
        Position {
            white: 4530074209484800,
            black: 34360262656,
            white_to_move: false,
        },
        Position {
            white: 4512482156609536,
            black: 17626948960256,
            white_to_move: false,
        },
        Position {
            white: 4512653418430464,
            black: 17592589221888,
            white_to_move: false,
        },
        Position {
            white: 4565189458395136,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 4512447394744320,
            black: 35253360001024,
            white_to_move: false,
        },
        Position {
            white: 4512413035528192,
            black: 35287720263680,
            white_to_move: false,
        },
        Position {
            white: 4512482156609536,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 4512653418430464,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 4547872150257664,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 4503685661196288,
            black: 2260630535405568,
            white_to_move: false,
        },
        Position {
            white: 4503685930418176,
            black: 2260630266970112,
            white_to_move: false,
        },
        Position {
            white: 7890181340266496,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 51673828864,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 51944359936,
            black: 17660906046464,
            white_to_move: false,
        },
        Position {
            white: 52479133696,
            black: 17660906046464,
            white_to_move: false,
        },
        Position {
            white: 257832257536,
            black: 17592455005184,
            white_to_move: false,
        },
        Position {
            white: 35304765392896,
            black: 17592455005184,
            white_to_move: false,
        },
        Position {
            white: 9024843114612736,
            black: 68988437504,
            white_to_move: false,
        },
        Position {
            white: 51540527104,
            black: 17661308174336,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 17661174218752,
            white_to_move: false,
        },
        Position {
            white: 51810666496,
            black: 17661040001024,
            white_to_move: false,
        },
        Position {
            white: 51573950464,
            black: 17661308174336,
            white_to_move: false,
        },
        Position {
            white: 257966999552,
            black: 17592320524288,
            white_to_move: false,
        },
        Position {
            white: 9024842980919296,
            black: 69122392064,
            white_to_move: false,
        },
        Position {
            white: 17247766528,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 17315661824,
            black: 17695600803840,
            white_to_move: false,
        },
        Position {
            white: 25837438976,
            black: 17695667912704,
            white_to_move: false,
        },
        Position {
            white: 257966999552,
            black: 17592387371008,
            white_to_move: false,
        },
        Position {
            white: 8847767373824,
            black: 17661241065472,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 17721035063296,
            white_to_move: false,
        },
        Position {
            white: 137842132992,
            black: 17721035063296,
            white_to_move: false,
        },
        Position {
            white: 2216337868800,
            black: 17704123629568,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 17686943760384,
            white_to_move: false,
        },
        Position {
            white: 35253226309632,
            black: 17652584022016,
            white_to_move: false,
        },
        Position {
            white: 18119919616,
            black: 22093311770624,
            white_to_move: false,
        },
        Position {
            white: 258101217280,
            black: 21990232555520,
            white_to_move: false,
        },
        Position {
            white: 8847767373824,
            black: 22059220467712,
            white_to_move: false,
        },
        Position {
            white: 35270406178816,
            black: 22024860729344,
            white_to_move: false,
        },
        Position {
            white: 1130315267966976,
            black: 17695533694976,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 17661175009280,
            white_to_move: false,
        },
        Position {
            white: 51674882048,
            black: 17661174484992,
            white_to_move: false,
        },
        Position {
            white: 51944357888,
            black: 17660907098112,
            white_to_move: false,
        },
        Position {
            white: 52479131648,
            black: 17660907098112,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 17592456056832,
            white_to_move: false,
        },
        Position {
            white: 35304765390848,
            black: 17592456056832,
            white_to_move: false,
        },
        Position {
            white: 9024843114610688,
            black: 68989489152,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 17661174218752,
            white_to_move: false,
        },
        Position {
            white: 51541573632,
            black: 17661308174336,
            white_to_move: false,
        },
        Position {
            white: 51811188736,
            black: 17661040525312,
            white_to_move: false,
        },
        Position {
            white: 257699086336,
            black: 17592589484032,
            white_to_move: false,
        },
        Position {
            white: 4521312341983232,
            black: 135004160,
            white_to_move: false,
        },
        Position {
            white: 9024842981441536,
            black: 69122916352,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 17661177102336,
            white_to_move: false,
        },
        Position {
            white: 51674882048,
            black: 17661176578048,
            white_to_move: false,
        },
        Position {
            white: 51944374272,
            black: 17660907094016,
            white_to_move: false,
        },
        Position {
            white: 52479131648,
            black: 17660909191168,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 17592458149888,
            white_to_move: false,
        },
        Position {
            white: 35304765390848,
            black: 17592458149888,
            white_to_move: false,
        },
        Position {
            white: 9024843114610688,
            black: 68991582208,
            white_to_move: false,
        },
        Position {
            white: 135792640,
            black: 17721303498752,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 17721303498752,
            white_to_move: false,
        },
        Position {
            white: 940572672,
            black: 17721035587584,
            white_to_move: false,
        },
        Position {
            white: 2216338391040,
            black: 17704124153856,
            white_to_move: false,
        },
        Position {
            white: 8830588026880,
            black: 17686944284672,
            white_to_move: false,
        },
        Position {
            white: 35253226831872,
            black: 17652584546304,
            white_to_move: false,
        },
        Position {
            white: 4521260936593408,
            black: 60130066432,
            white_to_move: false,
        },
        Position {
            white: 17315661824,
            black: 22093580206080,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 22093580206080,
            white_to_move: false,
        },
        Position {
            white: 18120441856,
            black: 22093312294912,
            white_to_move: false,
        },
        Position {
            white: 257833304064,
            black: 21990501515264,
            white_to_move: false,
        },
        Position {
            white: 8847767896064,
            black: 22059220992000,
            white_to_move: false,
        },
        Position {
            white: 35270406701056,
            black: 22024861253632,
            white_to_move: false,
        },
        Position {
            white: 1130315268489216,
            black: 17695534219264,
            white_to_move: false,
        },
        Position {
            white: 4521278116462592,
            black: 4432406773760,
            white_to_move: false,
        },
        Position {
            white: 17181704192,
            black: 26491760934912,
            white_to_move: false,
        },
        Position {
            white: 257699086336,
            black: 26388682244096,
            white_to_move: false,
        },
        Position {
            white: 4530074075267072,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 51676448768,
            black: 17661175005184,
            white_to_move: false,
        },
        Position {
            white: 51676979200,
            black: 17661174480896,
            white_to_move: false,
        },
        Position {
            white: 51543539712,
            black: 17661308174336,
            white_to_move: false,
        },
        Position {
            white: 257700134912,
            black: 17592590270464,
            white_to_move: false,
        },
        Position {
            white: 9024842982490112,
            black: 69123702784,
            white_to_move: false,
        },
        Position {
            white: 17450927104,
            black: 17695466586112,
            white_to_move: false,
        },
        Position {
            white: 17517772800,
            black: 17695400001536,
            white_to_move: false,
        },
        Position {
            white: 17585668096,
            black: 17695332892672,
            white_to_move: false,
        },
        Position {
            white: 17685282816,
            black: 17695265783808,
            white_to_move: false,
        },
        Position {
            white: 257968570368,
            black: 17592387895296,
            white_to_move: false,
        },
        Position {
            white: 4449856651264,
            black: 17661107372032,
            white_to_move: false,
        },
        Position {
            white: 4521277983293440,
            black: 34561589248,
            white_to_move: false,
        },
        Position {
            white: 51676448768,
            black: 17661710827520,
            white_to_move: false,
        },
        Position {
            white: 51676971008,
            black: 17661711351808,
            white_to_move: false,
        },
        Position {
            white: 258237005824,
            black: 17592589221888,
            white_to_move: false,
        },
        Position {
            white: 9024842982490112,
            black: 69659525120,
            white_to_move: false,
        },
        Position {
            white: 271057920,
            black: 17721169281024,
            white_to_move: false,
        },
        Position {
            white: 471859200,
            black: 17721035587584,
            white_to_move: false,
        },
        Position {
            white: 4432676782080,
            black: 17686810066944,
            white_to_move: false,
        },
        Position {
            white: 4521260803424256,
            black: 60264284160,
            white_to_move: false,
        },
        Position {
            white: 51676448768,
            black: 17798612910080,
            white_to_move: false,
        },
        Position {
            white: 51676971008,
            black: 17798613434368,
            white_to_move: false,
        },
        Position {
            white: 532578041856,
            black: 17592589221888,
            white_to_move: false,
        },
        Position {
            white: 9024842982490112,
            black: 206561607680,
            white_to_move: false,
        },
        Position {
            white: 34630796288,
            black: 19877242863616,
            white_to_move: false,
        },
        Position {
            white: 34765015040,
            black: 19877108645888,
            white_to_move: false,
        },
        Position {
            white: 34831597568,
            black: 19877109170176,
            white_to_move: false,
        },
        Position {
            white: 60400074752,
            black: 19860063518720,
            white_to_move: false,
        },
        Position {
            white: 240788701184,
            black: 19808523911168,
            white_to_move: false,
        },
        Position {
            white: 4521295163162624,
            black: 2216337866752,
            white_to_move: false,
        },
        Position {
            white: 9024826071056384,
            black: 2285057343488,
            white_to_move: false,
        },
        Position {
            white: 17450927104,
            black: 26491492499456,
            white_to_move: false,
        },
        Position {
            white: 17585668096,
            black: 26491358806016,
            white_to_move: false,
        },
        Position {
            white: 17651728384,
            black: 26491358806016,
            white_to_move: false,
        },
        Position {
            white: 257968570368,
            black: 26388413808640,
            white_to_move: false,
        },
        Position {
            white: 4449856651264,
            black: 26457133285376,
            white_to_move: false,
        },
        Position {
            white: 4530074076315648,
            black: 34494480384,
            white_to_move: false,
        },
        Position {
            white: 223607260160,
            black: 17626747109376,
            white_to_move: false,
        },
        Position {
            white: 223539888128,
            black: 17626814742528,
            white_to_move: false,
        },
        Position {
            white: 223742001152,
            black: 17626613415936,
            white_to_move: false,
        },
        Position {
            white: 2269615338029056,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 4521415151714304,
            black: 34830024704,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 17730967175168,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 17730967699456,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 17730699264000,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 17730833481728,
            white_to_move: false,
        },
        Position {
            white: 532575944704,
            black: 17593662963712,
            white_to_move: false,
        },
        Position {
            white: 4521312072499200,
            black: 138915872768,
            white_to_move: false,
        },
        Position {
            white: 9024911699869696,
            black: 138915872768,
            white_to_move: false,
        },
        Position {
            white: 240787129344,
            black: 19808523386880,
            white_to_move: false,
        },
        Position {
            white: 240652912640,
            black: 19808657604608,
            white_to_move: false,
        },
        Position {
            white: 240652648448,
            black: 19808658128896,
            white_to_move: false,
        },
        Position {
            white: 240787652608,
            black: 19808523911168,
            white_to_move: false,
        },
        Position {
            white: 240788701184,
            black: 19808523911168,
            white_to_move: false,
        },
        Position {
            white: 266287972352,
            black: 19791612477440,
            white_to_move: false,
        },
        Position {
            white: 2269632517898240,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 4521432331583488,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 9025031958953984,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 223607260160,
            black: 22024726511616,
            white_to_move: false,
        },
        Position {
            white: 223472779264,
            black: 22024861253632,
            white_to_move: false,
        },
        Position {
            white: 223742001152,
            black: 22024592818176,
            white_to_move: false,
        },
        Position {
            white: 1130521291653120,
            black: 17626948960256,
            white_to_move: false,
        },
        Position {
            white: 2269615338029056,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 4521415151714304,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 223607260160,
            black: 26422773022720,
            white_to_move: false,
        },
        Position {
            white: 223472779264,
            black: 26422907764736,
            white_to_move: false,
        },
        Position {
            white: 223742001152,
            black: 26422639329280,
            white_to_move: false,
        },
        Position {
            white: 1134919338164224,
            black: 17626948960256,
            white_to_move: false,
        },
        Position {
            white: 2269615338029056,
            black: 8830855938048,
            white_to_move: false,
        },
        Position {
            white: 4530211244736512,
            black: 34762915840,
            white_to_move: false,
        },
        Position {
            white: 189247521792,
            black: 52845411827712,
            white_to_move: false,
        },
        Position {
            white: 189113305088,
            black: 52845546045440,
            white_to_move: false,
        },
        Position {
            white: 189113827328,
            black: 52845546569728,
            white_to_move: false,
        },
        Position {
            white: 189249093632,
            black: 52845412352000,
            white_to_move: false,
        },
        Position {
            white: 2269580978290688,
            black: 35253494743040,
            white_to_move: false,
        },
        Position {
            white: 9060164791435264,
            black: 69122654208,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 88098637611008,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 88098638135296,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 88098369699840,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 88098503917568,
            white_to_move: false,
        },
        Position {
            white: 532575944704,
            black: 87961333399552,
            white_to_move: false,
        },
        Position {
            white: 4521312072499200,
            black: 70506586308608,
            white_to_move: false,
        },
        Position {
            white: 9024911699869696,
            black: 70506586308608,
            white_to_move: false,
        },
        Position {
            white: 9024860429746176,
            black: 60264284160,
            white_to_move: false,
        },
        Position {
            white: 9024825867632640,
            black: 94892457984,
            white_to_move: false,
        },
        Position {
            white: 9024825935267840,
            black: 2285191036928,
            white_to_move: false,
        },
        Position {
            white: 9024894789484544,
            black: 2216337866752,
            white_to_move: false,
        },
        Position {
            white: 9024826071056384,
            black: 2285057343488,
            white_to_move: false,
        },
        Position {
            white: 9024851570327552,
            black: 2268145909760,
            white_to_move: false,
        },
        Position {
            white: 9025031958953984,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 9024877743833088,
            black: 4432406773760,
            white_to_move: false,
        },
        Position {
            white: 9024843047501824,
            black: 4467169165312,
            white_to_move: false,
        },
        Position {
            white: 9025049138823168,
            black: 4398449688576,
            white_to_move: false,
        },
        Position {
            white: 10155106574008320,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 9024877743833088,
            black: 8830453284864,
            white_to_move: false,
        },
        Position {
            white: 9024843047501824,
            black: 8865215676416,
            white_to_move: false,
        },
        Position {
            white: 9025049138823168,
            black: 8796496199680,
            white_to_move: false,
        },
        Position {
            white: 9038002760187904,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 13537204341047296,
            black: 103482392576,
            white_to_move: false,
        },
        Position {
            white: 9007250929092608,
            black: 4521260801327104,
            white_to_move: false,
        },
        Position {
            white: 9007250929614848,
            black: 4521260801851392,
            white_to_move: false,
        },
        Position {
            white: 9007251064881152,
            black: 4521260667633664,
            white_to_move: false,
        },
        Position {
            white: 9007456952778752,
            black: 4521192216592384,
            white_to_move: false,
        },
        Position {
            white: 15762650235404288,
            black: 17661308698624,
            white_to_move: false,
        },
        Position {
            white: 51673828864,
            black: 35253360525312,
            white_to_move: false,
        },
        Position {
            white: 51944359936,
            black: 35253092090880,
            white_to_move: false,
        },
        Position {
            white: 52479133696,
            black: 35253092090880,
            white_to_move: false,
        },
        Position {
            white: 257832257536,
            black: 35184641049600,
            white_to_move: false,
        },
        Position {
            white: 18049703274874880,
            black: 268960768,
            white_to_move: false,
        },
        Position {
            white: 51540527104,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 35253360263168,
            white_to_move: false,
        },
        Position {
            white: 51810666496,
            black: 35253226045440,
            white_to_move: false,
        },
        Position {
            white: 51573950464,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 257966999552,
            black: 35184506568704,
            white_to_move: false,
        },
        Position {
            white: 51607373824,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 51607504896,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 35253427109888,
            white_to_move: false,
        },
        Position {
            white: 51810666496,
            black: 35253292892160,
            white_to_move: false,
        },
        Position {
            white: 60197177344,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 257966999552,
            black: 35184573415424,
            white_to_move: false,
        },
        Position {
            white: 940050432,
            black: 35313221107712,
            white_to_move: false,
        },
        Position {
            white: 137842132992,
            black: 35313221107712,
            white_to_move: false,
        },
        Position {
            white: 2216337868800,
            black: 35296309673984,
            white_to_move: false,
        },
        Position {
            white: 8830587504640,
            black: 35279129804800,
            white_to_move: false,
        },
        Position {
            white: 18049651735791616,
            black: 60397977600,
            white_to_move: false,
        },
        Position {
            white: 18119919616,
            black: 39685497815040,
            white_to_move: false,
        },
        Position {
            white: 258101217280,
            black: 39582418599936,
            white_to_move: false,
        },
        Position {
            white: 8847767373824,
            black: 39651406512128,
            white_to_move: false,
        },
        Position {
            white: 1130315267966976,
            black: 35287719739392,
            white_to_move: false,
        },
        Position {
            white: 18049668915660800,
            black: 4432674684928,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 35253361053696,
            white_to_move: false,
        },
        Position {
            white: 51674882048,
            black: 35253360529408,
            white_to_move: false,
        },
        Position {
            white: 51944357888,
            black: 35253093142528,
            white_to_move: false,
        },
        Position {
            white: 52479131648,
            black: 35253093142528,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 35184642101248,
            white_to_move: false,
        },
        Position {
            white: 18049703274872832,
            black: 270012416,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 35253360263168,
            white_to_move: false,
        },
        Position {
            white: 51541573632,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 51811188736,
            black: 35253226569728,
            white_to_move: false,
        },
        Position {
            white: 257699086336,
            black: 35184775528448,
            white_to_move: false,
        },
        Position {
            white: 17712714612736,
            black: 35184507092992,
            white_to_move: false,
        },
        Position {
            white: 51674351616,
            black: 35253363146752,
            white_to_move: false,
        },
        Position {
            white: 51674882048,
            black: 35253362622464,
            white_to_move: false,
        },
        Position {
            white: 51944374272,
            black: 35253093138432,
            white_to_move: false,
        },
        Position {
            white: 52479131648,
            black: 35253095235584,
            white_to_move: false,
        },
        Position {
            white: 257832255488,
            black: 35184644194304,
            white_to_move: false,
        },
        Position {
            white: 18049703274872832,
            black: 272105472,
            white_to_move: false,
        },
        Position {
            white: 51675400192,
            black: 35253427109888,
            white_to_move: false,
        },
        Position {
            white: 51607896064,
            black: 35253494743040,
            white_to_move: false,
        },
        Position {
            white: 51608551424,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 51811188736,
            black: 35253293416448,
            white_to_move: false,
        },
        Position {
            white: 257699086336,
            black: 35184842375168,
            white_to_move: false,
        },
        Position {
            white: 17712714612736,
            black: 35184573939712,
            white_to_move: false,
        },
        Position {
            white: 135792640,
            black: 35313489543168,
            white_to_move: false,
        },
        Position {
            white: 136052736,
            black: 35313489543168,
            white_to_move: false,
        },
        Position {
            white: 940572672,
            black: 35313221632000,
            white_to_move: false,
        },
        Position {
            white: 2216338391040,
            black: 35296310198272,
            white_to_move: false,
        },
        Position {
            white: 8830588026880,
            black: 35279130329088,
            white_to_move: false,
        },
        Position {
            white: 17661309222912,
            black: 35244502155264,
            white_to_move: false,
        },
        Position {
            white: 18049651736313856,
            black: 60398501888,
            white_to_move: false,
        },
        Position {
            white: 17315661824,
            black: 39685766250496,
            white_to_move: false,
        },
        Position {
            white: 17315921920,
            black: 39685766250496,
            white_to_move: false,
        },
        Position {
            white: 18120441856,
            black: 39685498339328,
            white_to_move: false,
        },
        Position {
            white: 257833304064,
            black: 39582687559680,
            white_to_move: false,
        },
        Position {
            white: 8847767896064,
            black: 39651407036416,
            white_to_move: false,
        },
        Position {
            white: 17678489092096,
            black: 39616778862592,
            white_to_move: false,
        },
        Position {
            white: 1130315268489216,
            black: 35287720263680,
            white_to_move: false,
        },
        Position {
            white: 18049668916183040,
            black: 4432675209216,
            white_to_move: false,
        },
        Position {
            white: 17181704192,
            black: 44083946979328,
            white_to_move: false,
        },
        Position {
            white: 257699086336,
            black: 43980868288512,
            white_to_move: false,
        },
        Position {
            white: 17678354874368,
            black: 44014959591424,
            white_to_move: false,
        },
        Position {
            white: 4512412901310464,
            black: 35287854481408,
            white_to_move: false,
        },
        Position {
            white: 51676448768,
            black: 35253361049600,
            white_to_move: false,
        },
        Position {
            white: 51676979200,
            black: 35253360525312,
            white_to_move: false,
        },
        Position {
            white: 51543539712,
            black: 35253494218752,
            white_to_move: false,
        },
        Position {
            white: 257700134912,
            black: 35184776314880,
            white_to_move: false,
        },
        Position {
            white: 51676448768,
            black: 35253896871936,
            white_to_move: false,
        },
        Position {
            white: 51676971008,
            black: 35253897396224,
            white_to_move: false,
        },
        Position {
            white: 258237005824,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 271057920,
            black: 35313355325440,
            white_to_move: false,
        },
        Position {
            white: 471859200,
            black: 35313221632000,
            white_to_move: false,
        },
        Position {
            white: 4432676782080,
            black: 35278996111360,
            white_to_move: false,
        },
        Position {
            white: 17661176053760,
            black: 35244636372992,
            white_to_move: false,
        },
        Position {
            white: 51676448768,
            black: 35390798954496,
            white_to_move: false,
        },
        Position {
            white: 51676971008,
            black: 35390799478784,
            white_to_move: false,
        },
        Position {
            white: 532578041856,
            black: 35184775266304,
            white_to_move: false,
        },
        Position {
            white: 17450927104,
            black: 44083678543872,
            white_to_move: false,
        },
        Position {
            white: 17585668096,
            black: 44083544850432,
            white_to_move: false,
        },
        Position {
            white: 17651728384,
            black: 44083544850432,
            white_to_move: false,
        },
        Position {
            white: 257968570368,
            black: 43980599853056,
            white_to_move: false,
        },
        Position {
            white: 4449856651264,
            black: 44049319329792,
            white_to_move: false,
        },
        Position {
            white: 17678355922944,
            black: 44014959591424,
            white_to_move: false,
        },
        Position {
            white: 4512413170794496,
            black: 35287586045952,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 35322616348672,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 35322616872960,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 35322348437504,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 35322482655232,
            white_to_move: false,
        },
        Position {
            white: 120800149504,
            black: 35322214219776,
            white_to_move: false,
        },
        Position {
            white: 532575944704,
            black: 35185312137216,
            white_to_move: false,
        },
        Position {
            white: 18049703140655104,
            black: 138379001856,
            white_to_move: false,
        },
        Position {
            white: 240787129344,
            black: 37400709431296,
            white_to_move: false,
        },
        Position {
            white: 240652912640,
            black: 37400843649024,
            white_to_move: false,
        },
        Position {
            white: 240652648448,
            black: 37400844173312,
            white_to_move: false,
        },
        Position {
            white: 240787652608,
            black: 37400709955584,
            white_to_move: false,
        },
        Position {
            white: 240788701184,
            black: 37400709955584,
            white_to_move: false,
        },
        Position {
            white: 266287972352,
            black: 37383798521856,
            white_to_move: false,
        },
        Position {
            white: 9042624144998400,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 18049823399739392,
            black: 2216606302208,
            white_to_move: false,
        },
        Position {
            white: 223607260160,
            black: 39616912556032,
            white_to_move: false,
        },
        Position {
            white: 223472779264,
            black: 39617047298048,
            white_to_move: false,
        },
        Position {
            white: 223742001152,
            black: 39616778862592,
            white_to_move: false,
        },
        Position {
            white: 1130521291653120,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 9042606965129216,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 18049806219870208,
            black: 4432809426944,
            white_to_move: false,
        },
        Position {
            white: 223607260160,
            black: 44014959067136,
            white_to_move: false,
        },
        Position {
            white: 223472779264,
            black: 44015093809152,
            white_to_move: false,
        },
        Position {
            white: 223742001152,
            black: 44014825373696,
            white_to_move: false,
        },
        Position {
            white: 1134919338164224,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 4512619058692096,
            black: 35219135004672,
            white_to_move: false,
        },
        Position {
            white: 9042606965129216,
            black: 8830855938048,
            white_to_move: false,
        },
        Position {
            white: 18049806219870208,
            black: 8830855938048,
            white_to_move: false,
        },
        Position {
            white: 120393828352,
            black: 105690823655424,
            white_to_move: false,
        },
        Position {
            white: 120393564160,
            black: 105690824179712,
            white_to_move: false,
        },
        Position {
            white: 120662786048,
            black: 105690555744256,
            white_to_move: false,
        },
        Position {
            white: 120529616896,
            black: 105690689961984,
            white_to_move: false,
        },
        Position {
            white: 532575944704,
            black: 105553519443968,
            white_to_move: false,
        },
        Position {
            white: 18049703140655104,
            black: 70506586308608,
            white_to_move: false,
        },
    ];
    println!(
        "Evaluating engine performance over {} positions",
        queue.len()
    );
    let mut total: u64 = 0;
    let now = SystemTime::now();
    for pos in queue {
        total += evaluate_position(depth, pos);
    }
    println!(
        "Evaluated {} nodes over {} ms",
        total,
        now.elapsed().unwrap().as_millis()
    );
    return 0;
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
                    (nxt_move, eval) = search_iterative(
                        white,
                        black,
                        white_to_move,
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
            (nxt_move, eval) = search_iterative(
                white,
                black,
                white_to_move,
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
                        (nxt_move, eval) = search_iterative(
                            white,
                            black,
                            white_to_move,
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
    } else if args.benchmark {
        benchmark(args.search_depth);
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
