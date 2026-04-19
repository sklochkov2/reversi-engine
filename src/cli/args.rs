use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// API base URL, e. g. http://example.com:8080/
    #[arg(short, long, default_value_t = String::new())]
    pub api_url: String,

    /// Player UUID as provided by server API
    #[arg(short, long, default_value_t = String::new())]
    pub player_uuid: String,

    /// Search depth
    #[arg(short, long, default_value_t = 8)]
    pub search_depth: u32,

    /// Opening book path
    #[arg(short, long, default_value_t = String::new())]
    pub book_path: String,

    /// Whether to generate an opening book
    #[arg(short, long, default_value_t = false)]
    pub generate_book: bool,

    /// Compare two eval settings
    #[arg(short, long, default_value_t = false)]
    pub compare_configs: bool,

    /// When generating an opening book, how deeply to evaluate all moves
    #[arg(short, long, default_value_t = 5)]
    pub full_depth: u32,

    #[arg(short, long, default_value_t = 7)]
    /// When generating an opening book, how deeply to analyze main lines
    pub k_partial_depth: u32,

    /// Run a benchmark for performance evaluation and profiling purposes.
    #[arg(short, long, default_value_t = false)]
    pub benchmark: bool,

    /// Run a late-game benchmark: each base position is rolled forward
    /// into endgame territory (~18 empties) before being searched. Exercises
    /// the exact endgame solver which never fires on the default fixture.
    #[arg(long, default_value_t = false)]
    pub benchmark_endgame: bool,

    /// Number of base benchmark positions to roll forward for
    /// `--benchmark-endgame`. The full 2315-position fixture takes ~minutes
    /// to prepare; subsets of 50-200 give a representative sample.
    #[arg(long, default_value_t = 100)]
    pub benchmark_endgame_positions: usize,

    /// Target empty-square count for rolled-forward endgame benchmark
    /// positions. Lower = deeper into the solver's territory.
    #[arg(long, default_value_t = 18)]
    pub benchmark_endgame_empties: u32,

    /// Run the eval-coefficient tuner: a (1+1)-ES that plays
    /// head-to-head matches against the current incumbent config on
    /// the symmetry-reduced 6-ply position set and keeps accepted
    /// improvements. Held-out validation is performed at the end.
    #[arg(long, default_value_t = false)]
    pub tune_eval: bool,

    /// Tuner: number of ES iterations (each iteration = one
    /// head-to-head match, two games per position). 100-200 is
    /// typical for a 4-dim parameter space.
    #[arg(long, default_value_t = 150)]
    pub tune_iterations: u32,

    /// Tuner: random seed for the perturbation PRNG. Identical seeds
    /// produce identical trajectories given deterministic search.
    #[arg(long, default_value_t = 42)]
    pub tune_seed: u64,

    /// Tuner: fraction of generated positions used for training. The
    /// complement is held back for the final validation match that
    /// gates adoption of the tuned config.
    #[arg(long, default_value_t = 0.7)]
    pub tune_train_frac: f64,

    /// Tuner: initial step size (standard deviation of the uniform
    /// per-dim perturbation, before rounding). Widened/narrowed at
    /// run-time by the 1/5-success rule.
    #[arg(long, default_value_t = 12.0)]
    pub tune_sigma: f64,

    /// Tuner: ply depth used for position generation. Lower = fewer
    /// positions and faster iterations; higher = more representative
    /// midgame positions but exponential cost. 6 matches the default
    /// `compare_configs` set size.
    #[arg(long, default_value_t = 6)]
    pub tune_ply: u32,

    /// Tuner: override the starting coefficients
    /// (comma-separated `corner,edge,antiedge,anticorner`). Defaults
    /// to the built-in `DEFAULT_CFG` when not provided.
    #[arg(long, default_value_t = String::new())]
    pub tune_initial_coefs: String,

    /// Play a single head-to-head match between
    /// `--tune-initial-coefs` and the built-in `DEFAULT_CFG` at
    /// `--search-depth`, using the same symmetry-reduced position
    /// set (at `--tune-ply` plies) as the tuner. Skips optimisation
    /// entirely - just reports the match score. Useful for
    /// validating a candidate config at a higher depth than tuning
    /// was done at.
    #[arg(long, default_value_t = false)]
    pub validate_match: bool,
}
