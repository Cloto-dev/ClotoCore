//! VOICEVOX Engine process lifecycle manager.
//!
//! Auto-starts the VOICEVOX Engine if it's not already running,
//! and shuts it down when mgp-avatar exits.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// VOICEVOX Engine manager — owns the child process (if we started it).
pub struct VoicevoxEngine {
    child: Option<Child>,
    url: String,
}

impl VoicevoxEngine {
    /// Ensure VOICEVOX Engine is running. If already reachable, returns immediately.
    /// Otherwise, spawns the engine from `engine_path` and waits until it's ready.
    pub fn ensure_running(url: &str, engine_path: Option<&str>) -> Result<Self, String> {
        // Check if already running
        if Self::health_check(url) {
            tracing::info!("VOICEVOX Engine already running at {url}");
            return Ok(Self {
                child: None,
                url: url.to_string(),
            });
        }

        // Need to start it
        let engine_path = engine_path
            .map(PathBuf::from)
            .or_else(Self::discover_engine)
            .ok_or_else(|| {
                format!(
                    "VOICEVOX Engine not running at {url} and no engine binary found. \
                     Set VOICEVOX_ENGINE_PATH or install VOICEVOX Engine to data/voicevox/"
                )
            })?;

        if !engine_path.exists() {
            return Err(format!(
                "VOICEVOX Engine binary not found at: {}",
                engine_path.display()
            ));
        }

        tracing::info!("Starting VOICEVOX Engine from {}", engine_path.display());

        // Extract host and port from URL
        let port = url
            .rsplit(':')
            .next()
            .and_then(|s| s.trim_end_matches('/').parse::<u16>().ok())
            .unwrap_or(50021);

        let child = Command::new(&engine_path)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start VOICEVOX Engine: {e}"))?;

        tracing::info!("VOICEVOX Engine spawned (PID: {})", child.id());

        let mut engine = Self {
            child: Some(child),
            url: url.to_string(),
        };

        // Wait for engine to become ready
        if let Err(e) = engine.wait_until_ready(Duration::from_secs(60)) {
            engine.shutdown();
            return Err(e);
        }

        tracing::info!("VOICEVOX Engine ready at {url}");
        Ok(engine)
    }

    /// Discover engine binary in standard locations.
    fn discover_engine() -> Option<PathBuf> {
        let candidates = [
            // Relative to project dir (CLOTO_PROJECT_DIR)
            std::env::var("CLOTO_PROJECT_DIR").ok().map(|d| {
                PathBuf::from(d)
                    .join("data")
                    .join("voicevox")
                    .join("run.exe")
            }),
            // Relative to exe dir
            std::env::current_exe().ok().and_then(|p| {
                p.parent()
                    .map(|p| p.join("data").join("voicevox").join("run.exe"))
            }),
        ];

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                tracing::info!("Discovered VOICEVOX Engine at {}", candidate.display());
                return Some(candidate);
            }
        }

        None
    }

    /// HTTP health check — returns true if VOICEVOX responds.
    fn health_check(url: &str) -> bool {
        let version_url = format!("{url}/version");
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .ok()
            .and_then(|c| c.get(&version_url).send().ok())
            .is_some_and(|r| r.status().is_success())
    }

    /// Poll until engine responds or timeout.
    fn wait_until_ready(&mut self, timeout: Duration) -> Result<(), String> {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(500);

        loop {
            if start.elapsed() > timeout {
                return Err(format!(
                    "VOICEVOX Engine did not become ready within {}s",
                    timeout.as_secs()
                ));
            }

            // Check if process died
            if let Some(ref mut child) = self.child {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!(
                        "VOICEVOX Engine exited unexpectedly with status: {status}"
                    ));
                }
            }

            if Self::health_check(&self.url) {
                return Ok(());
            }

            std::thread::sleep(poll_interval);
        }
    }

    /// Shut down the engine if we own the process.
    pub fn shutdown(&mut self) {
        if let Some(ref mut child) = self.child.take() {
            tracing::info!("Shutting down VOICEVOX Engine (PID: {})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for VoicevoxEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
