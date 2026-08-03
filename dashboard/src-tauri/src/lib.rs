use std::sync::OnceLock;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// bug-342: run a fallible Tauri window operation, logging a warning on failure
/// instead of silently discarding the `Result`. A discarded window-show result
/// previously hid WebView2 data corruption — the backend kept running headless
/// with no signal to the user that the window never appeared.
macro_rules! with_window_log {
    ($op:expr) => {
        if let Err(e) = $op {
            log::warn!("window operation `{}` failed: {e}", stringify!($op));
        }
    };
}

/// The running kernel handle, captured once `start_kernel` succeeds. Kept alive
/// here (the kernel task runs regardless) and used by the app-exit path to drain
/// and reap MCP subprocesses instead of orphaning them (orphan-leak fix, Step 4).
static KERNEL_HANDLE: OnceLock<cloto_core::KernelHandle> = OnceLock::new();

/// Guard so the MCP drain runs at most once across the tray-quit, dashboard
/// shutdown-button, and `RunEvent::ExitRequested` paths.
static MCP_DRAINED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Guard so the coordinated shutdown sequence ([`begin_shutdown`]) runs at most
/// once, no matter which UI path fires it (tray quit / dashboard button).
static SHUTDOWN_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Drain parameters shared by every exit path (an earlier decision: tray quit and the
/// dashboard shutdown button must run the identical sequence).
const DRAIN_GRACE_MS: u64 = 2000;
const DRAIN_GLOBAL_CAP_SECS: u64 = 6;

/// Drain and reap all MCP subprocesses (runs at most once, guarded).
///
/// `app.exit()` / `std::process::exit` skip Rust destructors, so `kill_on_drop`
/// and transport `Drop` never fire on a normal quit — the drain must be explicit.
async fn drain_mcp() {
    use std::sync::atomic::Ordering;
    if MCP_DRAINED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(handle) = KERNEL_HANDLE.get() else {
        return;
    };
    handle
        .mcp_manager
        .drain_all("app exit", DRAIN_GRACE_MS, DRAIN_GLOBAL_CAP_SECS)
        .await;
}

/// Blocking backstop for exit paths that bypass [`begin_shutdown`] (macOS Cmd+Q
/// and any other OS-initiated `RunEvent::ExitRequested`). The async drain is
/// spawned on the Tauri/tokio runtime and the calling (GUI) thread blocks until
/// it completes or a hard cap elapses, so quit stays responsive even if a server
/// ignores SIGTERM. A no-op when the drain already ran.
fn drain_mcp_before_exit() {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    tauri::async_runtime::spawn(async move {
        drain_mcp().await;
        let _ = tx.send(());
    });
    // The recv timeout is the hard ceiling on how long quit may block.
    let _ = rx.recv_timeout(std::time::Duration::from_secs(DRAIN_GLOBAL_CAP_SECS + 2));
}

/// The one safe-shutdown sequence shared by both user-facing exit paths
/// (an earlier decision): the tray-menu Quit and the dashboard's shutdown button.
///
/// Sequence: surface the window and emit `shutdown-started` (the webview shows
/// the shutdown overlay), drain and reap all MCP subprocesses off the GUI
/// thread, signal the in-process kernel to shut down gracefully, then exit the
/// app. Runs at most once; `app.exit` re-enters via `RunEvent::ExitRequested`,
/// where the already-drained guard makes the backstop a no-op.
fn begin_shutdown(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    if SHUTDOWN_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Same UX on both paths: bring the window up so the shutdown overlay is
    // visible even when quitting from the tray with the window hidden.
    if let Some(window) = app.get_webview_window("main") {
        with_window_log!(window.show());
    }
    if let Err(e) = app.emit("shutdown-started", ()) {
        log::warn!("failed to emit shutdown-started: {e}");
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        drain_mcp().await;
        if let Some(handle) = KERNEL_HANDLE.get() {
            handle.shutdown.notify_waiters();
            // Give the kernel's HTTP server and background tasks a beat to wind
            // down (and the overlay a beat to be seen) before the process exits.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        app.exit(0);
    });
}

/// Name of the environment variable WebView2's loader reads for extra browser
/// arguments. We honour it ourselves because the loader does not: once a host
/// passes `AdditionalBrowserArguments` explicitly — and this app does, from
/// `tauri.conf.json` — the variable is ignored. Measured on a real install
/// 2026-08-03: the variable reached the process and the port stayed closed.
const WEBVIEW2_EXTRA_ARGS_ENV: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";

/// Append `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` to the window's configured
/// browser arguments, when it is set.
///
/// This exists so the verification harness can drive **the artifact users
/// actually install** — opening `--remote-debugging-port` for a run gives it
/// the DOM: deterministic targeting instead of hard-coded pixel coordinates,
/// and a structural oracle to cross-check the visual one against. Building a
/// separate instrumented artifact was the alternative, and it would have meant
/// verifying something other than what ships.
///
/// Two properties this deliberately keeps:
///
/// * **Closed by default.** With the variable unset the config is returned
///   untouched, byte for byte.
/// * **Appended, never replaced.** The configured arguments carry real settings
///   (`--disable-features=…`, autoplay policy); overwriting them would change
///   how the app behaves under verification, which defeats the point of
///   verifying it.
///
/// The threat model is narrow: setting this variable requires control of the
/// app's environment, and anyone holding that can already replace the binary.
fn verification_browser_args(mut ctx: tauri::Context) -> tauri::Context {
    let Ok(extra) = std::env::var(WEBVIEW2_EXTRA_ARGS_ENV) else {
        return ctx;
    };
    let extra = extra.trim();
    if extra.is_empty() {
        return ctx;
    }
    for window in &mut ctx.config_mut().app.windows {
        window.additional_browser_args = Some(append_browser_args(
            window.additional_browser_args.as_deref(),
            extra,
        ));
    }
    // Visible on purpose: a run with a debug channel open should be
    // identifiable from the log afterwards, not silently indistinguishable
    // from an ordinary one.
    log::warn!("{WEBVIEW2_EXTRA_ARGS_ENV} is set; appending to the window browser arguments");
    ctx
}

/// Join configured browser arguments with the ones the environment adds.
///
/// Split from [`verification_browser_args`] because this is the part that can
/// silently lose settings, and a `tauri::Context` cannot be built in a test.
fn append_browser_args(existing: Option<&str>, extra: &str) -> String {
    match existing.map(str::trim) {
        Some(existing) if !existing.is_empty() => format!("{existing} {extra}"),
        _ => extra.to_string(),
    }
}

/// Run the shared safe-shutdown sequence from the dashboard UI (an earlier decision).
/// The frontend shows the shutdown overlay immediately; the `shutdown-started`
/// event keeps any other window in sync.
#[tauri::command]
fn shutdown_app(app: tauri::AppHandle) {
    begin_shutdown(&app);
}

/// End the process for a shutdown the *kernel* sequenced (bug-499).
///
/// `POST /api/system/shutdown` and the Danger Zone's `/api/system/uninstall`
/// both finish by signalling [`cloto_core::KernelHandle::shutdown`] — and that
/// signal only stops the HTTP server and the kernel's background tasks. Nothing
/// in the kernel can end the process, because the process belongs to this
/// shell: `app.exit(0)` is reachable from [`begin_shutdown`] alone. Until this
/// waiter existed, those two endpoints left the app running with its API gone.
///
/// For an uninstall that is not cosmetic. The detached purge helper is started
/// with this pid and waits 30s for it to exit (`PARENT_EXIT_TIMEOUT`); a parent
/// that never exits means the helper removes **nothing** — deliberately, since
/// deleting a running installation is how targets end up half-removed and
/// locked — and the window sits on the shutdown overlay forever. Measured on a
/// user's machine on 2026-08-03: plan and helper staged, no report, no listening
/// ports, install fully intact, app still up.
///
/// Deliberately *not* [`begin_shutdown`]: the kernel has already drained MCP
/// (and, on the uninstall path, closed the pool) before it signals. Re-draining
/// would spend seconds out of the helper's 30s budget to redo finished work.
fn exit_after_kernel_shutdown(app: &tauri::AppHandle) {
    use tauri::Emitter;

    if !claim_kernel_exit() {
        return;
    }
    if let Err(e) = app.emit("shutdown-started", ()) {
        log::warn!("failed to emit shutdown-started: {e}");
    }
    log::info!("kernel signalled shutdown; exiting the app");
    app.exit(0);
}

/// Decide whether this kernel-signalled shutdown owns the exit, and record that
/// the drain is already done. Split out from [`exit_after_kernel_shutdown`] so
/// the re-entrancy rule — the part that can actually race — is testable without
/// a `tauri::AppHandle`.
///
/// Returns `false` when a UI path claimed the sequence first: that path
/// signalled this very Notify on its way to its own `app.exit`, so exiting here
/// too would race it.
fn claim_kernel_exit() -> bool {
    use std::sync::atomic::Ordering;
    if SHUTDOWN_STARTED.swap(true, Ordering::SeqCst) {
        return false;
    }
    // The kernel drains before it signals, on both endpoints. Claiming this
    // guard keeps the `ExitRequested` backstop from draining an already-drained
    // manager and spending the purge helper's 30s budget on finished work.
    MCP_DRAINED.store(true, Ordering::SeqCst);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// One test, not three: the guards are process-wide statics, so separate
    /// `#[test]` functions would race each other inside the same binary.
    #[test]
    fn kernel_exit_is_claimed_once_and_marks_the_drain_done() {
        SHUTDOWN_STARTED.store(false, Ordering::SeqCst);
        MCP_DRAINED.store(false, Ordering::SeqCst);

        assert!(
            claim_kernel_exit(),
            "the first kernel-signalled shutdown must own the exit — nothing else will end the process"
        );
        assert!(
            MCP_DRAINED.load(Ordering::SeqCst),
            "the kernel drained before signalling; the exit backstop must not drain again"
        );
        assert!(
            !claim_kernel_exit(),
            "a second signal must not re-enter: app.exit is already under way"
        );

        // A UI path that started first owns the sequence, and this waiter — which
        // that path's own notify_waiters() wakes — must stand down.
        SHUTDOWN_STARTED.store(true, Ordering::SeqCst);
        MCP_DRAINED.store(false, Ordering::SeqCst);
        assert!(
            !claim_kernel_exit(),
            "begin_shutdown claimed the sequence; the kernel waiter must not race it"
        );
        assert!(
            !MCP_DRAINED.load(Ordering::SeqCst),
            "standing down must not touch the drain guard the UI path is relying on"
        );
    }

    #[test]
    fn env_browser_args_are_appended_never_substituted() {
        // The configured arguments are real settings, not decoration: this app
        // ships `--disable-features=…` and an autoplay policy. Replacing them
        // would change how the app behaves during the very run meant to verify
        // it — and overwriting is exactly what WebView2's own loader does with
        // this variable, which is why the app has to join them itself.
        assert_eq!(
            append_browser_args(
                Some("--disable-features=msWebOOUI"),
                "--remote-debugging-port=9222"
            ),
            "--disable-features=msWebOOUI --remote-debugging-port=9222"
        );
        assert_eq!(
            append_browser_args(None, "--remote-debugging-port=9222"),
            "--remote-debugging-port=9222"
        );
        assert_eq!(
            append_browser_args(Some("   "), "--remote-debugging-port=9222"),
            "--remote-debugging-port=9222",
            "whitespace-only config must not produce a leading separator"
        );
    }
}

/// Returns the kernel HTTP port (used by frontend to construct API URLs).
#[tauri::command]
fn get_kernel_port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8081)
}

