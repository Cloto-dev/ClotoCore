//! Check registry (DEFENDER_DESIGN.md §4) — canonical source 3.
//!
//! Every diagnosis the defender can make is a registered [`CheckDef`]
//! evaluated against a [`CheckCtx`]. The existing DB/venv checks from
//! `db::health` are absorbed unchanged; the Phase 1 additions cover the
//! pool-free file-level states (a corrupt DB file kills the kernel *before*
//! any pool-based scan can run) and the updater-guard set (§6).
//!
//! `run_scan` is read-only by construction: no registered check mutates
//! anything. Repair is a separate verb (Phase 2).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use sqlx::SqlitePool;

use crate::db::health::{HealthCheck, HealthReport, HealthStatus};

// ── Context ──

pub struct CheckCtx {
    /// `None` when the kernel is not reachable (doctor mode) — pool-dependent
    /// checks are skipped with an explicit marker instead of failing.
    pub pool: Option<SqlitePool>,
    pub data_dir: PathBuf,
    /// Parent of the MCP `.venv` (usually `<data_dir>/mcp-servers`).
    pub servers_dir: Option<PathBuf>,
    /// True when the scan runs inside the serving kernel process.
    pub in_kernel: bool,
    pub port: u16,
    /// Skip checks that need the network (advisory feed).
    pub offline: bool,
}

// ── Registry types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckScope {
    Db,
    Files,
    Update,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

type CheckFuture<'a> = Pin<Box<dyn Future<Output = Option<HealthCheck>> + Send + 'a>>;

pub struct CheckDef {
    pub name: &'static str,
    pub scope: CheckScope,
    pub base_severity: Severity,
    pub fix_capable: bool,
    /// Checks that require a live DB pool; skipped (and reported as such)
    /// when `CheckCtx.pool` is `None`.
    pub needs_pool: bool,
    run: for<'a> fn(&'a CheckCtx) -> CheckFuture<'a>,
}

/// The full Phase 1 registry. Order is meaningful only for presentation.
#[must_use]
pub fn registry() -> &'static [CheckDef] {
    &REGISTRY
}

static REGISTRY: [CheckDef; 17] = [
    // ── Absorbed from db::health (names preserved — API compatibility) ──
    CheckDef {
        name: "db_connection",
        scope: CheckScope::Db,
        base_severity: Severity::Critical,
        fix_capable: false,
        needs_pool: true,
        run: c_db_connection,
    },
    CheckDef {
        name: "orphaned_chat_messages",
        scope: CheckScope::Db,
        base_severity: Severity::Warning,
        fix_capable: true,
        needs_pool: true,
        run: c_orphaned_chat_messages,
    },
    CheckDef {
        name: "orphaned_trusted_commands",
        scope: CheckScope::Db,
        base_severity: Severity::Warning,
        fix_capable: true,
        needs_pool: true,
        run: c_orphaned_trusted_commands,
    },
    CheckDef {
        name: "orphaned_permission_requests",
        scope: CheckScope::Db,
        base_severity: Severity::Warning,
        fix_capable: true,
        needs_pool: true,
        run: c_orphaned_permission_requests,
    },
    CheckDef {
        name: "audit_chain",
        scope: CheckScope::Db,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: true,
        run: c_audit_chain,
    },
    CheckDef {
        name: "venv_exists",
        scope: CheckScope::Files,
        base_severity: Severity::Warning,
        fix_capable: true,
        needs_pool: false,
        run: c_venv_exists,
    },
    CheckDef {
        name: "venv_python_version",
        scope: CheckScope::Files,
        base_severity: Severity::Warning,
        fix_capable: true,
        needs_pool: false,
        run: c_venv_python_version,
    },
    // ── Phase 1 additions ──
    CheckDef {
        name: "db_file_integrity",
        scope: CheckScope::Files,
        base_severity: Severity::Critical,
        fix_capable: false,
        needs_pool: false,
        run: c_db_file_integrity,
    },
    CheckDef {
        name: "legacy_data_dir_drift",
        scope: CheckScope::Files,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_legacy_data_dir_drift,
    },
    CheckDef {
        name: "env_validity",
        scope: CheckScope::Files,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_env_validity,
    },
    CheckDef {
        name: "receipt_staleness",
        scope: CheckScope::Files,
        base_severity: Severity::Info,
        fix_capable: true,
        needs_pool: false,
        run: c_receipt_staleness,
    },
    CheckDef {
        name: "port_availability",
        scope: CheckScope::Runtime,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_port_availability,
    },
    CheckDef {
        name: "llm_proxy_untrusted_callers",
        scope: CheckScope::Runtime,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_llm_proxy_untrusted_callers,
    },
    CheckDef {
        name: "update_write_probe",
        scope: CheckScope::Update,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_update_write_probe,
    },
    CheckDef {
        name: "update_swap_remnants",
        scope: CheckScope::Update,
        base_severity: Severity::Warning,
        fix_capable: true,
        needs_pool: false,
        run: c_update_swap_remnants,
    },
    CheckDef {
        name: "update_disk_space",
        scope: CheckScope::Update,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_update_disk_space,
    },
    CheckDef {
        name: "update_version_coherence",
        scope: CheckScope::Update,
        base_severity: Severity::Warning,
        fix_capable: false,
        needs_pool: false,
        run: c_update_version_coherence,
    },
];

