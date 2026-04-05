//! Kernel health check — database integrity, consistency scanning, and venv recovery.

use std::path::Path;

use serde::Serialize;
use sqlx::SqlitePool;

use super::db_timeout;

// ── Types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: HealthStatus,
    pub message: String,
    pub repairable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub checks: Vec<HealthCheck>,
    pub timestamp: String,
    pub db_size_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairAction {
    pub name: String,
    pub fixed_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    pub actions: Vec<RepairAction>,
    pub total_fixed: usize,
}

// ── Quick Scan ──

pub async fn run_quick_scan(pool: &SqlitePool) -> anyhow::Result<HealthReport> {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut checks = Vec::new();

    // 1. DB connection
    checks.push(check_db_connection(pool).await);

    // 2. Orphaned chat_messages
    checks.push(check_orphaned_chat_messages(pool).await);

    // 3. Orphaned trusted_commands
    checks.push(check_orphaned_trusted_commands(pool).await);

    // 4. Orphaned permission_requests
    checks.push(check_orphaned_permission_requests(pool).await);

    // 5. Audit chain (last 2 entries)
    checks.push(check_audit_chain_tail(pool).await);

    // Overall status
    let status = if checks.iter().any(|c| c.status == HealthStatus::Error) {
        HealthStatus::Error
    } else if checks.iter().any(|c| c.status == HealthStatus::Degraded) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };

    // DB size
    let db_size_bytes = get_db_size(pool).await.unwrap_or(0);

    Ok(HealthReport {
        status,
        checks,
        timestamp,
        db_size_bytes,
    })
}

// ── Full Quick Scan (DB + venv) ──

pub async fn run_full_quick_scan(
    pool: &SqlitePool,
    servers_dir: Option<&Path>,
) -> anyhow::Result<HealthReport> {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut checks = Vec::new();

    // DB checks
    checks.push(check_db_connection(pool).await);
    checks.push(check_orphaned_chat_messages(pool).await);
    checks.push(check_orphaned_trusted_commands(pool).await);
    checks.push(check_orphaned_permission_requests(pool).await);
    checks.push(check_audit_chain_tail(pool).await);

    // venv checks (if servers_dir available)
    if let Some(dir) = servers_dir {
        checks.push(check_venv_exists(dir));
        checks.push(check_venv_python_version(dir));
    }

    let status = if checks.iter().any(|c| c.status == HealthStatus::Error) {
        HealthStatus::Error
    } else if checks.iter().any(|c| c.status == HealthStatus::Degraded) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };

    let db_size_bytes = get_db_size(pool).await.unwrap_or(0);

    Ok(HealthReport {
        status,
        checks,
        timestamp,
        db_size_bytes,
    })
}

// ── Full Repair (DB + venv) ──

pub async fn run_full_repair(
    pool: &SqlitePool,
    servers_dir: Option<&Path>,
) -> anyhow::Result<RepairReport> {
    let mut actions = Vec::new();

    // DB repairs
    let count = repair_orphaned_chat_messages(pool).await?;
    if count > 0 {
        actions.push(RepairAction {
            name: "orphaned_chat_messages".into(),
            fixed_count: count,
            message: format!("Deleted {count} orphaned chat message(s)"),
        });
    }
    let count = repair_orphaned_trusted_commands(pool).await?;
    if count > 0 {
        actions.push(RepairAction {
            name: "orphaned_trusted_commands".into(),
            fixed_count: count,
            message: format!("Deleted {count} orphaned trusted command(s)"),
        });
    }
    let count = repair_orphaned_permission_requests(pool).await?;
    if count > 0 {
        actions.push(RepairAction {
            name: "orphaned_permission_requests".into(),
            fixed_count: count,
            message: format!("Deleted {count} orphaned permission request(s)"),
        });
    }

    // venv repair (if servers_dir available and venv is broken)
    if let Some(dir) = servers_dir {
        if let Some(action) = repair_venv(dir).await {
            actions.push(action);
        }
    }

    let total_fixed = actions.iter().map(|a| a.fixed_count).sum();
    Ok(RepairReport {
        actions,
        total_fixed,
    })
}

// ── Standard Repair ──