/// Capture the primary screen and return a base64-encoded PNG.
#[tauri::command]
fn capture_screen() -> Result<String, String> {
    use base64::Engine;
    use xcap::Monitor;

    let monitors = Monitor::all().map_err(|e| format!("Failed to enumerate monitors: {}", e))?;
    let primary = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| Monitor::all().ok().and_then(|m| m.into_iter().next()))
        .ok_or_else(|| "No monitor found".to_string())?;

    let image = primary
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {}", e))?;

    let mut buf = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encoding failed: {}", e))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(buf.into_inner()))
}

/// Select a file within the scripts/ directory. Returns a relative path.
#[tauri::command]
fn select_script_file(base_dir: String) -> Result<Option<String>, String> {
    // This is a synchronous helper; the actual dialog is done via tauri-plugin-dialog on the frontend.
    // This command validates a proposed path against security constraints.
    let path = std::path::Path::new(&base_dir);
    if !path.exists() || !path.is_dir() {
        return Err(format!("Directory does not exist: {}", base_dir));
    }
    Ok(None)
}

/// Read a text file and return its contents.
#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

// ── Language Pack Management ──

/// Resolve the languages directory path, creating it if needed.
fn get_languages_dir_path() -> Result<std::path::PathBuf, String> {
    let home = if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    }
    .map_err(|_| "Cannot determine home directory".to_string())?;

    let dir = std::path::PathBuf::from(home)
        .join("Documents")
        .join("ClotoCore")
        .join("languages");

    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Return the path to `Documents/ClotoCore/languages`, creating it if needed.
