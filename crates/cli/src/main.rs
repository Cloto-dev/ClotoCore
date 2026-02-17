mod cli;
mod client;
mod commands;
mod config;
mod error;
mod output;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    if let Err(e) = commands::dispatch(cli).await {
        error::display_error(&e);
        std::process::exit(1);
    }
}
