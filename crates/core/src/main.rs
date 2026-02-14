use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = exiv_core::cli::Cli::parse();

    match cli.command {
        None => {
            // Default: load .env and run kernel (backward compatible)
            if dotenvy::dotenv().is_err() {
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(dir) = exe.parent() {
                        let _ = dotenvy::from_path(dir.join(".env"));
                    }
                }
            }
            tracing_subscriber::fmt::init();
            exiv_core::run_kernel().await
        }
        Some(cmd) => {
            tracing_subscriber::fmt::init();
            exiv_core::cli::dispatch(cmd).await
        }
    }
}
