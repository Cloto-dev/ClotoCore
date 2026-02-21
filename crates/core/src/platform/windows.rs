use anyhow::{bail, Context};
use std::path::Path;
use std::process::Command;
use tracing::info;

const SERVICE_NAME: &str = "Exiv";

/// Register Exiv as a Windows Service via sc.exe
pub fn install_service(prefix: &Path, _user: Option<&str>) -> anyhow::Result<()> {
    let exe_path = prefix.join("exiv_system.exe");

    let status = Command::new("sc.exe")
        .args([
            "create",
            SERVICE_NAME,
            &format!("binPath={}", exe_path.display()),
            "start=auto",
            "DisplayName=Exiv System",
        ])
        .status()
        .context("Failed to run sc.exe (are you running as Administrator?)")?;

    if !status.success() {
        bail!("sc.exe create failed with exit code {:?}", status.code());
    }

    // Configure restart on failure (restart after 5 seconds)
    let _ = Command::new("sc.exe")
        .args(["failure", SERVICE_NAME, "reset=60", "actions=restart/5000"])
        .status();

    info!("✅ Service registered: {}", SERVICE_NAME);
    info!("   Start with: sc.exe start {}", SERVICE_NAME);
    info!("   Status:     sc.exe query {}", SERVICE_NAME);
    Ok(())
}

/// Remove Exiv Windows Service
pub fn uninstall_service() -> anyhow::Result<()> {
    // Stop if running (ignore errors)
    let _ = Command::new("sc.exe").args(["stop", SERVICE_NAME]).status();

    // Wait briefly for stop
    std::thread::sleep(std::time::Duration::from_secs(2));

    let status = Command::new("sc.exe")
        .args(["delete", SERVICE_NAME])
        .status()
        .context("Failed to run sc.exe")?;

    if status.success() {
        info!("✅ Service removed: {}", SERVICE_NAME);
    } else {
        info!("ℹ️  Service removal returned non-zero (may not exist)");
    }
    Ok(())
}

pub fn start_service() -> anyhow::Result<()> {
    let status = Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .status()
        .context("Failed to run sc.exe")?;
    if !status.success() {
        bail!("sc.exe start failed");
    }
    Ok(())
}

pub fn stop_service() -> anyhow::Result<()> {
    let status = Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .status()
        .context("Failed to run sc.exe")?;
    if !status.success() {
        bail!("sc.exe stop failed");
    }
    Ok(())
}

pub fn service_status() -> anyhow::Result<String> {
    let output = Command::new("sc.exe")
        .args(["query", SERVICE_NAME])
        .output()
        .context("Failed to run sc.exe")?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Set executable permission (no-op on Windows — .exe files are executable by default)
pub fn set_executable_permission(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// Swap a running binary on Windows.
/// Cannot rename a running .exe, so spawn a subprocess to do it after parent exits.
pub fn swap_running_binary(
    new_path: &Path,
    current_path: &Path,
    _old_path: &Path,
) -> anyhow::Result<()> {
    let pid = std::process::id();

    // Spawn the new binary with swap-exe command to complete the swap
    info!("🔄 Spawning swap-exe subprocess (PID {} will exit)", pid);
    Command::new(new_path)
        .args([
            "swap-exe",
            "--target",
            &current_path.to_string_lossy(),
            "--pid",
            &pid.to_string(),
        ])
        .spawn()
        .context("Failed to spawn swap-exe subprocess")?;

    // Parent will exit shortly after this returns (triggered by update handler)
    Ok(())
}

/// Execute binary swap after parent exits (Windows-specific subprocess mode)
pub fn execute_swap(target: std::path::PathBuf, pid: u32) -> anyhow::Result<()> {
    eprintln!("Exiv swap-exe: waiting for PID {} to exit...", pid);

    // Poll until parent PID is gone (up to 30 seconds)
    for _ in 0..60 {
        if !is_process_alive(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    if is_process_alive(pid) {
        bail!("Parent process {} did not exit within 30 seconds", pid);
    }

    eprintln!("Exiv swap-exe: parent exited, performing binary swap...");

    let current_exe = std::env::current_exe().context("Cannot determine current exe path")?;

    let old_path = target.with_extension("old.exe");

    // Remove previous backup
    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }

    // target.exe → target.old.exe
    std::fs::rename(&target, &old_path)
        .with_context(|| format!("Failed to backup: {}", target.display()))?;

    // current (new) exe → target.exe
    std::fs::copy(&current_exe, &target)
        .with_context(|| format!("Failed to install new binary: {}", target.display()))?;

    eprintln!("Exiv swap-exe: binary updated. Restarting service...");

    // Try to restart the service
    let _ = Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .status();

    Ok(())
}

/// Check if a process is alive by PID (Windows)
fn is_process_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.contains(&pid.to_string())
        })
        .unwrap_or(false)
}