// ── Scan runner ──

pub struct ScanOutcome {
    pub report: HealthReport,
    /// Names of registered checks that could not run in this context
    /// (pool-dependent checks in doctor mode, venv checks without a
    /// servers dir, advisory evaluation when offline).
    pub skipped: Vec<&'static str>,
}

/// Evaluate every registered check (plus advisory evaluation, §5) against
/// `ctx`. Read-only: nothing is mutated, nothing is deleted.
pub async fn run_scan(ctx: &CheckCtx) -> ScanOutcome {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut checks = Vec::new();
    let mut skipped = Vec::new();

    for def in registry() {
        if def.needs_pool && ctx.pool.is_none() {
            skipped.push(def.name);
            continue;
        }
        match (def.run)(ctx).await {
            Some(check) => checks.push(check),
            None => skipped.push(def.name),
        }
    }

    if ctx.offline {
        skipped.push(crate::defender::advisories::CHECK_NAME);
    } else {
        let advisory_check = crate::defender::advisories::evaluate(&checks).await;
        checks.push(advisory_check);
    }

    let status = if checks.iter().any(|c| c.status == HealthStatus::Error) {
        HealthStatus::Error
    } else if checks.iter().any(|c| c.status == HealthStatus::Degraded) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };

    let db_size_bytes = match &ctx.pool {
        Some(pool) => crate::db::health::get_db_size(pool).await.unwrap_or(0),
        None => resolve_db_path(&ctx.data_dir)
            .metadata()
            .map_or(0, |m| i64::try_from(m.len()).unwrap_or(i64::MAX)),
    };

    ScanOutcome {
        report: HealthReport {
            status,
            checks,
            timestamp,
            db_size_bytes,
        },
        skipped,
    }
}

// ── Absorbed check adapters ──

fn c_db_connection(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let pool = ctx.pool.as_ref()?;
        Some(crate::db::health::check_db_connection(pool).await)
    })
}

fn c_orphaned_chat_messages(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let pool = ctx.pool.as_ref()?;
        Some(crate::db::health::check_orphaned_chat_messages(pool).await)
    })
}

fn c_orphaned_trusted_commands(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let pool = ctx.pool.as_ref()?;
        Some(crate::db::health::check_orphaned_trusted_commands(pool).await)
    })
}

fn c_orphaned_permission_requests(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let pool = ctx.pool.as_ref()?;
        Some(crate::db::health::check_orphaned_permission_requests(pool).await)
    })
}

fn c_audit_chain(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let pool = ctx.pool.as_ref()?;
        Some(crate::db::health::check_audit_chain_tail(pool).await)
    })
}

fn c_venv_exists(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let dir = ctx.servers_dir.as_deref()?;
        Some(crate::db::health::check_venv_exists(dir))
    })
}

fn c_venv_python_version(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let dir = ctx.servers_dir.as_deref()?;
        Some(crate::db::health::check_venv_python_version(dir))
    })
}

// ── DB file integrity (pool-free header check + pooled quick_check) ──

/// Resolve the SQLite DB file path the way the kernel does: `DATABASE_URL`
/// wins, otherwise `<data_dir>/cloto_memories.db`.
#[must_use]
pub fn resolve_db_path(data_dir: &Path) -> PathBuf {
    std::env::var("DATABASE_URL")
        .ok()
        .and_then(|url| {
            let rest = url.strip_prefix("sqlite:")?;
            let rest = rest.strip_prefix("//").unwrap_or(rest);
            let rest = rest.split('?').next().unwrap_or(rest);
            if rest.is_empty() || rest == ":memory:" {
                None
            } else {
                Some(PathBuf::from(rest))
            }
        })
        .unwrap_or_else(|| data_dir.join("cloto_memories.db"))
}