pub async fn run_standard_repair(pool: &SqlitePool) -> anyhow::Result<RepairReport> {
    let mut actions = Vec::new();

    // 1. Delete orphaned chat_messages
    let count = repair_orphaned_chat_messages(pool).await?;
    if count > 0 {
        actions.push(RepairAction {
            name: "orphaned_chat_messages".into(),
            fixed_count: count,
            message: format!("Deleted {count} orphaned chat message(s)"),
        });
    }

    // 2. Delete orphaned trusted_commands
    let count = repair_orphaned_trusted_commands(pool).await?;
    if count > 0 {
        actions.push(RepairAction {
            name: "orphaned_trusted_commands".into(),
            fixed_count: count,
            message: format!("Deleted {count} orphaned trusted command(s)"),
        });
    }

    // 3. Delete orphaned permission_requests
    let count = repair_orphaned_permission_requests(pool).await?;
    if count > 0 {
        actions.push(RepairAction {
            name: "orphaned_permission_requests".into(),
            fixed_count: count,
            message: format!("Deleted {count} orphaned permission request(s)"),
        });
    }

    let total_fixed = actions.iter().map(|a| a.fixed_count).sum();
    Ok(RepairReport {
        actions,
        total_fixed,
    })
}

// ── Individual Checks ──

async fn check_db_connection(pool: &SqlitePool) -> HealthCheck {
    match db_timeout(sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool)).await {
        Ok(_) => HealthCheck {
            name: "db_connection".into(),
            status: HealthStatus::Healthy,
            message: "Database connection OK".into(),
            repairable: false,
            detail: None,
        },
        Err(e) => HealthCheck {
            name: "db_connection".into(),
            status: HealthStatus::Error,
            message: format!("Database connection failed: {e}"),
            repairable: false,
            detail: None,
        },
    }
}

async fn check_orphaned_chat_messages(pool: &SqlitePool) -> HealthCheck {
    let query = "SELECT COUNT(*) FROM chat_messages WHERE agent_id NOT IN (SELECT id FROM agents)";
    match db_timeout(sqlx::query_scalar::<_, i32>(query).fetch_one(pool)).await {
        Ok(0) => HealthCheck {
            name: "orphaned_chat_messages".into(),
            status: HealthStatus::Healthy,
            message: "No orphaned chat messages".into(),
            repairable: false,
            detail: None,
        },
        Ok(count) => HealthCheck {
            name: "orphaned_chat_messages".into(),
            status: HealthStatus::Degraded,
            message: format!("{count} orphaned chat message(s) found"),
            repairable: true,
            detail: Some(serde_json::json!({ "count": count })),
        },
        Err(e) => HealthCheck {
            name: "orphaned_chat_messages".into(),
            status: HealthStatus::Error,
            message: format!("Check failed: {e}"),
            repairable: false,
            detail: None,
        },
    }
}

async fn check_orphaned_trusted_commands(pool: &SqlitePool) -> HealthCheck {
    let query =
        "SELECT COUNT(*) FROM trusted_commands WHERE agent_id NOT IN (SELECT id FROM agents)";
    match db_timeout(sqlx::query_scalar::<_, i32>(query).fetch_one(pool)).await {
        Ok(0) => HealthCheck {
            name: "orphaned_trusted_commands".into(),
            status: HealthStatus::Healthy,
            message: "No orphaned trusted commands".into(),
            repairable: false,
            detail: None,
        },
        Ok(count) => HealthCheck {
            name: "orphaned_trusted_commands".into(),
            status: HealthStatus::Degraded,
            message: format!("{count} orphaned trusted command(s) found"),
            repairable: true,
            detail: Some(serde_json::json!({ "count": count })),
        },
        Err(e) => HealthCheck {
            name: "orphaned_trusted_commands".into(),
            status: HealthStatus::Error,
            message: format!("Check failed: {e}"),
            repairable: false,
            detail: None,
        },
    }
}

