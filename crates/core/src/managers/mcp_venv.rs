//! Auto-setup for Python MCP server virtual environment.
//!
//! Checks `[paths].servers` in `mcp.toml` for the servers directory,
//! falling back to `mcp-servers/` in the project root.
//! If missing, creates a venv and installs dependencies from each server's `pyproject.toml`.
//! This is non-fatal — the kernel starts normally even if setup fails.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Resolve the project root using McpClientManager's existing detection logic.
fn resolve_project_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let root = super::McpClientManager::detect_project_root(&exe)?;
    Some(root)
}

/// Read `[paths].servers` from `mcp.toml` in the project root.
/// Uses the same `McpConfigFile` typed deserialization as `load_config_file()`
/// to avoid silent failures from generic `toml::Value` parsing.
#[must_use]
pub fn resolve_servers_dir_from_config() -> Option<PathBuf> {
    let root = resolve_project_root()?;
    let config_path = root.join("mcp.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: super::mcp_protocol::McpConfigFile = toml::from_str(&content).ok()?;
    let raw = config.paths.get("servers")?;
    // Support env var expansion in path values: ${VAR_NAME}
    // CLOTO_MCP_SERVERS defaults to ../cloto-mcp-servers/servers when unset.
    let resolved = if let Some(var_name) = raw.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
    {
        match std::env::var(var_name) {
            Ok(val) => val,
            Err(_) if var_name == "CLOTO_MCP_SERVERS" => "../cloto-mcp-servers/servers".to_string(),
            Err(_) => return None,
        }
    } else {
        raw.clone()
    };
    let path = PathBuf::from(&resolved);
    // Resolve relative paths against project root (not CWD)
    if path.is_relative() {
        Some(root.join(path))
    } else {
        Some(path)
    }
}

/// Get the path to the shared venv directory.
/// Checks `[paths].servers` in `mcp.toml` first, then falls back to
/// `mcp-servers/.venv` in the project root.
#[must_use]
pub fn resolve_venv_dir() -> Option<PathBuf> {
    // Primary: [paths].servers from mcp.toml
    if let Some(servers_dir) = resolve_servers_dir_from_config() {
        let venv = servers_dir.join(".venv");
        if venv.join("pyvenv.cfg").exists() {
            return Some(venv);
        }
    }
    // Fallback: legacy location
    resolve_project_root().map(|root| root.join("mcp-servers").join(".venv"))
}

/// Get the path to the Python executable inside the venv.
#[must_use]
pub fn resolve_venv_python() -> Option<PathBuf> {
    let venv_dir = resolve_venv_dir()?;

    // Windows: Scripts/python.exe, Unix: bin/python
    let python = if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    };

    if python.exists() {
        Some(python)
    } else {
        None
    }
}

/// Find a system Python command (python3 or python).
pub(crate) fn find_python() -> Option<String> {
    for cmd in &["python3", "python"] {
        let result = std::process::Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        if let Ok(mut child) = result {
            if let Ok(status) = child.wait() {
                if status.success() {
                    return Some((*cmd).to_string());
                }
            }
        }
    }
    None
}

/// Extract the Python version from a venv's `pyvenv.cfg` (e.g., "3.13.3").
fn read_venv_python_version(venv_dir: &Path) -> Option<String> {
    let cfg_path = venv_dir.join("pyvenv.cfg");
    let content = std::fs::read_to_string(cfg_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(version) = line.strip_prefix("version") {
            let version = version.trim_start_matches([' ', '=']);
            let version = version.trim();
            if !version.is_empty() {
                return Some(version.to_string());
            }
        }
    }
    None
}

/// Get the system Python major.minor version string (e.g., "3.13").
fn system_python_major_minor() -> Option<String> {
    let cmd = find_python()?;
    let output = std::process::Command::new(&cmd)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    let combined = if out.contains("Python") { out } else { err };
    let version = combined.trim().strip_prefix("Python ")?;
    // Extract major.minor only (e.g., "3.13.3" → "3.13")
    let mut parts = version.splitn(3, '.');
    let major = parts.next()?;
    let minor = parts.next()?;
    Some(format!("{major}.{minor}"))
}

/// Check if an existing venv's Python version mismatches the system Python.
/// Returns `true` if the venv is stale and should be recreated.
/// Returns `false` if the venv is OK, or if versions cannot be determined.
pub(crate) fn is_venv_stale(venv_dir: &Path) -> bool {
    let Some(venv_version) = read_venv_python_version(venv_dir) else {
        return false; // Can't read venv version — assume OK
    };
    let Some(system_mm) = system_python_major_minor() else {
        return false; // Can't detect system Python — assume OK
    };
    // Extract major.minor from venv version (e.g., "3.13.3" → "3.13")
    let venv_mm: String = venv_version
        .splitn(3, '.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".");

    if venv_mm != system_mm {
        tracing::info!(
            venv_python = %venv_mm,
            system_python = %system_mm,
            "Venv Python version mismatch detected"
        );
        return true;
    }
    false
}

/// Get the pip executable path inside the venv.
pub(crate) fn venv_pip(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("pip.exe")
    } else {
        venv_dir.join("bin").join("pip")
    }
}