#[tauri::command]
fn get_languages_dir() -> Result<String, String> {
    get_languages_dir_path().map(|p| p.to_string_lossy().into_owned())
}

/// Scan the languages directory and return all .json files as (filename, content) pairs.
#[tauri::command]
fn scan_languages_dir() -> Result<Vec<(String, String)>, String> {
    let dir = get_languages_dir_path()?;
    let mut results = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                results.push((name, content));
            }
        }
    }
    Ok(results)
}

/// bug-460: language-pack filenames arrive from imported pack JSON (`pack.code`),
/// which is fully attacker-controlled. Reject anything that is not a bare filename
/// so a crafted `code` cannot escape the languages directory (`..`, `/`, `\`, or an
/// absolute path that `join` would use to replace the base) and write arbitrary
/// content to an arbitrary path. Language codes (e.g. "ja", "en-US", "zh_Hans")
/// fit the conservative slug comfortably.
fn safe_language_pack_path(
    dir: &std::path::Path,
    filename: &str,
) -> Result<std::path::PathBuf, String> {
    let ok = !filename.is_empty()
        && filename != "."
        && filename != ".."
        && !filename.contains("..")
        && filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !ok {
        return Err(format!("Invalid language pack name: {filename:?}"));
    }
    let path = dir.join(format!("{filename}.json"));
    // Defense in depth: the resolved path must stay a direct child of `dir`.
    if path.parent() != Some(dir) {
        return Err(format!("Invalid language pack name: {filename:?}"));
    }
    Ok(path)
}

