use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cloto_core::cli::Cli::parse();

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
            cloto_core::init_tracing();
            cloto_core::run_kernel().await
        }
        Some(cmd) => {
            cloto_core::init_tracing();
            cloto_core::cli::dispatch(cmd).await
        }
    }
}