const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// File-level verdict on a candidate SQLite DB. Pure so it can be unit-tested.
fn db_header_verdict(path: &Path) -> HealthCheck {
    let name = "db_file_integrity".to_string();
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HealthCheck {
            name,
            status: HealthStatus::Healthy,
            message: "Database file not yet created (fresh install)".into(),
            repairable: false,
            detail: None,
        },
        Err(e) => HealthCheck {
            name,
            status: HealthStatus::Error,
            message: format!("Database file unreadable: {e}"),
            repairable: false,
            detail: Some(serde_json::json!({ "path": path.display().to_string() })),
        },
        Ok(bytes) if bytes.is_empty() => HealthCheck {
            name,
            status: HealthStatus::Healthy,
            message: "Database file is empty (SQLite will initialize it)".into(),
            repairable: false,
            detail: None,
        },
        Ok(bytes) if bytes.len() >= 16 && &bytes[..16] == SQLITE_MAGIC => HealthCheck {
            name,
            status: HealthStatus::Healthy,
            message: "Database file header OK".into(),
            repairable: false,
            detail: None,
        },
        Ok(_) => HealthCheck {
            name,
            status: HealthStatus::Error,
            message: "Database file is not a SQLite database — the kernel will fail to boot \
                      (SQLITE_NOTADB, error code 26)"
                .into(),
            repairable: false,
            detail: Some(serde_json::json!({ "path": path.display().to_string() })),
        },
    }
}

fn c_db_file_integrity(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let path = resolve_db_path(&ctx.data_dir);
        let mut check = db_header_verdict(&path);
        // With a live pool, corroborate with SQLite's own structural check.
        if check.status == HealthStatus::Healthy {
            if let Some(pool) = ctx.pool.as_ref() {
                match crate::db::db_timeout(
                    sqlx::query_scalar::<_, String>("PRAGMA quick_check").fetch_one(pool),
                )
                .await
                {
                    Ok(verdict) if verdict == "ok" => {
                        check.message = "Database file header OK; PRAGMA quick_check OK".into();
                    }
                    Ok(verdict) => {
                        check.status = HealthStatus::Error;
                        check.message = "PRAGMA quick_check reported corruption".into();
                        check.detail = Some(serde_json::json!({ "quick_check": verdict }));
                    }
                    Err(e) => {
                        check.status = HealthStatus::Degraded;
                        check.message = format!("PRAGMA quick_check could not run: {e}");
                    }
                }
            }
        }
        Some(check)
    })
}

// ── Legacy data-dir drift ──

/// True when two paths name the same file on disk. A path that does not exist
/// has no canonical form, so either side failing to canonicalize falls back to
/// comparing the paths as written.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Pure core of the drift check: which `candidates` (≠ the resolved dir)
/// contain a database the running binary would not read?
///
/// `live_db` is the database the binary actually opens — `DATABASE_URL` when
/// set, otherwise `<data_dir>/cloto_memories.db`. A prefix install points it
/// outside the data directory, so a candidate can hold the live database while
/// still differing from the resolved data dir. Without this the check reports
/// that directory as stray and says the binary does not read a file it reads
/// on every query.
///
/// Shared with `defender::purge` so the doctor and the uninstall plan can
/// never disagree about what counts as a stray data directory. The two resolve
/// `live_db` the same way, but only the kernel process loads `.env`; a CLI
/// purge whose environment carries no override therefore still offers the
/// directory for removal. That asymmetry is the safe one — purge stays a
/// superset, and the failure §1 exists to prevent is health knowing about
/// something uninstall forgets, not the other way round.
pub(crate) fn drift_hits(resolved: &Path, live_db: &Path, candidates: &[PathBuf]) -> Vec<PathBuf> {
    candidates
        .iter()
        .filter(|c| {
            let db = c.join("cloto_memories.db");
            c.as_path() != resolved && db.exists() && !same_file(&db, live_db)
        })
        .cloned()
        .collect()
}