/// Save a language pack JSON file to the languages directory.
#[tauri::command]
fn save_language_pack(filename: String, content: String) -> Result<(), String> {
    let dir = get_languages_dir_path()?;
    let path = safe_language_pack_path(&dir, &filename)?;
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// Remove a language pack file from the languages directory.
#[tauri::command]
fn remove_language_pack(filename: String) -> Result<(), String> {
    let dir = get_languages_dir_path()?;
    let path = safe_language_pack_path(&dir, &filename)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Install or update bundled default language packs.
///
/// Uses a snapshot file (`.ja.bundled`) to detect user edits:
/// - If `ja.json` matches the snapshot → user hasn't edited → safe to overwrite
/// - If `ja.json` differs from the snapshot → user customized → skip update
/// - If `ja.json` or snapshot doesn't exist → fresh install → write both
///
/// Returns the number of packs installed or updated.
const DEFAULT_JA_PACK: &str = include_str!("../resources/ja.json");
#[tauri::command]
fn install_default_packs() -> Result<u32, String> {
    let dir = get_languages_dir_path()?;
    let mut installed = 0u32;

    let ja_path = dir.join("ja.json");
    let snapshot_path = dir.join(".ja.bundled");

    let needs_write = if ja_path.exists() {
        let existing = std::fs::read_to_string(&ja_path).unwrap_or_default();
        let snapshot = std::fs::read_to_string(&snapshot_path).unwrap_or_default();

        if existing.trim() == DEFAULT_JA_PACK.trim() {
            // Already up to date
            false
        } else if !snapshot_path.exists() || existing.trim() == snapshot.trim() {
            // No snapshot (pre-update install) or file matches snapshot → user hasn't edited
            true
        } else {
            // File differs from snapshot → user has customized, skip
            false
        }
    } else {
        true
    };

    if needs_write {
        std::fs::write(&ja_path, DEFAULT_JA_PACK).map_err(|e| e.to_string())?;
        std::fs::write(&snapshot_path, DEFAULT_JA_PACK).map_err(|e| e.to_string())?;
        installed += 1;
    }

    Ok(installed)
}

/// Returns the API key for dashboard use. Reads the process environment,
/// which the kernel keeps current across rotations (POST /system/regenerate-key
/// updates CLOTO_API_KEY in place; kernel and shell share this process).
#[tauri::command]
fn get_auto_api_key() -> Option<String> {
    std::env::var("CLOTO_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ── Channel-aware updater (docs/RELEASE_PIPELINE_DESIGN.md §5.1) ──

/// Channels served by the signed `updater-feed` rolling release. Names come
/// from the Release Lifecycle Standard tiers verbatim.
const UPDATE_CHANNELS: [&str; 3] = ["stable", "current", "experimental"];

/// Build an updater pointed at the requested channel view of the feed. The
/// channel string arrives from the frontend (localStorage) and is validated
/// against the closed channel set before it touches a URL.
fn channel_updater(
    app: &tauri::AppHandle,
    channel: &str,
) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    if !UPDATE_CHANNELS.contains(&channel) {
        return Err(format!("Unknown update channel: {channel}"));
    }
    let endpoint: tauri::Url = format!(
        "https://github.com/Cloto-dev/ClotoCore/releases/download/updater-feed/{channel}.json"
    )
    .parse()
    .map_err(|e| format!("{e}"))?;
    app.updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckResult {
    available: bool,
    current_version: String,
    latest_version: String,
    release_date: Option<String>,
    release_notes: Option<String>,
}

/// Check the given channel for an update. Signature verification of the
/// downloaded artifact still happens inside the plugin against the pubkey in
/// `tauri.conf.json`; only the endpoint is channel-selected at runtime.
#[tauri::command]
async fn updater_check(
    app: tauri::AppHandle,
    channel: String,
) -> Result<UpdateCheckResult, String> {
    let updater = channel_updater(&app, &channel)?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    Ok(match update {
        Some(u) => UpdateCheckResult {
            available: true,
            current_version: u.current_version.clone(),
            latest_version: u.version.clone(),
            release_date: u.date.map(|d| {
                d.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| d.to_string())
            }),
            release_notes: u.body.clone(),
        },
        None => UpdateCheckResult {
            available: false,
            current_version: current.clone(),
            latest_version: current,
            release_date: None,
            release_notes: None,
        },
    })
}

/// Download and install the update available on the given channel. Returns a
/// human-readable summary; the frontend performs the relaunch (on Windows the
/// NSIS installer may kill the process first — same contract as before).
#[tauri::command]
async fn updater_download_and_install(
    app: tauri::AppHandle,
    channel: String,
) -> Result<String, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    let updater = channel_updater(&app, &channel)?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No update available".to_string())?;
    let downloaded = AtomicU64::new(0);
    let total = AtomicU64::new(0);
    update
        .download_and_install(
            |chunk, content_length| {
                downloaded.fetch_add(chunk as u64, Ordering::Relaxed);
                if let Some(len) = content_length {
                    total.store(len, Ordering::Relaxed);
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    let size = match total.load(Ordering::Relaxed) {
        0 => downloaded.load(Ordering::Relaxed),
        n => n,
    };
    #[allow(clippy::cast_precision_loss)]
    let size_mb = size as f64 / 1_048_576.0;
    Ok(format!(
        "Updated to v{} ({size_mb:.1} MB). Restarting…",
        update.version
    ))
}

/// Headless release-gate self-check, invoked by `app --smoke`.
///
/// Boots the **real** kernel (config load, SQLite open + migrations, plugin / MCP
/// manager init, HTTP server bind) and then asserts the no-auth liveness route
/// actually serves `200 {"status":"ok"}` before exiting. No Tauri event loop, no
/// window, no display — so it runs unmodified on a headless CI runner / VM and the
/// **exit code is the assertion** (no flaky GUI screenshotting). This is the
/// cross-platform release gate's keystone: it proves the shipped binary *launches and
/// wires up* on the target OS, not merely that it compiled. See
/// `bin/docs/harness-engineering-doctrine.md` §7 (FACET 2) / §2 (tier-doctrine).
///
/// Hermetic by construction — and by the CLAUDE.md Destructive-DB rule, the smoke must
/// never touch real state: it points `DATABASE_URL` at a throwaway temp DB, so it runs
/// migrations from scratch (catching cross-platform migration breakage such as the
/// SQLx CRLF-checksum class) and starts with zero registered MCP servers to spawn.
///
/// Returns a process exit code: `0` healthy; non-zero identifies the failed stage
/// (`10` runtime, `2` kernel boot/wiring, `3` health route never OK).
fn run_smoke() -> i32 {
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

    // Isolate state: a fresh temp DB per run, keyed by PID. Never touch the user's
    // real DB (2026-04-21 incident: a destructive op on the dev DB lost ~30 min of
    // unrecoverable state). A from-scratch DB also exercises the migration path.
    let tmp = std::env::temp_dir().join(format!("clotocore-smoke-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let db_path = tmp.join("cloto_smoke.db");
    std::env::set_var("DATABASE_URL", format!("sqlite:{}", db_path.display()));

    // Use a smoke-specific port unless one was pinned, so a dev instance already on
    // the default 8081 does not make the smoke bind-fail (and vice versa).
    if std::env::var("PORT").is_err() {
        std::env::set_var("PORT", "8097");
    }
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8097);

    // Mirror the GUI boot's key handling so admin-gated wiring initializes the same way.
    if std::env::var("CLOTO_API_KEY").is_err() {
        std::env::set_var("CLOTO_API_KEY", cloto_core::apikey::generate());
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("SMOKE FAIL [runtime]: could not build async runtime: {e}");
            return 10;
        }
    };

    let exit_code = rt.block_on(async move {
        cloto_core::init_tracing();

        let handle = match cloto_core::start_kernel().await {
            Ok(h) => h,
            Err(e) => {
                eprintln!("SMOKE FAIL [boot]: start_kernel returned an error: {e:#}");
                return 2;
            }
        };

        let deadline = std::time::Instant::now() + TIMEOUT;
        let code = match smoke_health_check(port, deadline).await {
            Ok(()) => {
                println!(
                    "SMOKE OK: kernel booted + GET /api/system/health -> 200 on 127.0.0.1:{port}"
                );
                0
            }
            Err(reason) => {
                eprintln!("SMOKE FAIL [health]: {reason}");
                3
            }
        };
        // Best-effort graceful shutdown; process::exit follows regardless.
        handle.shutdown.notify_one();
        code
    });

    // Best-effort cleanup of the throwaway DB dir on success; on failure it is kept
    // for forensics. Always safe — this is the smoke's own temp dir, never real state.
    if exit_code == 0 {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    exit_code
}

/// Poll the kernel's no-auth liveness route over a raw HTTP/1.0 request until it
/// returns `200` with a `"status":"ok"` body, or `deadline` elapses. Raw TCP (no HTTP
/// client dependency) keeps the smoke's own surface minimal and TLS-free for what is
/// always a loopback call.
async fn smoke_health_check(port: u16, deadline: std::time::Instant) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let addr = format!("127.0.0.1:{port}");
    let req = "GET /api/system/health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    loop {
        // One self-contained attempt; its Err payload is the most recent failure reason.
        let attempt: Result<(), String> = async {
            let mut stream = tokio::net::TcpStream::connect(&addr)
                .await
                .map_err(|e| format!("connect failed: {e}"))?;
            stream
                .write_all(req.as_bytes())
                .await
                .map_err(|e| format!("write failed: {e}"))?;
            let mut buf = Vec::new();
            stream
                .read_to_end(&mut buf)
                .await
                .map_err(|e| format!("read failed: {e}"))?;
            let resp = String::from_utf8_lossy(&buf);
            let status_line = resp.lines().next().unwrap_or("");
            if status_line.contains(" 200") && resp.contains("\"status\":\"ok\"") {
                Ok(())
            } else {
                Err(format!(
                    "unexpected response (status line: {status_line:?})"
                ))
            }
        }
        .await;

        match attempt {
            Ok(()) => return Ok(()),
            Err(last) if std::time::Instant::now() >= deadline => {
                return Err(format!(
                    "health route never returned 200/ok within timeout; last error: {last}"
                ));
            }
            Err(_) => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[allow(clippy::too_many_lines)]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tauri desktop mode: the kernel serves only this machine's WebView, so pin the
    // listener to loopback rather than inheriting a wider BIND_ADDRESS. This narrows
    // reachability; it is not a security boundary (a tunnel or reverse proxy that
    // forwards to loopback still reaches it).
    std::env::set_var("BIND_ADDRESS", "127.0.0.1");

    // Add Tauri WebView origins to CORS allowlist
    let existing_cors = std::env::var("CORS_ORIGINS").unwrap_or_default();
    let tauri_origins = "tauri://localhost,http://tauri.localhost";
    let combined = if existing_cors.is_empty() {
        format!(
            "http://localhost:1420,http://127.0.0.1:1420,{}",
            tauri_origins
        )
    } else {
        format!("{},{}", existing_cors, tauri_origins)
    };
    std::env::set_var("CORS_ORIGINS", combined);

    // Headless release-gate self-check. `app --smoke` boots the kernel, asserts the
    // liveness route serves, and exits with a status code — no window, no display.
    // Placed AFTER the loopback BIND_ADDRESS + CORS setup above so it exercises the
    // same environment the GUI boot would. See harness-engineering-doctrine.md §7.
    if std::env::args().skip(1).any(|a| a == "--smoke") {
        std::process::exit(run_smoke());
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app_handle, _argv, _cwd| {
                // Second instance detected: bring existing window to front
                if let Some(window) = app_handle.get_webview_window("main") {
                    with_window_log!(window.show());
                    with_window_log!(window.unminimize());
                    with_window_log!(window.set_focus());
                }
            },
        ))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_kernel_port,
            capture_screen,
            select_script_file,
            get_auto_api_key,
            read_text_file,
            get_languages_dir,
            scan_languages_dir,
            save_language_pack,
            remove_language_pack,
            install_default_packs,
            shutdown_app,
            updater_check,
            updater_download_and_install
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // --- System Tray ---
            let status_item =
                MenuItem::with_id(app, "status", "Cloto: Online", false, None::<&str>)?;
            let show_item = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Cloto", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &status_item,
                    &PredefinedMenuItem::separator(app)?,
                    &show_item,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_item,
                ],
            )?;

            let mut tray_builder = TrayIconBuilder::new();
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            tray_builder
                .tooltip("ClotoCore")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            with_window_log!(window.show());
                            with_window_log!(window.set_focus());
                        }
                    }
                    "quit" => {
                        // Shared safe-shutdown sequence (an earlier decision): overlay +
                        // MCP drain off the GUI thread + kernel shutdown + exit.
                        // Same path as the dashboard's shutdown button.
                        begin_shutdown(app);
                    }
                    _ => {}
                })
                .build(app)?;

            // --- Global Shortcut: CmdOrCtrl+Shift+E to toggle dashboard ---
            app.global_shortcut()
                .on_shortcut(
                    "CmdOrCtrl+Shift+E",
                    |app_handle: &tauri::AppHandle,
                     _shortcut: &tauri_plugin_global_shortcut::Shortcut,
                     event: tauri_plugin_global_shortcut::ShortcutEvent| {
                        if event.state == ShortcutState::Pressed {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    with_window_log!(window.hide());
                                } else {
                                    with_window_log!(window.show());
                                    with_window_log!(window.set_focus());
                                }
                            }
                        }
                    },
                )
                .ok();

            // --- Launch the Cloto Kernel Server ---
            // Load .env before spawn so we can inspect CLOTO_API_KEY synchronously.
            dotenvy::dotenv().ok();
            // Desktop installs have no cwd `.env` — the persistent admin key
            // lives in the user-data dir (docs/ONBOARDING_MODERNIZATION_DESIGN.md §2).
            if std::env::var("CLOTO_API_KEY").is_err() {
                let _ = dotenvy::from_path(cloto_core::config::data_dir().join(".env"));
            }

            // Initialize kernel tracing (stderr + dev-only daily-rotated file log)
            // before spawning start_kernel so the earliest boot logs are captured.
            cloto_core::init_tracing();

            // Ensure a persistent admin key: reuse the loaded .env key or
            // generate one and persist it to data_dir/.env so the key the user
            // is handed (Setup Wizard / Settings -> Security) survives restarts.
            let _ = cloto_core::apikey::ensure_persistent_key();

            let kernel_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match cloto_core::start_kernel().await {
                    Ok(handle) => {
                        // Kernel ready. Stash the handle so the app-exit path can
                        // drain and reap MCP subprocesses (orphan-leak fix, Step 4);
                        // the kernel task keeps running regardless. Shutdown is also
                        // available via the /api/system/shutdown endpoint.
                        //
                        // Give the kernel's shutdown signal somewhere to land
                        // (bug-499): it stops the HTTP server, and this ends the
                        // process it belongs to. `enable()` registers interest
                        // before the first await point, so a signal that fires
                        // between spawning this task and polling it is still
                        // received — `notify_waiters` only wakes registered
                        // waiters, and a missed one here is an app that never
                        // exits.
                        let exit_signal = handle.shutdown.clone();
                        let exit_app = kernel_app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let notified = exit_signal.notified();
                            tokio::pin!(notified);
                            notified.as_mut().enable();
                            notified.await;
                            exit_after_kernel_shutdown(&exit_app);
                        });
                        let _ = KERNEL_HANDLE.set(handle);
                    }
                    Err(e) => {
                        eprintln!("FATAL: Failed to start Cloto Kernel: {}", e);
                        let msg = format!(
                            "Cloto Kernel failed to start:\n\n{}\n\nThe application will now exit.",
                            e
                        );
                        use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                        kernel_app_handle
                            .dialog()
                            .message(msg)
                            .kind(MessageDialogKind::Error)
                            .title("Cloto Startup Error")
                            .blocking_show();
                        kernel_app_handle.exit(1);
                    }
                }
            });

            // Window starts hidden (visible: false in tauri.conf.json).
            // In release builds, defend against WebView2 ERR_CONNECTION_REFUSED race
            // condition by force-navigating to the embedded assets URL after a delay.
            // In dev builds, Vite is already running so just show the window.
            let show_handle = app.handle().clone();
            std::thread::spawn(move || {
                if cfg!(debug_assertions) {
                    // Dev mode: Vite server is already up, just show
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    if let Some(window) = show_handle.get_webview_window("main") {
                        with_window_log!(window.show());
                    }
                } else {
                    // Release mode: WebView2 may fail initial navigation to
                    // http://tauri.localhost before custom protocol handler is ready.
                    // navigate() works at native WebView2 level — recovers from error pages.
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if let Some(window) = show_handle.get_webview_window("main") {
                        let url_str = if cfg!(target_os = "windows") {
                            "http://tauri.localhost"
                        } else {
                            "tauri://localhost"
                        };
                        if let Ok(url) = url_str.parse() {
                            with_window_log!(window.navigate(url));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        with_window_log!(window.show());
                    }
                }
            });

            Ok(())
        })
        // Intercept window close: minimize to tray instead of quitting
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                with_window_log!(window.hide());
            }
        })
        .build(verification_browser_args(tauri::generate_context!()))
        .expect("error while building tauri application");

    // Run with cleanup on exit
    app.run(|_app_handle, event| match event {
        // Fires before the process tears down (covers macOS Cmd+Q and any other
        // exit path, not just the tray Quit item). Drain MCP subprocesses here
        // too — the once-guard makes a second call after tray Quit a no-op
        // (orphan-leak fix, Step 4).
        tauri::RunEvent::ExitRequested { .. } => {
            drain_mcp_before_exit();
        }
        tauri::RunEvent::Exit => {
            // Clean up stale maintenance file if present
            let maint = cloto_core::config::exe_dir().join(".maintenance");
            let _ = std::fs::remove_file(maint);
        }
        _ => {}
    });
}
