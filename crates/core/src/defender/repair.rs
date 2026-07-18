//! Repair verb (DEFENDER_DESIGN.md §2, Phase 2).
//!
//! `repair` fixes `fix_capable` findings **non-destructively**: regeneration,
//! ledger correction, and removal of ClotoCore's own transient artifacts
//! (interrupted-update remnants). It has no code path that deletes user data
//! — that invariant is structural, not conventional (§8.1). Destructive
//! cleanup is the purge verb (Phase 3), reachable only through the explicit
//! uninstall flow.
//!
//! Deliberately NOT repaired here: legacy data-dir drift. A stray-looking
//! data dir can be the *active* dir of a coexisting install (dev layout and
//! a production desktop install on the same machine), so it stays
//! report-only until the Phase 3 purge plan makes removal explicit and
//! user-approved.

use std::path::Path;

use crate::db::health::{RepairAction, RepairReport};
use crate::defender::checks::CheckCtx;
use crate::defender::footprint::{self, ReceiptEntry};

/// Run every applicable repair against `ctx`. Superset of the historical
/// `run_full_repair` (orphaned DB rows + venv rebuild), preserving the
/// response shape. Pool-dependent repairs are skipped without a pool.
pub async fn run_repair(ctx: &CheckCtx) -> anyhow::Result<RepairReport> {
    let mut actions = Vec::new();

    // ── Absorbed DB repairs (pool required) ──
    if let Some(pool) = &ctx.pool {
        let count = crate::db::health::repair_orphaned_chat_messages(pool).await?;
        if count > 0 {
            actions.push(RepairAction {
                name: "orphaned_chat_messages".into(),
                fixed_count: count,
                message: format!("Deleted {count} orphaned chat message(s)"),
            });
        }
        let count = crate::db::health::repair_orphaned_trusted_commands(pool).await?;
        if count > 0 {
            actions.push(RepairAction {
                name: "orphaned_trusted_commands".into(),
                fixed_count: count,
                message: format!("Deleted {count} orphaned trusted command(s)"),
            });
        }
        let count = crate::db::health::repair_orphaned_permission_requests(pool).await?;
        if count > 0 {
            actions.push(RepairAction {
                name: "orphaned_permission_requests".into(),
                fixed_count: count,
                message: format!("Deleted {count} orphaned permission request(s)"),
            });
        }
    }

    // ── Absorbed venv repair ──
    if let Some(dir) = ctx.servers_dir.as_deref() {
        if let Some(action) = crate::db::health::repair_venv(dir, &ctx.data_dir).await {
            actions.push(action);
        }
    }

    // ── Swap remnant cleanup (Phase 2) ──
    if let Ok(exe) = std::env::current_exe() {
        let removed = cleanup_swap_remnants_at(&exe);
        if !removed.is_empty() {
            actions.push(RepairAction {
                name: "swap_remnant_cleanup".into(),
                fixed_count: removed.len(),
                message: format!(
                    "Removed {} interrupted-update remnant(s): {}",
                    removed.len(),
                    removed.join(", ")
                ),
            });
        }
    }

    // ── Receipt sync (Phase 2) ──
    let sync = sync_receipt(&ctx.data_dir);
    if sync.adopted > 0 || sync.dropped > 0 {
        actions.push(RepairAction {
            name: "receipt_sync".into(),
            fixed_count: sync.adopted + sync.dropped,
            message: format!(
                "Install receipt synced: adopted {} pre-receipt MCP server dir(s), \
                 dropped {} stale entr(ies)",
                sync.adopted, sync.dropped
            ),
        });
    }

    let total_fixed = actions.iter().map(|a| a.fixed_count).sum();
    Ok(RepairReport {
        actions,
        total_fixed,
    })
}

