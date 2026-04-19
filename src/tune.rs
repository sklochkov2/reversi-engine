//! Eval-coefficient tuning via head-to-head self-play.
//!
//! Structure:
//!   - [`generate_ply_positions`] reproduces the symmetry-reduced 6-ply
//!     starting set used by `compare_configs`, as a `Vec<Position>`.
//!   - [`split_positions`] deterministically partitions that set into
//!     disjoint train / validation halves via a splitmix64 hash of the
//!     (white, black, side) triple, so train/val membership is stable
//!     across runs and doesn't depend on insertion order.
//!   - [`run_match`] plays a two-colour head-to-head over a cached
//!     position slice and returns the aggregate score (+2 per win, -2
//!     per loss, 0 per draw, summed over both colour assignments).
//!   - [`tune_eval`] drives a (1 + 1)-ES with a 1/5-success-rule
//!     step-size schedule on the integer-valued [`EvalCfg`] space.
//!     Each evaluation is one match against the *current incumbent*,
//!     which gives a well-defined fitness (candidate score minus
//!     incumbent score = 0 when they're identical, positive when the
//!     candidate wins). Unlike gradient methods this handles
//!     non-smooth integer spaces natively and needs only a handful of
//!     hyperparameters.
//!
//! TT note: `run_match` clears the shared transposition table at the
//! top of each match. Within a match both configs share the TT, which
//! is a mild but harmless noise source - entries are bound-correct
//! in isolation, but cached scores encode the eval of whichever
//! config stored them, so cross-config probes can return off-by-a-bit
//! values. Empirically the effect is dwarfed by the true fitness
//! signal and keeping TT sharing is essential for tuning-loop
//! throughput. If noise becomes a concern, switch to a per-thread or
//! per-config TT.

use rayon::prelude::*;
use reversi_tools::position::{apply_move, check_game_status};
use std::collections::HashMap;

use crate::engine::{find_legal_moves_alt, search_moves_opt, EvalCfg};
use crate::openingbook::{
    flip_position_horizontal, flip_position_vertical, rotate_position_90, Position,
};
use crate::tt;
use crate::utils::splitmix64;

// --------------------------------------------------------------------------
// Position generation (shared with compare_configs)
// --------------------------------------------------------------------------

/// Enumerate all distinct Reversi positions reachable in exactly `ply`
/// plies from the initial position, deduplicated by the 8-way symmetry
/// group (4 rotations × horizontal flip). This is the same set that
/// `compare_configs` uses internally, factored out so the tuner can
/// share one generation pass across hundreds of match evaluations.
pub fn generate_ply_positions(ply: u32) -> Vec<Position> {
    let black = 0x0000000810000000u64;
    let white = 0x0000001008000000u64;
    let starting_pos: Position = Position {
        black,
        white,
        white_to_move: false,
    };
    let mut queue: Vec<Position> = Vec::new();
    let mut dedup_cache: HashMap<Position, bool> = HashMap::new();
    queue.push(starting_pos);
    for _ in 0..ply {
        let mut next_queue: Vec<Position> = Vec::new();
        for pos in queue {
            if dedup_cache.contains_key(&pos) {
                continue;
            }
            let next_moves = find_legal_moves_alt(pos.white, pos.black, pos.white_to_move);
            for next_move in next_moves {
                let new_pos_opt = apply_move(pos.white, pos.black, next_move, pos.white_to_move);
                if let Ok((w, b)) = new_pos_opt {
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
            }
        }
        queue = next_queue;
    }
    queue
}

// --------------------------------------------------------------------------
// Deterministic train / validation split
// --------------------------------------------------------------------------

#[inline]
fn position_hash(p: &Position) -> u64 {
    // Mixes white/black bitboards and side-to-move through two
    // independent splitmix stages so train/val membership is stable
    // across runs and has no correlation with the enumeration order.
    let a = splitmix64(p.white);
    let b = splitmix64(p.black);
    let c = if p.white_to_move { 1u64 } else { 0u64 };
    splitmix64(a ^ b.rotate_left(17) ^ c.wrapping_mul(0xD6E8_FEB8_6659_FD93))
}

/// Deterministically split `positions` into `(train, val)` where
/// roughly `train_frac` of entries go to `train`. Uses a splitmix64
/// hash of each position so the split is stable across runs.
pub fn split_positions(positions: &[Position], train_frac: f64) -> (Vec<Position>, Vec<Position>) {
    let threshold = (train_frac.clamp(0.0, 1.0) * (u64::MAX as f64)) as u64;
    let mut train = Vec::with_capacity(positions.len());
    let mut val = Vec::with_capacity(positions.len());
    for p in positions {
        if position_hash(p) < threshold {
            train.push(*p);
        } else {
            val.push(*p);
        }
    }
    (train, val)
}

// --------------------------------------------------------------------------
// Silent head-to-head match
// --------------------------------------------------------------------------

/// Play each position in `positions` twice (once with `candidate` as
/// black, once as white) against `incumbent`, summing per-game
/// outcomes. Returns the candidate-minus-incumbent score:
///   +2 per candidate win, -2 per candidate loss, 0 per draw.
///
/// Positive values mean the candidate outplayed the incumbent.
pub fn run_match(
    candidate: EvalCfg,
    incumbent: EvalCfg,
    depth: u32,
    positions: &[Position],
) -> i32 {
    // Clear the TT once at the start so stale entries from a prior
    // match don't bias the first positions of this one. Within-match
    // sharing between the two configs is tolerated (see module
    // comment).
    tt::tt().clear();

    positions
        .par_iter()
        .map(|pos| {
            // candidate = black, incumbent = white
            let a = play_game_from_position_silent(candidate, incumbent, depth, *pos);
            // candidate = white, incumbent = black -> flip sign so the
            // return value is always in the candidate's favour when
            // positive.
            let b = -play_game_from_position_silent(incumbent, candidate, depth, *pos);
            2 * (a + b)
        })
        .sum()
}

/// Single-game self-play between two EvalCfgs. Returns +1 if black
/// wins, -1 if white wins, 0 if draw. `black_cfg` is used when it's
/// black's turn, `white_cfg` on white's. No I/O.
fn play_game_from_position_silent(
    black_cfg: EvalCfg,
    white_cfg: EvalCfg,
    depth: u32,
    pos: Position,
) -> i32 {
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
            BLACK_WON => return 1,
            WHITE_WON => return -1,
            DRAWN_GAME => return 0,
            _ => {
                let curr_cfg = if white_to_move { white_cfg } else { black_cfg };
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
                    Err(_) => return 0,
                }
            }
        }
    }
}

