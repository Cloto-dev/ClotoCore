use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cloto_core::cli::Cli::parse();

    match cli.command {
        None => {
            // Default: load .env and run kernel (backward compatible).
            // Sample the environment first: after the load, a key that came
            // from the environment is indistinguishable from one that came
            // from the file, and a rotation needs to tell them apart.
            cloto_core::apikey::note_env_key_before_dotenv();
            if dotenvy::dotenv().is_err() {
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(dir) = exe.parent() {
                        let _ = dotenvy::from_path(dir.join(".env"));
                    }
                }
            }
            // Same last resort as the desktop boot: a key persisted into the
            // user-data dir is where `apikey::resolve_env_target` writes when
            // no other `.env` exists, so this is the read side of that write.
            // Without it a rotation on a headless install writes a file the
            // next boot never opens.
            if std::env::var("CLOTO_API_KEY").is_err() {
                let _ = dotenvy::from_path(cloto_core::config::data_dir().join(".env"));
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
