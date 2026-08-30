//! Bridge to `cloto-installer`, the marketplace install engine that runs as
//! a subprocess (`tools/cloto-installer/`).
//!
//! The engine fetches a connector archive, verifies it, extracts it, builds
//! its environment and decides its seal; the kernel keeps request handling,
//! the private-address policy for downloads, `uv` provisioning, virtualenv
//! resolution and the database registration. Each stage reads one JSON
//! document on stdin and writes progress events on stdout — one JSON object
//! per line in the shape of [`SetupProgressEvent`], which this module
//! forwards unchanged — ending with a `Result` line that carries the stage's
//! answer as data.
//!
//! The binary ships beside the kernel and is checked before use: a missing
//! or stale engine stops the marketplace path with an explicit error rather
//! than degrading silently. `CLOTO_INSTALLER` overrides the location for
//! development checkouts and tests.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, warn};

use crate::handlers::setup::{emit, SetupProgressEvent};

/// Environment variable naming the engine binary, for development checkouts
/// and tests. When set, a `dev`-stamped build is accepted.
pub const ENV_OVERRIDE: &str = "CLOTO_INSTALLER";

/// The engine version the kernel requires: its own. Both are built from one
/// release commit, and the stage contract is not versioned separately.
pub const EXPECTED_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Bound on `cloto-installer version`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many trailing stderr lines a failure report carries.
const STDERR_TAIL: usize = 8;

/// The name of the engine binary beside the kernel.
#[must_use]
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "cloto-installer.exe"
    } else {
        "cloto-installer"
    }
}

/// Where the engine is, and whether the location came from
/// [`ENV_OVERRIDE`].
#[must_use]
pub fn locate() -> (PathBuf, bool) {
    if let Ok(value) = std::env::var(ENV_OVERRIDE) {
        let value = value.trim();
        if !value.is_empty() {
            return (PathBuf::from(value), true);
        }
    }
    (crate::config::exe_dir().join(binary_name()), false)
}

/// Why the engine can or cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallerState {
    /// Present, runs, and reports the version this kernel requires.
    Ready,
    /// No file at the expected location.
    Missing,
    /// Present, but built for another kernel version.
    VersionMismatch,
    /// Present, but could not be run or did not answer as the engine does.
    Unusable,
}

/// The outcome of probing the engine.
#[derive(Debug, Clone, Serialize)]
pub struct InstallerStatus {
    pub state: InstallerState,
    pub path: PathBuf,
    /// The version the binary reported, when it answered.
    pub version: Option<String>,
    pub expected: &'static str,
    /// What to tell the operator when the engine is not ready.
    pub error: Option<String>,
}

impl InstallerStatus {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state == InstallerState::Ready
    }

    fn not_ready(
        state: InstallerState,
        path: PathBuf,
        version: Option<String>,
        error: String,
    ) -> Self {
        Self {
            state,
            path,
            version,
            expected: EXPECTED_VERSION,
            error: Some(error),
        }
    }
}

static LAST_STATUS: RwLock<Option<InstallerStatus>> = RwLock::new(None);

/// The most recent probe result, for the health endpoint. `None` until the
/// engine has been probed once (at boot, or by the first install).
#[must_use]
pub fn last_status() -> Option<InstallerStatus> {
    LAST_STATUS.read().ok().and_then(|guard| guard.clone())
}

/// Locate the engine and run `cloto-installer version`, recording the
/// outcome for [`last_status`].
pub async fn probe() -> InstallerStatus {
    let (path, overridden) = locate();
    let status = probe_at(&path, overridden).await;
    if let Ok(mut guard) = LAST_STATUS.write() {
        *guard = Some(status.clone());
    }
    status
}

async fn probe_at(path: &Path, overridden: bool) -> InstallerStatus {
    if !path.is_file() {
        let hint = if overridden {
            format!(" ({ENV_OVERRIDE} points there)")
        } else {
            "; the ClotoCore installation is incomplete — reinstall ClotoCore".to_string()
        };
        return InstallerStatus::not_ready(
            InstallerState::Missing,
            path.to_path_buf(),
            None,
            format!(
                "marketplace install engine not found at {}{hint}",
                path.display()
            ),
        );
    }

    let mut cmd = tokio::process::Command::new(path);
    cmd.arg("version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut cmd);
    let output = match tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return InstallerStatus::not_ready(
                InstallerState::Unusable,
                path.to_path_buf(),
                None,
                format!(
                    "marketplace install engine at {} could not be run: {e}",
                    path.display()
                ),
            );
        }
        Err(_) => {
            return InstallerStatus::not_ready(
                InstallerState::Unusable,
                path.to_path_buf(),
                None,
                format!(
                    "marketplace install engine at {} did not answer `version` within {}s",
                    path.display(),
                    PROBE_TIMEOUT.as_secs()
                ),
            );
        }
    };

    // `cloto-installer <version> commit=<sha> go=<toolchain> <os>/<arch>`
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut words = stdout.lines().next().unwrap_or_default().split_whitespace();
    let (true, Some("cloto-installer"), Some(version)) =
        (output.status.success(), words.next(), words.next())
    else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return InstallerStatus::not_ready(
            InstallerState::Unusable,
            path.to_path_buf(),
            None,
            format!(
                "marketplace install engine at {} did not identify itself (exit {:?}): {}",
                path.display(),
                output.status.code(),
                first_line(&stdout)
                    .or_else(|| first_line(&stderr))
                    .unwrap_or("no output")
            ),
        );
    };
    let version = version.to_string();

    // A `dev` build carries no version to compare; it is accepted only where
    // a developer put it deliberately (the override, or a checkout).
    let accepted = version == EXPECTED_VERSION
        || (version == "dev" && (overridden || crate::config::is_dev_layout()));
    if !accepted {
        return InstallerStatus::not_ready(
            InstallerState::VersionMismatch,
            path.to_path_buf(),
            Some(version.clone()),
            format!(
                "marketplace install engine at {} is version {version}, this ClotoCore is {EXPECTED_VERSION}; \
                 the installation is inconsistent — reinstall ClotoCore",
                path.display()
            ),
        );
    }
    InstallerStatus {
        state: InstallerState::Ready,
        path: path.to_path_buf(),
        version: Some(version),
        expected: EXPECTED_VERSION,
        error: None,
    }
}

fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}

/// Probe the engine as the first step of a marketplace install, reporting
/// through the progress stream. Returns the engine path, or `None` after a
/// non-recoverable `StepError` — the install must not continue.
pub async fn check_for_install(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
) -> Option<PathBuf> {
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "check_installer".into(),
            description: "Checking the marketplace install engine".into(),
        },
    );
    let status = probe().await;
    match status.error {
        None => {
            emit(
                tx,
                SetupProgressEvent::StepComplete {
                    step: "check_installer".into(),
                },
            );
            Some(status.path)
        }
        Some(error) => {
            error!("{error}");
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "check_installer".into(),
                    error,
                    recoverable: false,
                },
            );
            None
        }
    }
}

/// Run one engine stage (`fetch` / `materialize`): hand it `input` on
/// stdin, forward its progress events to `tx` as they arrive, and return
/// the `Result` line.
///
/// The engine's exit code says whether its answer was positive (0) or
/// negative (2, a `StepError` was already emitted); both return the
/// `Result` line and the caller reads `ok` from it. Any other exit means
/// the stage could not run at all (bad input, an I/O failure) and is an
/// error carrying the engine's stderr.
pub async fn run_stage(
    binary: &Path,
    stage: &str,
    input: &serde_json::Value,
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
) -> anyhow::Result<serde_json::Value> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg(stage)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "failed to start the marketplace install engine ({} {stage}): {e}",
            binary.display()
        )
    })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("install engine stdin was not captured"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("install engine stdout was not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("install engine stderr was not captured"))?;
    let payload = serde_json::to_vec(input)?;

    // The engine reads all of stdin before it writes anything, and its
    // output is drained concurrently, so neither side can fill a pipe and
    // wait on the other.
    let feed = async move {
        if let Err(e) = stdin.write_all(&payload).await {
            // The engine exiting early is reported through its exit code.
            debug!("install engine stdin closed early: {e}");
        }
        let _ = stdin.shutdown().await;
    };
    let events = async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut result = None;
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value)
                    if value.get("type").and_then(serde_json::Value::as_str) == Some("Result") =>
                {
                    result = Some(value);
                }
                Ok(value) => match serde_json::from_value::<SetupProgressEvent>(value) {
                    Ok(event) => emit(tx, event),
                    Err(e) => warn!("install engine: event line not understood ({e}): {line}"),
                },
                Err(e) => warn!("install engine: stdout line is not JSON ({e}): {line}"),
            }
        }
        result
    };
    let diagnostics = async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = std::collections::VecDeque::with_capacity(STDERR_TAIL);
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            // `level: message`, the engine's rendering of the log lines the
            // kernel wrote when this ran in-process.
            match line.split_once(": ") {
                Some(("error", msg)) => error!("install engine: {msg}"),
                Some(("warn", msg)) => warn!("install engine: {msg}"),
                Some(("info", msg)) => info!("install engine: {msg}"),
                _ => info!("install engine: {line}"),
            }
            if tail.len() == STDERR_TAIL {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        tail.into_iter().collect::<Vec<_>>().join("\n")
    };
    let ((), result, stderr_tail) = tokio::join!(feed, events, diagnostics);
    let status = child.wait().await?;

    match (status.code(), result) {
        (Some(0 | 2), Some(result)) => Ok(result),
        (Some(0 | 2), None) => anyhow::bail!(
            "the marketplace install engine ({stage}) exited {} without a result line{}",
            status.code().unwrap_or_default(),
            tail_suffix(&stderr_tail)
        ),
        (code, _) => anyhow::bail!(
            "the marketplace install engine ({stage}) could not run (exit {}){}",
            code.map_or_else(|| "signal".to_string(), |c| c.to_string()),
            tail_suffix(&stderr_tail)
        ),
    }
}

fn tail_suffix(tail: &str) -> String {
    if tail.is_empty() {
        String::new()
    } else {
        format!(": {tail}")
    }
}

#[cfg(windows)]
fn hide_window(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_window(_cmd: &mut tokio::process::Command) {}