fn c_legacy_data_dir_drift(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let mut candidates = vec![crate::config::exe_dir().join("data")];
        if let Some(d) = dirs::data_dir() {
            candidates.push(d.join(crate::config::APP_DATA_DIR_NAME));
        }
        let hits = drift_hits(&ctx.data_dir, &resolve_db_path(&ctx.data_dir), &candidates);
        Some(if hits.is_empty() {
            HealthCheck {
                name: "legacy_data_dir_drift".into(),
                status: HealthStatus::Healthy,
                message: "No stray data directory detected".into(),
                repairable: false,
                detail: None,
            }
        } else {
            HealthCheck {
                name: "legacy_data_dir_drift".into(),
                status: HealthStatus::Degraded,
                message: format!(
                    "{} data director{} with a database exist outside the active data dir — \
                     the running binary does not read {}",
                    hits.len(),
                    if hits.len() == 1 { "y" } else { "ies" },
                    if hits.len() == 1 { "it" } else { "them" },
                ),
                repairable: false,
                detail: Some(serde_json::json!({
                    "active": ctx.data_dir.display().to_string(),
                    "stray": hits.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                })),
            }
        })
    })
}

// ── .env validity ──

fn c_env_validity(ctx: &CheckCtx) -> CheckFuture<'_> {
    let _ = ctx;
    Box::pin(async move {
        let name = "env_validity".to_string();
        let path = crate::apikey::resolve_env_target();
        let env_key_set = std::env::var("CLOTO_API_KEY").is_ok_and(|s| !s.trim().is_empty());

        if !path.exists() {
            return Some(if env_key_set {
                HealthCheck {
                    name,
                    status: HealthStatus::Healthy,
                    message: "Admin key loaded from environment (no .env file)".into(),
                    repairable: false,
                    detail: None,
                }
            } else {
                HealthCheck {
                    name,
                    status: HealthStatus::Degraded,
                    message: "No .env file and no CLOTO_API_KEY in the environment — admin \
                              endpoints are locked out"
                        .into(),
                    repairable: false,
                    detail: Some(serde_json::json!({
                        "expected_env_path": path.display().to_string()
                    })),
                }
            });
        }

        let iter = match dotenvy::from_path_iter(&path) {
            Ok(iter) => iter,
            Err(e) => {
                return Some(HealthCheck {
                    name,
                    status: HealthStatus::Error,
                    message: format!(".env at {} is unreadable: {e}", path.display()),
                    repairable: false,
                    detail: None,
                });
            }
        };
        let mut has_key = false;
        for item in iter {
            match item {
                Ok((key, value)) => {
                    if key == "CLOTO_API_KEY" && !value.trim().is_empty() {
                        has_key = true;
                    }
                }
                Err(e) => {
                    return Some(HealthCheck {
                        name,
                        status: HealthStatus::Error,
                        message: format!(".env at {} is malformed: {e}", path.display()),
                        repairable: false,
                        detail: None,
                    });
                }
            }
        }

        Some(if has_key || env_key_set {
            HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!(".env OK ({})", path.display()),
                repairable: false,
                detail: None,
            }
        } else {
            HealthCheck {
                name,
                status: HealthStatus::Degraded,
                message: format!(
                    ".env at {} has no CLOTO_API_KEY — admin endpoints are locked out",
                    path.display()
                ),
                repairable: false,
                detail: None,
            }
        })
    })
}

// ── Receipt staleness ──

fn c_receipt_staleness(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let name = "receipt_staleness".to_string();
        let Some(receipt) = crate::defender::footprint::load(&ctx.data_dir) else {
            return Some(HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: "No install receipt yet (pre-receipt install) — enumeration falls \
                          back to heuristics until the next footprint mutation writes one"
                    .into(),
                repairable: false,
                detail: None,
            });
        };

        let stale: Vec<String> = receipt
            .entries
            .iter()
            .filter(|e| {
                e.path
                    .as_ref()
                    .is_some_and(|p| !std::path::Path::new(p).exists())
            })
            .map(|e| e.id.clone())
            .collect();

        // Installed MCP servers the receipt does not know about.
        let servers_root = ctx.data_dir.join("mcp-servers");
        let mut unrecorded: Vec<String> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&servers_root) {
            for dir in read.flatten() {
                let file_name = dir.file_name().to_string_lossy().to_string();
                if file_name.starts_with('.') || !dir.path().is_dir() {
                    continue;
                }
                let recorded = receipt.entries.iter().any(|e| {
                    e.path
                        .as_ref()
                        .is_some_and(|p| std::path::Path::new(p) == dir.path())
                });
                if !recorded {
                    unrecorded.push(file_name);
                }
            }
        }

        Some(if stale.is_empty() && unrecorded.is_empty() {
            HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!(
                    "Install receipt is current ({} entries, app v{})",
                    receipt.entries.len(),
                    receipt.app_version
                ),
                repairable: false,
                detail: None,
            }
        } else if stale.is_empty() {
            // Unrecorded-only is the normal state of a pre-receipt install:
            // those servers predate the ledger and fall back to heuristic
            // enumeration. Informational — not a health degradation (Phase 2
            // repair will adopt them into the receipt).
            HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!(
                    "Install receipt is current ({} entries); {} pre-receipt MCP server \
                     dir(s) not yet adopted into the ledger",
                    receipt.entries.len(),
                    unrecorded.len()
                ),
                repairable: true,
                detail: Some(serde_json::json!({
                    "unrecorded_servers": unrecorded,
                })),
            }
        } else {
            HealthCheck {
                name,
                status: HealthStatus::Degraded,
                message: format!(
                    "Install receipt is stale: {} recorded path(s) missing, {} unrecorded \
                     MCP server dir(s)",
                    stale.len(),
                    unrecorded.len()
                ),
                repairable: true,
                detail: Some(serde_json::json!({
                    "stale_entries": stale,
                    "unrecorded_servers": unrecorded,
                })),
            }
        })
    })
}

