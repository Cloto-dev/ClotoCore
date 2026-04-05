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

/// Find a system Python command and its version string.
///
/// Search order:
/// 1. PATH-based candidates: `python3`, `python`, and on Windows `py`
///    (the Python Launcher installed into `C:\Windows\` by python.org — available
///    even when the user did not check "Add Python to PATH").
/// 2. On Windows only: well-known installation directories as a last resort
///    (covers Tauri desktop apps that may not inherit the user's full PATH).
///
/// Each candidate is given a 5-second timeout so a hanging binary does not
/// block the caller indefinitely.
pub(crate) fn find_python_with_version() -> Option<(String, String)> {
    #[cfg(windows)]
    const CANDIDATES: &[&str] = &["python3", "python", "py"];
    #[cfg(not(windows))]
    const CANDIDATES: &[&str] = &["python3", "python"];

    // Try PATH-based candidates first.
    if let Some(result) = try_python_candidates(CANDIDATES) {
        return Some(result);
    }

    // Windows fallback: probe common installation directories.
    #[cfg(windows)]
    if let Some(result) = find_python_in_known_paths() {
        return Some(result);
    }

    None
}

/// Try each command name from `candidates` by spawning `<cmd> --version`.
fn try_python_candidates(candidates: &[&str]) -> Option<(String, String)> {
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    for cmd in candidates {
        if let Some(result) = probe_python(cmd, TIMEOUT) {
            return Some(result);
        }
    }
    None
}

/// Spawn `<cmd> --version` and return `(command, version_string)` on success.
fn probe_python(cmd: &str, timeout: std::time::Duration) -> Option<(String, String)> {
    let child = std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = child else { return None };

    // Poll with timeout to avoid blocking indefinitely.
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };

    let status = status?;
    if !status.success() {
        return None;
    }

    // Read version from stdout/stderr.
    let stdout = child.stdout.take().and_then(|mut s| {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
        Some(buf)
    }).unwrap_or_default();
    let stderr = child.stderr.take().and_then(|mut s| {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
        Some(buf)
    }).unwrap_or_default();

    let combined = if stdout.contains("Python") { stdout } else { stderr };
    let version = combined.trim().strip_prefix("Python ")?;
    Some((cmd.to_string(), version.to_string()))
}

/// Windows-only: scan well-known Python installation directories.
///
/// Covers the case where a Tauri desktop app does not inherit the user's PATH
/// and neither `python`, `python3`, nor `py` are discoverable via PATH alone.
#[cfg(windows)]
fn find_python_in_known_paths() -> Option<(String, String)> {
    use std::path::PathBuf;

    let candidates: Vec<PathBuf> = [
        // python.org default install locations
        std::env::var("LOCALAPPDATA").ok().map(|p| PathBuf::from(p).join("Programs").join("Python")),
        // System-wide install
        Some(PathBuf::from(r"C:\Python")),
        // Windows Store location
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|p| PathBuf::from(p).join("Microsoft").join("WindowsApps")),
    ]
    .into_iter()
    .flatten()
    .collect();

    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    for base in &candidates {
        if !base.is_dir() {
            continue;
        }
        // Scan for python.exe directly or inside versioned subdirectories
        // (e.g., C:\Users\X\AppData\Local\Programs\Python\Python313\python.exe)
        let dirs_to_check: Vec<PathBuf> = std::fs::read_dir(base)
            .ok()
            .map(|entries| {
                let mut dirs: Vec<PathBuf> = entries
                    .filter_map(std::result::Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();
                // Prefer higher version numbers (reverse sort)
                dirs.sort_unstable_by(|a, b| b.cmp(a));
                dirs
            })
            .unwrap_or_default();

        // Also check the base directory itself (e.g., C:\Python\python.exe)
        let mut search_dirs = vec![base.clone()];
        search_dirs.extend(dirs_to_check);

        for dir in &search_dirs {
            let exe = dir.join("python.exe");
            if exe.is_file() {
                let exe_str = exe.to_string_lossy().to_string();
                if let Some(result) = probe_python(&exe_str, TIMEOUT) {
                    return Some(result);
                }
            }
        }
    }
    None
}

/// Find a system Python command (python3, python, or py on Windows).
///
/// Thin wrapper around [`find_python_with_version`] for call sites that only
/// need the command name.
pub(crate) fn find_python() -> Option<String> {
    find_python_with_version().map(|(cmd, _)| cmd)
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
    let (_, version) = find_python_with_version()?;
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
