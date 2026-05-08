//! `cloto` — administrative CLI for ClotoCore.
//!
//! Currently provides Magic Seal management (MGP_ISOLATION_DESIGN.md §8 L0).
//! Exists as a separate binary from `cloto_system` so the operator-facing
//! `cloto seal …` invocation matches the spec verbatim.
//!
//! Exit codes:
//! - 0: success / seal verified
//! - 1: seal verification mismatch (`verify` only)
//! - 2: I/O, format, or other error

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use cloto_core::managers::mcp_seal;

const DEFAULT_DATA_DIR: &str = "data";

#[derive(Parser)]
#[command(
    name = "cloto",
    version = env!("CARGO_PKG_VERSION"),
    about = "ClotoCore administrative CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Magic Seal management (MGP_ISOLATION_DESIGN.md §8 L0).
    Seal {
        #[command(subcommand)]
        action: SealAction,
    },
}

#[derive(Subcommand)]
enum SealAction {
    /// Compute a Magic Seal for a server entry point and print it to stdout.
    ///
    /// stdout contains exactly one line: the `sha256:HEX` seal value, suitable
    /// for pasting into `mcp.toml`'s `seal = "..."` field. Diagnostics go to
    /// stderr.
    Generate {
        /// Path to the server entry point (e.g. `mcp-servers/cpersona/server.py`).
        #[arg(long)]
        entry_point: PathBuf,

        /// Server identifier — recorded in stderr only, for log readability.
        #[arg(long)]
        server_id: String,

        /// Override the data directory used to resolve `seal.key`.
        /// Defaults to `./data`. Honors `CLOTO_SEAL_KEY` env var first
        /// (see `load_or_generate_seal_key` resolution order).
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Verify a stored Magic Seal against a server entry point.
    ///
    /// Exit 0 = seal matches the file's HMAC.
    /// Exit 1 = seal mismatches — the file may have been tampered with.
    /// Exit 2 = I/O error or malformed seal value.
    ///
    /// If `--seal` is omitted, the CLI computes the current seal and prints
    /// it instead of comparing — handy for "what *should* this be?" lookups.
    Verify {
        #[arg(long)]
        entry_point: PathBuf,

        #[arg(long)]
        server_id: String,

        /// Expected seal in `sha256:HEX` form. If omitted, the current seal
        /// is computed and printed (no verification is performed).
        #[arg(long)]
        seal: Option<String>,

        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Seal { action } => run_seal(action),
    }
}

fn run_seal(action: SealAction) -> ExitCode {
    match action {
        SealAction::Generate {
            entry_point,
            server_id,
            data_dir,
        } => generate(&entry_point, &server_id, data_dir.as_deref()),

        SealAction::Verify {
            entry_point,
            server_id,
            seal,
            data_dir,
        } => verify(
            &entry_point,
            &server_id,
            seal.as_deref(),
            data_dir.as_deref(),
        ),
    }
}

fn generate(
    entry_point: &std::path::Path,
    server_id: &str,
    data_dir: Option<&std::path::Path>,
) -> ExitCode {
    let dir = data_dir.map_or_else(
        || PathBuf::from(DEFAULT_DATA_DIR),
        std::path::Path::to_path_buf,
    );
    let key = match mcp_seal::load_or_generate_seal_key(&dir) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: failed to load seal key from {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    };
    let seal = match mcp_seal::compute_seal(entry_point, &key) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to seal {}: {e}", entry_point.display());
            return ExitCode::from(2);
        }
    };
    println!("{seal}");
    eprintln!(
        "ok: server_id={server_id} entry_point={}",
        entry_point.display()
    );
    ExitCode::SUCCESS
}

fn verify(
    entry_point: &std::path::Path,
    server_id: &str,
    expected: Option<&str>,
    data_dir: Option<&std::path::Path>,
) -> ExitCode {
    let dir = data_dir.map_or_else(
        || PathBuf::from(DEFAULT_DATA_DIR),
        std::path::Path::to_path_buf,
    );
    let key = match mcp_seal::load_or_generate_seal_key(&dir) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: failed to load seal key from {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    };

    let Some(expected) = expected else {
        // No expected value provided — print the current seal as a hint.
        match mcp_seal::compute_seal(entry_point, &key) {
            Ok(s) => {
                println!("{s}");
                eprintln!(
                    "info: --seal not provided; printed the current seal value above. \
                     server_id={server_id} entry_point={}",
                    entry_point.display()
                );
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("error: failed to compute seal: {e}");
                return ExitCode::from(2);
            }
        }
    };

    match mcp_seal::verify_seal(entry_point, expected, &key) {
        Ok(true) => {
            eprintln!(
                "ok: seal verified — server_id={server_id} entry_point={}",
                entry_point.display()
            );
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!(
                "FAIL: seal mismatch — server_id={server_id} entry_point={} (binary may be tampered)",
                entry_point.display()
            );
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