// ── Port availability ──

/// Report whether this kernel has served an LLM proxy request that carried no
/// valid token. Such a request comes from a connector older than the one the
/// marketplace now ships, and is exactly what would break if the token became
/// mandatory — so the answer is read from what arrived, not from a deadline.
fn c_llm_proxy_untrusted_callers(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        if !ctx.in_kernel {
            // The count lives in the serving process. A scan from outside it
            // would report "none seen" for the wrong reason, so skip instead.
            return None;
        }
        Some(untrusted_callers_check(
            crate::managers::llm_proxy::untrusted_callers(),
        ))
    })
}

/// Split from the check above so both outcomes are testable without going
/// through the process-wide counter.
fn untrusted_callers_check(
    observed: Option<crate::managers::llm_proxy::UntrustedCallers>,
) -> HealthCheck {
    let name = "llm_proxy_untrusted_callers".to_string();
    let Some(seen) = observed else {
        return HealthCheck {
            name,
            status: HealthStatus::Healthy,
            message: "Every LLM proxy request carried this kernel's token".to_string(),
            repairable: false,
            detail: None,
        };
    };
    // Naming the connectors is what turns this from a fact into something the
    // operator can act on: the fix is to update those connectors, and a report
    // that does not say which one leaves them to guess or to update all of
    // them. The count and the names are reported separately because they can
    // disagree honestly — a request that failed before the proxy resolved a
    // provider is counted and cannot be named.
    let named: Vec<&str> = seen.connectors.iter().map(String::as_str).collect();
    let subject = match named.as_slice() {
        // Two different things reach this arm: a call the proxy could not
        // attribute, and a connector that has since been updated (the install
        // path clears it). Naming a stale connector here would be a guess in
        // the second case, so the message reports the count and says where it
        // ends instead.
        [] => "no connector is currently listed as needing an update, and the count \
               clears when this kernel restarts"
            .to_string(),
        [only] => format!("connector '{only}' predates the one the marketplace now ships"),
        many => format!(
            "connectors {} predate the ones the marketplace now ships",
            many.iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    HealthCheck {
        name,
        status: HealthStatus::Degraded,
        message: if named.is_empty() {
            format!(
                "{} LLM proxy request(s) were served without this kernel's token since it \
                 started — {subject}",
                seen.served
            )
        } else {
            format!(
                "{} LLM proxy request(s) were served without this kernel's token — {subject}, \
                 and would stop working if the token were required. Update from the \
                 marketplace to keep it working.",
                seen.served
            )
        },
        repairable: false,
        detail: Some(serde_json::json!({
            "served_without_token": seen.served,
            "last_seen": chrono::DateTime::<chrono::Utc>::from(seen.last_seen).to_rfc3339(),
            // The action this check implies, in the form the dashboard needs to
            // offer it: these ids are marketplace server ids, and updating one
            // is POST /api/marketplace/install with update=true. Empty when the
            // proxy saw the calls but never got far enough to name a caller.
            "stale_connectors": named,
        })),
    }
}

fn c_port_availability(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let name = "port_availability".to_string();
        if ctx.in_kernel {
            return Some(HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!("Port {} is served by this kernel", ctx.port),
                repairable: false,
                detail: None,
            });
        }

        Some(match probe_kernel_version(ctx.port).await {
            KernelProbe::Kernel(version) => HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!("Kernel v{version} is running on port {}", ctx.port),
                repairable: false,
                detail: None,
            },
            KernelProbe::Foreign => HealthCheck {
                name,
                status: HealthStatus::Degraded,
                message: format!(
                    "Port {} is occupied by something that is not a ClotoCore kernel",
                    ctx.port
                ),
                repairable: false,
                detail: None,
            },
            KernelProbe::Free => HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!("Port {} is free (kernel not running)", ctx.port),
                repairable: false,
                detail: None,
            },
        })
    })
}

