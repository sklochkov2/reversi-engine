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
}