/// Remove `.new` / `.old` update-swap remnants next to `exe`. These are
/// ClotoCore's own transient artifacts (an interrupted download or a
/// superseded rollback backup), never user data. Returns the removed paths;
/// failures are logged and skipped.
#[must_use]
pub fn cleanup_swap_remnants_at(exe: &Path) -> Vec<String> {
    let mut removed = Vec::new();
    for ext in ["new", "old", "old.exe"] {
        let candidate = exe.with_extension(ext);
        if !candidate.is_file() {
            continue;
        }
        match std::fs::remove_file(&candidate) {
            Ok(()) => removed.push(candidate.display().to_string()),
            Err(e) => {
                tracing::warn!("Could not remove swap remnant {}: {e}", candidate.display());
            }
        }
    }
    removed
}

pub struct ReceiptSync {
    /// Pre-receipt MCP server dirs adopted into the ledger.
    pub adopted: usize,
    /// Entries whose recorded path no longer exists, dropped from the ledger.
    pub dropped: usize,
}

/// Converge the install receipt toward on-disk reality: adopt MCP server
/// dirs the ledger does not know about (pre-receipt installs) and drop
/// entries whose paths are gone. Ledger correction only — no file is
/// touched.
#[must_use]
pub fn sync_receipt(data_dir: &Path) -> ReceiptSync {
    // Ensure a receipt exists so pre-receipt installs can be adopted at all.
    if footprint::load(data_dir).is_none() {
        footprint::record(data_dir, footprint::boot_entries(data_dir));
    }
    let Some(receipt) = footprint::load(data_dir) else {
        // Receipt still unwritable (read-only data dir?) — nothing to sync.
        return ReceiptSync {
            adopted: 0,
            dropped: 0,
        };
    };

    // Drop stale entries (recorded path no longer exists).
    let stale: Vec<String> = receipt
        .entries
        .iter()
        .filter(|e| e.path.as_ref().is_some_and(|p| !Path::new(p).exists()))
        .map(|e| e.id.clone())
        .collect();
    for id in &stale {
        footprint::remove(data_dir, id);
    }

    // Adopt unrecorded MCP server dirs.
    let servers_root = data_dir.join("mcp-servers");
    let mut adopt = Vec::new();
    if let Ok(read) = std::fs::read_dir(&servers_root) {
        for dir in read.flatten() {
            let file_name = dir.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') || !dir.path().is_dir() {
                continue;
            }
            let recorded = receipt
                .entries
                .iter()
                .any(|e| e.path.as_ref().is_some_and(|p| Path::new(p) == dir.path()));
            if !recorded {
                adopt.push(ReceiptEntry::dir(format!("mcp:{file_name}"), &dir.path()));
            }
        }
    }
    let adopted = adopt.len();
    if adopted > 0 {
        footprint::record(data_dir, adopt);
    }

    ReceiptSync {
        adopted,
        dropped: stale.len(),
    }
}

/// Clean-update first-boot phase (DEFENDER_DESIGN.md §6): called when the
/// booting binary's version differs from the one that last wrote the
/// receipt. Runs the non-destructive convergence only — the `.old` rollback
/// backup next to the exe is deliberately left in place (it IS the
/// backup-first quarantine; the scan reports it as safe to clean and the
/// explicit repair verb removes it).
pub fn first_boot_maintenance(data_dir: &Path, previous_version: &str) {
    tracing::info!(
        "🩺 First boot of v{} (receipt was written by v{previous_version}) — \
         running receipt convergence",
        env!("CARGO_PKG_VERSION")
    );
    let sync = sync_receipt(data_dir);
    if sync.adopted > 0 || sync.dropped > 0 {
        tracing::info!(
            "Install receipt converged: {} adopted, {} stale dropped",
            sync.adopted,
            sync.dropped
        );
    }
}