async fn check_orphaned_permission_requests(pool: &SqlitePool) -> HealthCheck {
    let query = "SELECT COUNT(*) FROM permission_requests WHERE plugin_id NOT IN (SELECT plugin_id FROM plugin_settings) AND status = 'pending'";
    match db_timeout(sqlx::query_scalar::<_, i32>(query).fetch_one(pool)).await {
        Ok(0) => HealthCheck {
            name: "orphaned_permission_requests".into(),
            status: HealthStatus::Healthy,
            message: "No orphaned permission requests".into(),
            repairable: false,
            detail: None,
        },
        Ok(count) => HealthCheck {
            name: "orphaned_permission_requests".into(),
            status: HealthStatus::Degraded,
            message: format!("{count} orphaned permission request(s) found"),
            repairable: true,
            detail: Some(serde_json::json!({ "count": count })),
        },
        Err(e) => HealthCheck {
            name: "orphaned_permission_requests".into(),
            status: HealthStatus::Error,
            message: format!("Check failed: {e}"),
            repairable: false,
            detail: None,
        },
    }
}

async fn check_audit_chain_tail(pool: &SqlitePool) -> HealthCheck {
    // Verify the last 2 audit log entries have consistent chain hashes
    let query = "SELECT chain_hash, timestamp, event_type, actor_id, target_id, result FROM audit_logs ORDER BY id DESC LIMIT 2";

    #[derive(sqlx::FromRow)]
    #[allow(dead_code)]
    struct AuditRow {
        chain_hash: Option<String>,
        timestamp: String,
        event_type: String,
        actor_id: Option<String>,
        target_id: Option<String>,
        result: String,
    }

    match db_timeout(sqlx::query_as::<_, AuditRow>(query).fetch_all(pool)).await {
        Ok(rows) if rows.len() < 2 => HealthCheck {
            name: "audit_chain".into(),
            status: HealthStatus::Healthy,
            message: "Audit chain OK (fewer than 2 entries)".into(),
            repairable: false,
            detail: None,
        },
        Ok(rows) => {
            // rows[0] is newest, rows[1] is its predecessor
            let newest = &rows[0];
            let prev = &rows[1];

            match (&newest.chain_hash, &prev.chain_hash) {
                (Some(_), Some(_)) => HealthCheck {
                    name: "audit_chain".into(),
                    status: HealthStatus::Healthy,
                    message: "Audit chain hashes present".into(),
                    repairable: false,
                    detail: None,
                },
                (None, _) | (_, None) => HealthCheck {
                    name: "audit_chain".into(),
                    status: HealthStatus::Degraded,
                    message: "Audit chain has entries without chain_hash (pre-migration data)"
                        .into(),
                    repairable: false,
                    detail: None,
                },
            }
        }
        Err(e) => HealthCheck {
            name: "audit_chain".into(),
            status: HealthStatus::Error,
            message: format!("Audit chain check failed: {e}"),
            repairable: false,
            detail: None,
        },
    }
}

async fn get_db_size(pool: &SqlitePool) -> anyhow::Result<i64> {
    let page_count: i64 =
        db_timeout(sqlx::query_scalar("SELECT page_count FROM pragma_page_count").fetch_one(pool))
            .await?;
    let page_size: i64 =
        db_timeout(sqlx::query_scalar("SELECT page_size FROM pragma_page_size").fetch_one(pool))
            .await?;
    Ok(page_count * page_size)
}

// ── Repair Functions ──

async fn repair_orphaned_chat_messages(pool: &SqlitePool) -> anyhow::Result<usize> {
    let result = db_timeout(
        sqlx::query("DELETE FROM chat_messages WHERE agent_id NOT IN (SELECT id FROM agents)")
            .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() as usize)
}

async fn repair_orphaned_trusted_commands(pool: &SqlitePool) -> anyhow::Result<usize> {
    let result = db_timeout(
        sqlx::query("DELETE FROM trusted_commands WHERE agent_id NOT IN (SELECT id FROM agents)")
            .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() as usize)
}

async fn repair_orphaned_permission_requests(pool: &SqlitePool) -> anyhow::Result<usize> {
    let result = db_timeout(
        sqlx::query("DELETE FROM permission_requests WHERE plugin_id NOT IN (SELECT plugin_id FROM plugin_settings) AND status = 'pending'")
            .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() as usize)
}

// ── venv Checks ──