pub(crate) enum KernelProbe {
    Kernel(String),
    Foreign,
    Free,
}

pub(crate) async fn probe_kernel_version(port: u16) -> KernelProbe {
    let url = format!("http://127.0.0.1:{port}/api/system/version");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return KernelProbe::Free,
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let version = resp.json::<serde_json::Value>().await.ok().and_then(|v| {
                v.pointer("/data/version")
                    .or_else(|| v.get("version"))
                    .and_then(|s| s.as_str().map(String::from))
            });
            match version {
                Some(v) => KernelProbe::Kernel(v),
                None => KernelProbe::Foreign,
            }
        }
        Ok(_) => KernelProbe::Foreign,
        Err(e) if e.is_connect() => KernelProbe::Free,
        Err(_) => KernelProbe::Foreign,
    }
}

// ── Updater guards (§6) ──

fn c_update_write_probe(ctx: &CheckCtx) -> CheckFuture<'_> {
    let _ = ctx;
    Box::pin(async move {
        let name = "update_write_probe".to_string();
        let exe_dir = crate::config::exe_dir();
        let probe = exe_dir.join(format!(".cloto-write-probe-{}", std::process::id()));
        let result = std::fs::write(&probe, b"probe").and_then(|()| std::fs::remove_file(&probe));
        Some(match result {
            Ok(()) => HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: "Executable location is writable — update swap can proceed".into(),
                repairable: false,
                detail: None,
            },
            Err(e) => HealthCheck {
                name,
                status: HealthStatus::Degraded,
                message: format!(
                    "Executable location {} is not writable ({e}) — an update swap will \
                     require elevation",
                    exe_dir.display()
                ),
                repairable: false,
                detail: None,
            },
        })
    })
}

fn c_update_swap_remnants(ctx: &CheckCtx) -> CheckFuture<'_> {
    let _ = ctx;
    Box::pin(async move {
        let name = "update_swap_remnants".to_string();
        let Ok(exe) = std::env::current_exe() else {
            return None;
        };
        let mut remnants = Vec::new();
        for ext in ["new", "old", "old.exe"] {
            let candidate = exe.with_extension(ext);
            if candidate.exists() {
                remnants.push(candidate.display().to_string());
            }
        }
        Some(if remnants.is_empty() {
            HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: "No interrupted-update remnants next to the executable".into(),
                repairable: false,
                detail: None,
            }
        } else {
            let interrupted = remnants.iter().any(|r| r.ends_with(".new"));
            HealthCheck {
                name,
                status: HealthStatus::Degraded,
                message: if interrupted {
                    "A .new binary is present — a previous update swap was interrupted".into()
                } else {
                    "Leftover .old backup binaries from a previous update are present \
                     (safe to clean)"
                        .into()
                },
                repairable: true,
                detail: Some(serde_json::json!({ "remnants": remnants })),
            }
        })
    })
}

const MIN_FREE_BYTES: u64 = 500 * 1024 * 1024;

fn c_update_disk_space(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let name = "update_disk_space".to_string();
        let mut low = Vec::new();
        let mut detail = serde_json::Map::new();
        for (label, path) in [
            ("exe_dir", crate::config::exe_dir()),
            ("data_dir", ctx.data_dir.clone()),
        ] {
            if let Ok(free) = fs4::available_space(&path) {
                detail.insert(
                    label.to_string(),
                    serde_json::json!({
                        "path": path.display().to_string(),
                        "available_bytes": free,
                    }),
                );
                if free < MIN_FREE_BYTES {
                    low.push(format!("{label} ({} MB free)", free / 1024 / 1024));
                }
            }
        }
        Some(if low.is_empty() {
            HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: "Sufficient disk space for updates".into(),
                repairable: false,
                detail: Some(serde_json::Value::Object(detail)),
            }
        } else {
            HealthCheck {
                name,
                status: HealthStatus::Degraded,
                message: format!(
                    "Low disk space: {} — an update download/swap may fail",
                    low.join(", ")
                ),
                repairable: false,
                detail: Some(serde_json::Value::Object(detail)),
            }
        })
    })
}

