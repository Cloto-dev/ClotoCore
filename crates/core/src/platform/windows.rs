use anyhow::{bail, Context};
use std::path::Path;
use std::process::Command;
use tracing::info;

const SERVICE_NAME: &str = "Cloto";

/// Register Cloto as a Windows Service via sc.exe
pub fn install_service(prefix: &Path, _user: Option<&str>) -> anyhow::Result<()> {
    let exe_path = prefix.join("clotocore.exe");

    // bug-454: sc.exe requires each `option=` token to be followed by its value
    // as a SEPARATE argv entry (the space after '=' is mandatory). Concatenating
    // `binPath=<value>` into one token makes sc.exe reject the option, so the
    // create always failed. Split each option into two argv entries.
    let status = Command::new(system32_tool("sc.exe"))
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            &exe_path.display().to_string(),
            "start=",
            "auto",
            "DisplayName=",
            "ClotoCore",
        ])
        .status()
        .context("Failed to run sc.exe (are you running as Administrator?)")?;

    if !status.success() {
        bail!("sc.exe create failed with exit code {:?}", status.code());
    }

    // Configure restart on failure (restart after 5 seconds)
    let _ = Command::new(system32_tool("sc.exe"))
        .args(["failure", SERVICE_NAME, "reset=60", "actions=restart/5000"])
        .status();

    info!("✅ Service registered: {}", SERVICE_NAME);
    info!("   Start with: sc.exe start {}", SERVICE_NAME);
    info!("   Status:     sc.exe query {}", SERVICE_NAME);
    Ok(())
}

/// Remove Cloto Windows Service
/// `ERROR_SERVICE_DOES_NOT_EXIST` — the one `sc.exe delete` failure that means
/// "there was nothing to remove" rather than "the removal did not work".
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

/// Deregister the Windows service.
///
/// `Ok(true)` = a registration was removed, `Ok(false)` = there was none,
/// `Err` = the removal failed.
///
/// Every non-zero exit used to be swallowed as success with a "may not exist"
/// log line. That made an access-denied `sc.exe delete` indistinguishable from
/// a clean uninstall, and the purge executor turned it into a `removed` report
/// and exit 0 while the service stayed registered. `clotocore service
/// uninstall` now surfaces such a failure instead of reporting success.
pub fn uninstall_service() -> anyhow::Result<bool> {
    // Stop if running (ignore errors)
    let _ = Command::new(system32_tool("sc.exe"))
        .args(["stop", SERVICE_NAME])
        .status();

    // Wait briefly for stop
    std::thread::sleep(std::time::Duration::from_secs(2));

    let status = Command::new(system32_tool("sc.exe"))
        .args(["delete", SERVICE_NAME])
        .status()
        .context("Failed to run sc.exe")?;

    if status.success() {
        info!("✅ Service removed: {}", SERVICE_NAME);
        return Ok(true);
    }
    if status.code() == Some(ERROR_SERVICE_DOES_NOT_EXIST) {
        info!("ℹ️  No service registration to remove: {}", SERVICE_NAME);
        return Ok(false);
    }
    bail!(
        "sc.exe delete {} failed with exit code {:?}",
        SERVICE_NAME,
        status.code()
    )
}

pub fn start_service() -> anyhow::Result<()> {
    let status = Command::new(system32_tool("sc.exe"))
        .args(["start", SERVICE_NAME])
        .status()
        .context("Failed to run sc.exe")?;
    if !status.success() {
        bail!("sc.exe start failed");
    }
    Ok(())
}

pub fn stop_service() -> anyhow::Result<()> {
    let status = Command::new(system32_tool("sc.exe"))
        .args(["stop", SERVICE_NAME])
        .status()
        .context("Failed to run sc.exe")?;
    if !status.success() {
        bail!("sc.exe stop failed");
    }
    Ok(())
}

pub fn service_status() -> anyhow::Result<String> {
    let output = Command::new(system32_tool("sc.exe"))
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
    eprintln!("Cloto swap-exe: waiting for PID {} to exit...", pid);

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

    eprintln!("Cloto swap-exe: parent exited, performing binary swap...");

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

    eprintln!("Cloto swap-exe: binary updated. Restarting service...");

    // Try to restart the service
    let _ = Command::new(system32_tool("sc.exe"))
        .args(["start", SERVICE_NAME])
        .status();

    Ok(())
}

/// Start `exe` and return without waiting for it.
///
/// Used by the uninstall handoff (§7): the spawned helper has to outlive this
/// process, which is about to exit so the helper can delete its files.
pub fn spawn_detached(exe: &Path, args: &[String]) -> anyhow::Result<()> {
    Command::new(exe)
        .args(args)
        .spawn()
        .with_context(|| format!("Failed to spawn {}", exe.display()))?;
    Ok(())
}