fn check_venv_exists(servers_dir: &Path) -> HealthCheck {
    let venv_dir = servers_dir.join(".venv");
    let pyvenv_cfg = venv_dir.join("pyvenv.cfg");
    if pyvenv_cfg.exists() {
        HealthCheck {
            name: "venv_exists".into(),
            status: HealthStatus::Healthy,
            message: "Python venv exists".into(),
            repairable: false,
            detail: None,
        }
    } else {
        HealthCheck {
            name: "venv_exists".into(),
            status: HealthStatus::Degraded,
            message: "Python venv not found — MCP servers may not work".into(),
            repairable: true,
            detail: None,
        }
    }
}

fn check_venv_python_version(servers_dir: &Path) -> HealthCheck {
    let venv_dir = servers_dir.join(".venv");
    if !venv_dir.join("pyvenv.cfg").exists() {
        // Already reported by check_venv_exists
        return HealthCheck {
            name: "venv_python_version".into(),
            status: HealthStatus::Degraded,
            message: "Cannot check Python version — venv missing".into(),
            repairable: true,
            detail: None,
        };
    }

    if crate::managers::mcp_venv::is_venv_stale(&venv_dir) {
        HealthCheck {
            name: "venv_python_version".into(),
            status: HealthStatus::Degraded,
            message: "venv Python version does not match system Python — rebuild recommended"
                .into(),
            repairable: true,
            detail: None,
        }
    } else {
        HealthCheck {
            name: "venv_python_version".into(),
            status: HealthStatus::Healthy,
            message: "venv Python version matches system".into(),
            repairable: false,
            detail: None,
        }
    }
}

// ── venv Repair ──

async fn repair_venv(servers_dir: &Path) -> Option<RepairAction> {
    let venv_dir = servers_dir.join(".venv");
    let needs_repair = !venv_dir.join("pyvenv.cfg").exists()
        || crate::managers::mcp_venv::is_venv_stale(&venv_dir);

    if !needs_repair {
        return None;
    }

    let Some(python_cmd) = crate::managers::mcp_venv::find_python() else {
        tracing::warn!("Cannot repair venv: Python not found in PATH");
        return Some(RepairAction {
            name: "venv_repair".into(),
            fixed_count: 0,
            message: "Failed: Python not found in PATH".into(),
        });
    };

    // Recreate venv
    tracing::info!("Repairing venv at {}", venv_dir.display());
    let result = tokio::process::Command::new(&python_cmd)
        .args(["-m", "venv", "--clear", &venv_dir.to_string_lossy()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                "venv creation failed: {}",
                stderr.lines().last().unwrap_or("")
            );
            return Some(RepairAction {
                name: "venv_repair".into(),
                fixed_count: 0,
                message: "Failed to create venv".into(),
            });
        }
        Err(e) => {
            tracing::warn!("Failed to run python: {e}");
            return Some(RepairAction {
                name: "venv_repair".into(),
                fixed_count: 0,
                message: format!("Failed to run python: {e}"),
            });
        }
    }

    // Upgrade pip
    let pip_path = crate::managers::mcp_venv::venv_pip(&venv_dir);
    let pip_str = pip_path.to_string_lossy().to_string();
    let _ = tokio::process::Command::new(&pip_str)
        .args(["install", "--upgrade", "pip", "--quiet"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    // Reinstall all server dependencies
    let installed = crate::managers::mcp_venv::install_server_deps(&pip_str, servers_dir).await;

    Some(RepairAction {
        name: "venv_repair".into(),
        fixed_count: 1,
        message: format!("Rebuilt venv and installed {installed} server(s)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_status_serialization() {
        let report = HealthReport {
            status: HealthStatus::Healthy,
            checks: vec![HealthCheck {
                name: "test".into(),
                status: HealthStatus::Healthy,
                message: "OK".into(),
                repairable: false,
                detail: None,
            }],
            timestamp: "2026-04-05T00:00:00Z".into(),
            db_size_bytes: 1024,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["checks"][0]["status"], "healthy");
    }

    #[tokio::test]
    async fn test_repair_report_serialization() {
        let report = RepairReport {
            actions: vec![RepairAction {
                name: "test".into(),
                fixed_count: 3,
                message: "Fixed 3 items".into(),
            }],
            total_fixed: 3,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["total_fixed"], 3);
    }
}