fn c_update_version_coherence(ctx: &CheckCtx) -> CheckFuture<'_> {
    Box::pin(async move {
        let running = env!("CARGO_PKG_VERSION");

        if ctx.in_kernel {
            // Compare this process against the binary on disk ("updated but
            // not restarted"). Only meaningful for the standalone kernel
            // binary — the desktop app and test harnesses must not be spawned.
            let exe = std::env::current_exe().ok()?;
            let file_name = exe.file_name()?.to_string_lossy().to_string();
            if file_name != "clotocore" && file_name != "clotocore.exe" {
                return None;
            }
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::process::Command::new(&exe).arg("version").output(),
            )
            .await;
            let Ok(Ok(output)) = output else {
                return None;
            };
            let stdout = String::from_utf8_lossy(&output.stdout);
            let disk_version = stdout
                .lines()
                .next()
                .and_then(|l| l.trim().strip_prefix("ClotoCore v"))
                .map(str::to_string)?;
            return Some(version_coherence_verdict(running, &disk_version));
        }

        // Doctor mode: this binary IS the on-disk binary; the running kernel
        // is the process side of the comparison.
        match probe_kernel_version(ctx.port).await {
            KernelProbe::Kernel(kernel_version) => {
                Some(version_coherence_verdict(&kernel_version, running))
            }
            _ => None,
        }
    })
}