/// Start `exe` elevated, waiting only for the elevation prompt to be answered.
///
/// There is no Windows-API crate in this workspace, so the prompt goes through
/// PowerShell's `Start-Process -Verb RunAs`. Without `-Wait` on that call, the
/// PowerShell process exits as soon as the elevated process has been created —
/// so waiting for *PowerShell* means waiting for the user's answer, not for the
/// uninstall. A refusal (or a timeout) comes back as an error, which is what
/// lets the caller keep the app running instead of exiting into nothing.
///
/// Each argument is quoted twice, for two different readers. `-ArgumentList` is
/// not an argv: PowerShell joins its elements with spaces into one command line
/// and quotes nothing, so the argument first has to carry the quoting the
/// *child* needs to split it out again (`win_argv_quote` — without it, `--root
/// C:\Program Files\ClotoCore` reached the helper as two arguments and the
/// uninstall died in argument parsing, before it could report anything). That
/// result is then wrapped as a single-quoted PowerShell string so PowerShell
/// itself passes it through untouched.
///
/// `-FilePath` takes a single value rather than a command line, so the
/// executable path needs the PowerShell quoting only.
pub fn spawn_elevated_and_wait(
    exe: &Path,
    args: &[String],
    timeout: std::time::Duration,
) -> anyhow::Result<()> {
    let arg_list = args
        .iter()
        .map(|a| ps_single_quote(&super::win_argv_quote(a)))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$ErrorActionPreference='Stop'; Start-Process -FilePath {} -ArgumentList {arg_list} -Verb RunAs",
        ps_single_quote(&exe.to_string_lossy())
    );

    let mut child = Command::new(system32_tool(r"WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .spawn()
        .context("Failed to run powershell.exe for the elevation prompt")?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child
            .try_wait()
            .context("Failed to await the elevation prompt")?
        {
            Some(status) if status.success() => return Ok(()),
            // The one failure users actually hit: "the operation was cancelled
            // by the user" from a declined UAC dialog. PowerShell's message is
            // localized, so the exit status is all that is reported.
            Some(status) => bail!(
                "The uninstall was not started: the elevation request ended with exit code {:?} \
                 (a declined prompt looks like this). Nothing was removed.",
                status.code()
            ),
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                bail!(
                    "The elevation prompt was not answered within {}s. Nothing was removed.",
                    timeout.as_secs()
                )
            }
            None => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
}

/// Quote a value as a PowerShell single-quoted string.
fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Absolute path to a System32 tool.
///
/// Windows resolves a bare program name against the calling image's directory
/// and the current directory *before* System32. The uninstall helper (§7) is
/// copied to a temp directory and re-launched elevated from there, so a bare
/// `reg.exe` would let anyone who can write that directory execute code with
/// the elevated token. Always name the tool absolutely.
fn system32_tool(exe: &str) -> std::path::PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    Path::new(&root).join("System32").join(exe)
}

/// Delete a registry key and everything under it.
///
/// Returns `true` when a key was deleted and `false` when there was nothing to
/// delete, so an idempotent caller can tell "removed" from "already gone".
///
/// The delete is attempted *first* and the `query` probe only interprets a
/// failure. Probing first conflates "the key is absent" with "the probe was
/// denied", and reported a key that is still present as already gone. Deciding
/// afterwards is unambiguous: if the key cannot be seen either, it was not
/// there; if it can, the delete genuinely failed. `reg.exe`'s error text is
/// localized, so it is never parsed.
pub fn delete_registry_key(key_path: &str) -> anyhow::Result<bool> {
    let reg = system32_tool("reg.exe");

    let deleted = Command::new(&reg)
        .args(["delete", key_path, "/f"])
        .status()
        .context("Failed to run reg.exe delete")?;
    if deleted.success() {
        return Ok(true);
    }

    let probe = Command::new(&reg)
        .args(["query", key_path])
        .output()
        .context("Failed to run reg.exe query")?;
    if probe.status.success() {
        bail!(
            "reg.exe delete {} failed with exit code {:?} and the key is still present",
            key_path,
            deleted.code()
        );
    }
    Ok(false)
}

/// Check if a process is alive by PID (Windows).
///
/// Fails closed: if `tasklist` cannot be run at all — a PATH without System32,
/// AppLocker policy, a trimmed Server Core image — the answer is "alive". The
/// uninstall helper waits on this before it deletes anything, so guessing
/// "gone" would start removing files from under a kernel that is still
/// running. An unusable probe must stall the handoff, not release it.
#[must_use]
pub fn is_process_alive(pid: u32) -> bool {
    let Ok(output) = Command::new(system32_tool("tasklist.exe"))
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
    else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
}
