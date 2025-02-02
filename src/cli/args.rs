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
}