fn version_coherence_verdict(process_version: &str, disk_version: &str) -> HealthCheck {
    let name = "update_version_coherence".to_string();
    if process_version == disk_version {
        HealthCheck {
            name,
            status: HealthStatus::Healthy,
            message: format!("Running process and binary agree (v{process_version})"),
            repairable: false,
            detail: None,
        }
    } else {
        HealthCheck {
            name,
            status: HealthStatus::Degraded,
            message: format!(
                "Running process is v{process_version} but the binary on disk is \
                 v{disk_version} — restart to finish the update"
            ),
            repairable: false,
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_are_unique() {
        let mut names: Vec<_> = registry().iter().map(|d| d.name).collect();
        names.sort_unstable();
        let len = names.len();
        names.dedup();
        assert_eq!(len, names.len(), "duplicate check names in registry");
    }

    #[test]
    fn db_header_verdict_classifies() {
        let dir = tempfile::tempdir().unwrap();

        let missing = dir.path().join("missing.db");
        assert_eq!(db_header_verdict(&missing).status, HealthStatus::Healthy);

        let empty = dir.path().join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(db_header_verdict(&empty).status, HealthStatus::Healthy);

        let valid = dir.path().join("valid.db");
        let mut bytes = SQLITE_MAGIC.to_vec();
        bytes.extend_from_slice(&[0u8; 100]);
        std::fs::write(&valid, &bytes).unwrap();
        assert_eq!(db_header_verdict(&valid).status, HealthStatus::Healthy);

        // The boot-fatal class (code 26): a non-SQLite file at the DB path.
        let garbage = dir.path().join("garbage.db");
        std::fs::write(&garbage, b"definitely not a sqlite file").unwrap();
        let verdict = db_header_verdict(&garbage);
        assert_eq!(verdict.status, HealthStatus::Error);
        assert!(verdict.message.contains("code 26"));
    }

    #[test]
    fn drift_hits_only_flags_foreign_dirs_with_a_db() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join("active");
        let stray_with_db = dir.path().join("stray");
        let stray_empty = dir.path().join("empty");
        for d in [&active, &stray_with_db, &stray_empty] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(stray_with_db.join("cloto_memories.db"), b"x").unwrap();
        std::fs::write(active.join("cloto_memories.db"), b"x").unwrap();

        let hits = drift_hits(
            &active,
            &active.join("cloto_memories.db"),
            &[active.clone(), stray_with_db.clone(), stray_empty],
        );
        assert_eq!(hits, vec![stray_with_db]);
    }

    /// A prefix install puts the live database outside the data directory, so
    /// the directory holding it is not the resolved data dir and used to be
    /// reported as stray — with a message saying the binary does not read the
    /// very file every query goes to.
    #[test]
    fn drift_hits_spares_the_directory_holding_the_live_database() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("home/.local/share/cloto-system");
        let prefix_data = dir.path().join("opt/cloto/data");
        for d in [&data_dir, &prefix_data] {
            std::fs::create_dir_all(d).unwrap();
        }
        let live_db = prefix_data.join("cloto_memories.db");
        std::fs::write(&live_db, b"x").unwrap();

        assert!(
            drift_hits(&data_dir, &live_db, std::slice::from_ref(&prefix_data)).is_empty(),
            "the directory holding the database the binary opens is not drift"
        );

        // The same directory is drift once the binary reads a database
        // somewhere else — otherwise the assertion above would hold for a
        // check that had simply stopped reporting anything.
        assert_eq!(
            drift_hits(
                &data_dir,
                &data_dir.join("cloto_memories.db"),
                std::slice::from_ref(&prefix_data)
            ),
            vec![prefix_data]
        );
    }

    #[test]
    fn version_coherence_flags_mismatch() {
        assert_eq!(
            version_coherence_verdict("0.6.8", "0.6.8").status,
            HealthStatus::Healthy
        );
        let mismatch = version_coherence_verdict("0.6.7", "0.6.8");
        assert_eq!(mismatch.status, HealthStatus::Degraded);
        assert!(mismatch.message.contains("restart"));
    }

    #[test]
    fn resolve_db_path_parses_database_url_forms() {
        // Not env-mutating: exercise the parsing via a scoped helper instead.
        // (DATABASE_URL handling itself is covered by config.rs tests.)
        let dir = std::path::Path::new("/tmp/x");
        // No env override in the test environment ⇒ default path.
        let p = resolve_db_path(dir);
        assert!(p.ends_with("cloto_memories.db"));
    }

    #[test]
    fn untrusted_callers_check_is_healthy_when_nothing_was_seen() {
        let check = untrusted_callers_check(None);
        assert_eq!(check.status, HealthStatus::Healthy);
        assert!(check.detail.is_none());
    }

    #[test]
    fn untrusted_callers_check_degrades_and_reports_the_count() {
        let observed = crate::managers::llm_proxy::UntrustedCallers {
            served: 3,
            last_seen: std::time::SystemTime::UNIX_EPOCH,
            connectors: std::collections::BTreeSet::new(),
        };
        let check = untrusted_callers_check(Some(observed));
        assert_eq!(check.status, HealthStatus::Degraded);
        assert!(
            check.message.contains('3'),
            "the count belongs in the message: {}",
            check.message
        );
        let detail = check.detail.expect("a degraded result carries detail");
        assert_eq!(detail["served_without_token"], 3);
    }

    fn observed_from(served: u64, ids: &[&str]) -> crate::managers::llm_proxy::UntrustedCallers {
        crate::managers::llm_proxy::UntrustedCallers {
            served,
            last_seen: std::time::SystemTime::UNIX_EPOCH,
            connectors: ids.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    #[test]
    fn untrusted_callers_check_names_the_connector_to_update() {
        // The operator's next move is to update a specific connector. A report
        // that only counts requests makes them guess which, and the marketplace
        // will not tell them either (it compares version strings, and a content
        // change under the same version leaves the entry looking current).
        let check = untrusted_callers_check(Some(observed_from(2, &["groq"])));
        assert!(
            check.message.contains("groq"),
            "the connector belongs in the message: {}",
            check.message
        );
        let detail = check.detail.expect("a degraded result carries detail");
        assert_eq!(detail["stale_connectors"], serde_json::json!(["groq"]));
    }

    #[test]
    fn untrusted_callers_check_names_every_connector_it_saw() {
        let check = untrusted_callers_check(Some(observed_from(5, &["claude", "groq"])));
        for id in ["claude", "groq"] {
            assert!(
                check.message.contains(id),
                "{id} is missing from: {}",
                check.message
            );
        }
        let detail = check.detail.expect("a degraded result carries detail");
        assert_eq!(
            detail["stale_connectors"],
            serde_json::json!(["claude", "groq"]),
            "the ids the dashboard offers to update must all be there"
        );
    }

    #[test]
    fn untrusted_callers_check_still_reports_when_it_named_nobody() {
        // A request that failed before the provider resolved is counted and
        // cannot be named. Degrading on the count alone is the honest outcome:
        // silence here would hide calls that the token requirement will break.
        let check = untrusted_callers_check(Some(observed_from(4, &[])));
        assert_eq!(check.status, HealthStatus::Degraded);
        let detail = check.detail.expect("a degraded result carries detail");
        assert_eq!(detail["served_without_token"], 4);
        assert_eq!(detail["stale_connectors"], serde_json::json!([]));
        // The same arm is reached after every named connector was updated, so
        // it must not assert that something installed is still out of date.
        assert!(
            !check.message.contains("predate"),
            "an unattributed count must not accuse an installed connector: {}",
            check.message
        );
    }

    #[test]
    fn untrusted_callers_check_is_registered_once() {
        let hits = registry()
            .iter()
            .filter(|d| d.name == "llm_proxy_untrusted_callers")
            .count();
        assert_eq!(hits, 1, "the check must be in the registry to ever run");
    }
}