/// `clotocore repair` — file-level repairs that work without the kernel
/// (swap remnants, receipt sync, venv rebuild). DB-level repairs need the
/// kernel's pool and run via the dashboard / `POST /api/health/repair`.
pub async fn run_cli(json: bool) -> anyhow::Result<()> {
    let data_dir = crate::config::data_dir();
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8081);

    // A live kernel owns the venv (running MCP servers) — rebuilding it out
    // from under the kernel is disruptive, so defer venv repair to the
    // in-kernel repair endpoint in that case.
    let kernel_running = matches!(
        crate::defender::checks::probe_kernel_version(port).await,
        crate::defender::checks::KernelProbe::Kernel(_)
    );
    let servers_dir = if kernel_running {
        None
    } else {
        crate::managers::mcp_venv::resolve_venv_dir()
            .and_then(|v| v.parent().map(Path::to_path_buf))
    };

    let ctx = CheckCtx {
        pool: None,
        data_dir: data_dir.clone(),
        servers_dir,
        in_kernel: false,
        port,
        offline: true,
    };
    let report = run_repair(&ctx).await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "report": report,
                "kernel_running": kernel_running,
                "data_dir": data_dir.display().to_string(),
            }))?
        );
    } else {
        println!("ClotoCore repair v{}", env!("CARGO_PKG_VERSION"));
        println!("Data directory: {}", data_dir.display());
        println!();
        if report.actions.is_empty() {
            println!("  Nothing to repair.");
        } else {
            for action in &report.actions {
                println!("  ✔ {:<24} {}", action.name, action.message);
            }
        }
        println!();
        if kernel_running {
            println!(
                "Kernel is running on port {port} — venv and DB-level repairs are handled \
                 by the kernel: dashboard → Settings → Health → Repair."
            );
        } else {
            println!("DB-level repairs need the running kernel (dashboard → Settings → Health).");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_only_remnants() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("clotocore");
        std::fs::write(&exe, b"exe").unwrap();
        std::fs::write(exe.with_extension("new"), b"half-download").unwrap();
        std::fs::write(exe.with_extension("old"), b"rollback").unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), b"keep").unwrap();

        let removed = cleanup_swap_remnants_at(&exe);
        assert_eq!(removed.len(), 2);
        assert!(!exe.with_extension("new").exists());
        assert!(!exe.with_extension("old").exists());
        assert!(exe.exists(), "the binary itself is never touched");
        assert!(dir.path().join("unrelated.txt").exists());

        // Idempotent: nothing left to remove.
        assert!(cleanup_swap_remnants_at(&exe).is_empty());
    }

    #[test]
    fn sync_adopts_unrecorded_servers_and_drops_stale() {
        let dir = tempfile::tempdir().unwrap();
        let servers = dir.path().join("mcp-servers");
        std::fs::create_dir_all(servers.join("cpersona")).unwrap();
        std::fs::create_dir_all(servers.join(".venv")).unwrap();

        // Seed a receipt with one stale entry.
        footprint::record(
            dir.path(),
            vec![ReceiptEntry::dir(
                "mcp:ghost",
                &servers.join("ghost-was-removed"),
            )],
        );

        let sync = sync_receipt(dir.path());
        assert_eq!(sync.dropped, 1, "stale ghost entry dropped");
        assert!(sync.adopted >= 1, "cpersona adopted");

        let receipt = footprint::load(dir.path()).unwrap();
        assert!(receipt.entries.iter().any(|e| e.id == "mcp:cpersona"));
        assert!(!receipt.entries.iter().any(|e| e.id == "mcp:ghost"));
        assert!(
            !receipt.entries.iter().any(|e| e.id == "mcp:.venv"),
            "dotdirs are not servers"
        );

        // Idempotent: a second sync changes nothing.
        let again = sync_receipt(dir.path());
        assert_eq!(again.adopted, 0);
        assert_eq!(again.dropped, 0);
    }

    #[test]
    fn sync_creates_receipt_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mcp-servers").join("terminal")).unwrap();
        assert!(footprint::load(dir.path()).is_none());

        let sync = sync_receipt(dir.path());
        assert_eq!(sync.adopted, 1);
        assert!(footprint::load(dir.path()).is_some());
    }
}