// --------------------------------------------------------------------------
// (1+1)-ES with 1/5-success rule
// --------------------------------------------------------------------------

/// Number of tunable scalar parameters in [`EvalCfg`]. Matches the
/// field enumeration in [`cfg_to_vec`] / [`vec_to_cfg`]; bumping
/// this requires updating both marshalers and the parser in
/// `main.rs::parse_coefs_or_default`.
pub const TUNE_DIM: usize = 10;

/// Marshal [`EvalCfg`] to/from a fixed-length `f64` vector so the
/// optimizer can work in a uniform parameter space. Parameter order:
/// corner, edge, antiedge, anticorner, disc[opening],
/// disc[midgame], disc[endgame], mobility[opening],
/// mobility[midgame], mobility[endgame].
fn cfg_to_vec(cfg: &EvalCfg) -> [f64; TUNE_DIM] {
    [
        cfg.corner_value as f64,
        cfg.edge_value as f64,
        cfg.antiedge_value as f64,
        cfg.anticorner_value as f64,
        cfg.disc_values[0] as f64,
        cfg.disc_values[1] as f64,
        cfg.disc_values[2] as f64,
        cfg.mobility_values[0] as f64,
        cfg.mobility_values[1] as f64,
        cfg.mobility_values[2] as f64,
    ]
}

fn vec_to_cfg(v: &[f64; TUNE_DIM]) -> EvalCfg {
    EvalCfg {
        corner_value: v[0].round() as i32,
        edge_value: v[1].round() as i32,
        antiedge_value: v[2].round() as i32,
        anticorner_value: v[3].round() as i32,
        disc_values: [
            v[4].round() as i32,
            v[5].round() as i32,
            v[6].round() as i32,
        ],
        mobility_values: [
            v[7].round() as i32,
            v[8].round() as i32,
            v[9].round() as i32,
        ],
    }
}

/// Tiny uniform RNG built on splitmix64. Sufficient for generating
/// perturbation vectors; we don't need cryptographic randomness or a
/// high-quality distribution here.
struct Rng64(u64);

impl Rng64 {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero fixed point of some LCG-family PRNGs by
        // forcing a non-zero mix of the seed.
        Self(splitmix64(seed ^ 0xA5A5_5A5A_DEAD_BEEF))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = splitmix64(self.0);
        self.0
    }

    /// Uniform in [-1.0, 1.0).
    fn unit(&mut self) -> f64 {
        // 53-bit precision mantissa from the upper bits, mapped to
        // [0,1), then shifted to [-1,1).
        let x = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        2.0 * x - 1.0
    }
}