/// Install (or re-sync) each MCP server's pyproject.toml dependencies into
/// the shared venv. Uses a single `pip install` invocation with all server
/// paths to avoid lock contention from parallel pip processes.
pub(crate) async fn install_server_deps(pip_str: &str, mcp_servers_dir: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(mcp_servers_dir) else {
        return 0;
    };

    // Collect valid server directories (with pyproject.toml)
    let mut server_paths: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip .venv, common, tests, and non-Python servers (Rust: discord, avatar)
        if name.starts_with('.')
            || name == "common"
            || name == "tests"
            || !path.join("pyproject.toml").exists()
        {
            continue;
        }
        // Skip Rust servers (they have Cargo.toml, not pip-installable)
        if path.join("Cargo.toml").exists() {
            continue;
        }
        server_paths.push(path.to_string_lossy().to_string());
    }

    if server_paths.is_empty() {
        return 0;
    }

    let count = server_paths.len() as u32;
    info!("  Installing MCP deps for {} server(s)", count);

    // Single pip invocation with all server paths — avoids parallel lock contention
    let mut args = vec!["install".to_string(), "--quiet".to_string()];
    args.extend(server_paths);

    let result = tokio::process::Command::new(pip_str)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => count,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last_line = stderr.lines().last().unwrap_or("unknown error");
            warn!(
                "  pip install failed (exit code {:?}): {}",
                output.status.code(),
                last_line
            );
            0
        }
        Err(e) => {
            warn!("  Failed to run pip: {}", e);
            0
        }
    }
}

/// Ensure the MCP Python venv exists and has dependencies installed.
/// Non-fatal: logs warnings on failure but does not prevent kernel startup.
/// Re-syncs dependencies on every startup so late-added servers are picked up.
///
/// If `data_dir` is provided, also syncs marketplace-installed servers from
/// `data_dir/mcp-servers/` so they survive venv recreation.
pub async fn ensure_mcp_venv(data_dir: Option<&Path>) {
    let Some(project_root) = resolve_project_root() else {
        warn!("Could not detect project root — skipping MCP venv setup");
        return;
    };

    // Use [paths].servers from mcp.toml if available, otherwise legacy path
    let (venv_dir, mcp_servers_dir) = if let Some(servers_dir) = resolve_servers_dir_from_config() {
        (servers_dir.join(".venv"), servers_dir)
    } else {
        (
            project_root.join("mcp-servers").join(".venv"),
            project_root.join("mcp-servers"),
        )
    };
    let mut venv_exists = venv_dir.join("pyvenv.cfg").exists();

    // Detect stale venv (Python major.minor mismatch) and recreate
    if venv_exists && is_venv_stale(&venv_dir) {
        warn!(
            "Python version mismatch — recreating venv at {}",
            venv_dir.display()
        );
        let _ = std::fs::remove_dir_all(&venv_dir);
        venv_exists = false;
    }

    if venv_exists {
        info!("MCP Python venv found at {}", venv_dir.display());
    } else {
        info!("MCP Python venv not found — setting up automatically...");

        // Find system Python
        let Some(python) = find_python() else {
            warn!(
                "Python 3.10+ not found in PATH. MCP servers requiring Python will not start. \
                 Install Python and restart, or run: bash scripts/setup-mcp-deps.sh"
            );
            return;
        };

        info!("Using system Python: {}", python);

        // Create venv
        let venv_path_str = venv_dir.to_string_lossy().to_string();
        let create_result = tokio::process::Command::new(&python)
            .args(["-m", "venv", &venv_path_str])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .status()
            .await;

        match create_result {
            Ok(status) if status.success() => {
                info!("Created Python venv at {}", venv_dir.display());
            }
            Ok(status) => {
                warn!(
                    "Failed to create Python venv (exit code: {:?}). \
                     Run manually: bash scripts/setup-mcp-deps.sh",
                    status.code()
                );
                return;
            }
            Err(e) => {
                warn!("Failed to run Python for venv creation: {}", e);
                return;
            }
        }

        // Upgrade pip (only on fresh venv creation)
        let pip = venv_pip(&venv_dir);
        let pip_str = pip.to_string_lossy().to_string();
        let _ = tokio::process::Command::new(&pip_str)
            .args(["install", "--upgrade", "pip", "--quiet"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .status()
            .await;
    }

    // Always sync dependencies — handles late-added servers and new packages.
    let pip = venv_pip(&venv_dir);
    let pip_str = pip.to_string_lossy().to_string();
    let mut installed = install_server_deps(&pip_str, &mcp_servers_dir).await;

    // Also sync marketplace-installed servers from data_dir/mcp-servers/.
    // pip install is idempotent (no-op for satisfied packages), so double-scanning is safe.
    if let Some(data_dir) = data_dir {
        let marketplace_dir = data_dir.join("mcp-servers");
        if marketplace_dir.is_dir() && marketplace_dir != mcp_servers_dir {
            let mp_count = install_server_deps(&pip_str, &marketplace_dir).await;
            if mp_count > 0 {
                info!("Marketplace dep sync: {} server(s) processed", mp_count);
            }
            installed += mp_count;
        }
    }

    info!(
        "MCP venv dep sync complete: {} server(s) processed at {}",
        installed,
        venv_dir.display()
    );
}
