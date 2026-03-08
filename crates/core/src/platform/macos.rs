use anyhow::{bail, Context};
use std::path::Path;
use std::process::Command;
use tracing::info;

const SERVICE_LABEL: &str = "com.cloto.system";

/// Path to the launchd plist file
fn plist_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", SERVICE_LABEL))
}

/// Generate launchd plist content
fn plist_content(prefix: &Path) -> String {
    let exec_path = prefix.join("cloto_system");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exec}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{prefix}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>DOTENV_PATH</key>
        <string>{prefix}/.env</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{prefix}/logs/cloto.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>{prefix}/logs/cloto.stderr.log</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        exec = exec_path.display(),
        prefix = prefix.display(),
    )
}

/// Register Cloto as a launchd user agent
pub fn install_service(prefix: &Path, _user: Option<&str>) -> anyhow::Result<()> {
    let plist = plist_path();
    let content = plist_content(prefix);

    // Ensure LaunchAgents directory exists
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)
            .context("Failed to create ~/Library/LaunchAgents directory")?;
    }

    // Ensure logs directory exists
    let logs_dir = prefix.join("logs");
    std::fs::create_dir_all(&logs_dir).context("Failed to create logs directory")?;

    info!("Writing launchd plist to {}", plist.display());
    std::fs::write(&plist, &content)
        .with_context(|| format!("Failed to write plist to {}", plist.display()))?;

    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist)
        .status()
        .context("Failed to run launchctl load")?;

    if !status.success() {
        bail!("launchctl load failed with exit code {:?}", status.code());
    }

    info!("Service registered: {}", SERVICE_LABEL);
    info!("   Start with: launchctl start {}", SERVICE_LABEL);
    info!("   Status:     launchctl list {}", SERVICE_LABEL);
    Ok(())
}

/// Remove Cloto launchd user agent
pub fn uninstall_service() -> anyhow::Result<()> {
    let plist = plist_path();

    if plist.exists() {
        // Unload (stops if running)
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist)
            .status();

        std::fs::remove_file(&plist)
            .with_context(|| format!("Failed to remove plist {}", plist.display()))?;
        info!("Service removed: {}", SERVICE_LABEL);
    } else {
        info!("Plist not found, nothing to remove");
    }
    Ok(())
}

pub fn start_service() -> anyhow::Result<()> {
    let status = Command::new("launchctl")
        .args(["start", SERVICE_LABEL])
        .status()
        .context("Failed to run launchctl start")?;
    if !status.success() {
        bail!("launchctl start failed with exit code {:?}", status.code());
    }
    Ok(())
}

pub fn stop_service() -> anyhow::Result<()> {
    let status = Command::new("launchctl")
        .args(["stop", SERVICE_LABEL])
        .status()
        .context("Failed to run launchctl stop")?;
    if !status.success() {
        bail!("launchctl stop failed with exit code {:?}", status.code());
    }
    Ok(())
}

pub fn service_status() -> anyhow::Result<String> {
    let output = Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()
        .context("Failed to run launchctl list")?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Set executable permission on a file (chmod 0o755)
pub fn set_executable_permission(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("Failed to set executable permission on {}", path.display()))?;
    Ok(())
}

/// Swap a running binary (macOS: rename is safe even while running, same as Linux)
pub fn swap_running_binary(
    new_path: &Path,
    current_path: &Path,
    old_path: &Path,
) -> anyhow::Result<()> {
    // Remove previous backup if exists (ignore NotFound)
    match std::fs::remove_file(old_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to remove old backup {}: {}",
                old_path.display(),
                e
            ))
        }
    }

    // current → old (backup)
    std::fs::rename(current_path, old_path).with_context(|| {
        format!(
            "Failed to backup current binary: {}",
            current_path.display()
        )
    })?;

    // new → current (activate)
    std::fs::rename(new_path, current_path).map_err(|e| {
        // Attempt rollback on failure
        match std::fs::rename(old_path, current_path) {
            Ok(()) => anyhow::anyhow!("Failed to install new binary (rolled back): {}", e),
            Err(rb_err) => {
                eprintln!("CRITICAL: Binary install failed and rollback also failed! install_err={}, rollback_err={}", e, rb_err);
                anyhow::anyhow!("Failed to install new binary AND rollback failed: install={}, rollback={}", e, rb_err)
            }
        }
    })?;

    Ok(())
}

/// Execute binary swap (direct rename on macOS — no subprocess needed, same as Linux)
pub fn execute_swap(target: std::path::PathBuf, _pid: u32) -> anyhow::Result<()> {
    info!("swap-exe is a no-op on macOS (rename works on running files)");
    let _ = target;
    Ok(())
}