/// Run the tuning loop. Starts from `initial`, evaluates every
/// candidate via [`run_match`] on `train_positions`, and returns the
/// best config found (validated against `val_positions`). `depth` is
/// the per-move search depth used during tuning games.
///
/// The algorithm is a plain (1+1)-ES: each iteration generates one
/// Gaussian-ish offspring of the current incumbent with standard
/// deviation `sigma`, evaluates it via head-to-head, and replaces the
/// incumbent if the offspring wins (train score > 0). `sigma` is
/// adapted with the 1/5-success rule: if the moving-window success
/// rate exceeds 1/5 the step is widened; if it drops below, the step
/// is narrowed. This converges in O(100) iterations for 4 integer
/// dimensions with coefficient magnitudes around ±100.
pub fn tune_eval(
    initial: EvalCfg,
    train_positions: &[Position],
    val_positions: &[Position],
    depth: u32,
    iterations: u32,
    seed: u64,
    initial_sigma: f64,
) -> EvalCfg {
    let mut rng = Rng64::new(seed);
    let mut incumbent_vec = cfg_to_vec(&initial);
    let mut incumbent = initial;
    let mut sigma = initial_sigma;

    const WINDOW: u32 = 10;
    const TARGET_SUCCESS: f64 = 0.2; // 1/5 rule
    const SIGMA_MIN: f64 = 0.75;
    const SIGMA_MAX: f64 = 40.0;
    const SIGMA_UP: f64 = 1.4;
    const SIGMA_DOWN: f64 = 1.0 / 1.4;

    let mut window_successes: u32 = 0;
    let mut total_accepted: u32 = 0;

    println!(
        "tune: starting from {:?}, depth={}, train={}, val={}, iterations={}, seed={}, sigma0={}",
        initial,
        depth,
        train_positions.len(),
        val_positions.len(),
        iterations,
        seed,
        initial_sigma
    );

    for iter in 1..=iterations {
        // Sample offspring: incumbent + sigma * uniform perturbation
        // in each dimension. Uniform is simpler than Gaussian for
        // integer-rounded parameters and has bounded support that
        // avoids runaway steps near the coefficient limits.
        let mut offspring_vec = incumbent_vec;
        for x in offspring_vec.iter_mut() {
            *x += sigma * rng.unit();
        }
        let offspring = vec_to_cfg(&offspring_vec);

        // Short-circuit if rounding collapsed the offspring onto the
        // incumbent: no information to gain from a null match.
        if offspring == incumbent {
            continue;
        }

        let score = run_match(offspring, incumbent, depth, train_positions);
        let accepted = score > 0;

        if accepted {
            incumbent_vec = offspring_vec;
            incumbent = offspring;
            window_successes += 1;
            total_accepted += 1;
        }

        println!(
            "tune iter {:4}/{:4}: sigma={:5.2} offspring={:?} match_score={:+5} {} incumbent={:?}",
            iter,
            iterations,
            sigma,
            offspring,
            score,
            if accepted { "ACCEPT" } else { "reject" },
            incumbent,
        );

        // 1/5 rule: adjust sigma every WINDOW iterations based on
        // success rate. This keeps the step size roughly matched to
        // the local curvature without requiring gradient estimates.
        if iter % WINDOW == 0 {
            let rate = window_successes as f64 / WINDOW as f64;
            let old_sigma = sigma;
            if rate > TARGET_SUCCESS {
                sigma = (sigma * SIGMA_UP).min(SIGMA_MAX);
            } else if rate < TARGET_SUCCESS {
                sigma = (sigma * SIGMA_DOWN).max(SIGMA_MIN);
            }
            if (sigma - old_sigma).abs() > 0.01 {
                println!(
                    "tune: window rate={:.2} -> sigma {:.2} -> {:.2} (accepted {}/{})",
                    rate, old_sigma, sigma, total_accepted, iter
                );
            }
            window_successes = 0;
        }
    }

    // Validation pass: match the best config found against the
    // starting config on the held-out set at the same depth. Accept
    // only if the tuned config wins on val too, otherwise report the
    // finding and return the original config to avoid regressing into
    // a local optimum that doesn't generalise.
    if incumbent == initial {
        println!(
            "\ntune: no accepted moves, skipping validation (tuned == initial)"
        );
        return initial;
    }

    println!(
        "\ntune: accepted {}/{} moves, running validation match (tuned vs initial) on {} held-out positions at depth {}",
        total_accepted,
        iterations,
        val_positions.len(),
        depth
    );
    let val_score = run_match(incumbent, initial, depth, val_positions);
    println!(
        "tune: validation score (tuned vs initial) = {:+}",
        val_score
    );

    if val_score > 0 {
        println!("tune: tuned config wins on validation, adopting: {:?}", incumbent);
        incumbent
    } else {
        println!(
            "tune: tuned config failed validation (score {:+}); reverting to initial",
            val_score
        );
        initial
    }
}
