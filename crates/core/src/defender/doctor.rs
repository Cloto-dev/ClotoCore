//! `clotocore doctor` — pool-free, read-only installation diagnosis
//! (DEFENDER_DESIGN.md §4).
//!
//! The single most valuable diagnosis — "the DB file is corrupt and the
//! kernel will fatal on boot" (SQLITE_NOTADB, code 26) — must work when the
//! kernel *cannot* boot, so doctor never opens anything through sqlx.
//! Pool-dependent checks are skipped with an explicit marker instead.

use std::path::Path;

use crate::db::health::HealthStatus;
use crate::defender::checks::{run_scan, CheckCtx};

/// Run the doctor scan and print a report. Read-only. Returns an error (→
/// non-zero exit) when any check reports `Error`.
pub async fn run(json: bool, offline: bool) -> anyhow::Result<()> {
    let data_dir = crate::config::data_dir();
    let servers_dir = crate::managers::mcp_venv::resolve_venv_dir()
        .and_then(|v| v.parent().map(Path::to_path_buf));
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8081);

    let ctx = CheckCtx {
        pool: None,
        data_dir: data_dir.clone(),
        servers_dir,
        in_kernel: false,
        port,
        offline,
    };
    let outcome = run_scan(&ctx).await;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "report": outcome.report,
                "skipped": outcome.skipped,
                "data_dir": data_dir.display().to_string(),
            }))?
        );
    } else {
        println!("ClotoCore doctor v{}", env!("CARGO_PKG_VERSION"));
        println!("Data directory: {}", data_dir.display());
        println!();
        for check in &outcome.report.checks {
            let icon = match check.status {
                HealthStatus::Healthy => "✓",
                HealthStatus::Degraded => "⚠",
                HealthStatus::Error => "✗",
            };
            println!("  {icon} {:<28} {}", check.name, check.message);
        }
        if !outcome.skipped.is_empty() {
            println!();
            println!(
                "  – skipped (kernel not reachable / not applicable): {}",
                outcome.skipped.join(", ")
            );
        }
        println!();
        let errors = count_status(&outcome, HealthStatus::Error);
        let degraded = count_status(&outcome, HealthStatus::Degraded);
        match outcome.report.status {
            HealthStatus::Healthy => println!("Overall: healthy"),
            HealthStatus::Degraded => println!("Overall: degraded ({degraded} warning(s))"),
            HealthStatus::Error => {
                println!("Overall: ERROR ({errors} error(s), {degraded} warning(s))");
            }
        }
        println!("(read-only scan — nothing was modified)");
    }

    if outcome.report.status == HealthStatus::Error {
        anyhow::bail!("doctor found error-level issues");
    }
    Ok(())
}

fn count_status(outcome: &crate::defender::checks::ScanOutcome, status: HealthStatus) -> usize {
    outcome
        .report
        .checks
        .iter()
        .filter(|c| c.status == status)
        .count()
}
