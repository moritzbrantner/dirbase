use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Serve all JSON files in a folder as a REST API"
)]
pub struct Cli {
    /// Folder containing .json files
    #[arg(short, long, default_value = "./data")]
    pub folder: PathBuf,

    /// Bind address, e.g. 127.0.0.1:3000
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    pub bind: SocketAddr,

    /// Enable read-only mode (only GET endpoints are exposed)
    #[arg(long)]
    pub readonly: bool,
}
