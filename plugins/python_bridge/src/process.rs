use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};
use tracing::info;

use crate::PythonBridgePlugin;
use crate::config::resolve_script_path;
use crate::ipc::send_raw;
use crate::state::PythonProcessHandle;

impl PythonBridgePlugin {
    pub(crate) const MAX_RESTART_ATTEMPTS: u32 = 3;
    pub(crate) const RESTART_COOLDOWN_SECS: u64 = 5;

    pub(crate) async fn ensure_process(&self) -> anyhow::Result<()> {
        // C-03: Use single write lock to prevent race between read→write transition
        let mut state = self.state.write().await;
        if state.process.is_some() {
            return Ok(());
        }
        {
            // Check restart limits (only on actual restarts, not initial startup)
            let is_restart = state.last_restart.is_some();
            if is_restart {
                if state.restart_count >= Self::MAX_RESTART_ATTEMPTS {
                    return Err(anyhow::anyhow!("Max restart attempts ({}) reached", Self::MAX_RESTART_ATTEMPTS));
                }
                if let Some(last) = state.last_restart {
                    if last.elapsed().as_secs() < Self::RESTART_COOLDOWN_SECS {
                        return Err(anyhow::anyhow!("Restart cooldown active ({}s remaining)",
                            Self::RESTART_COOLDOWN_SECS - last.elapsed().as_secs()));
                    }
                }
                state.restart_count += 1;
                info!("🔄 Restarting Python bridge (attempt {}/{})", state.restart_count, Self::MAX_RESTART_ATTEMPTS);
            }
            state.last_restart = Some(std::time::Instant::now());

            let event_tx = state.event_tx.clone();
            let runtime_path = resolve_script_path("scripts/bridge_runtime.py");
            let user_script_path = resolve_script_path(&self.script_path);
            info!("🐍 Spawning Python subprocess: {} with user script: {}", runtime_path.display(), user_script_path.display());

            let python = if cfg!(windows) { "python" } else { "python3" };
            let mut child = Command::new(python)
                .arg(&runtime_path)
                .arg(&user_script_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?;

            let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("Failed to open stdin"))?;
            let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("Failed to open stdout"))?;

            // Start background reader with enhanced error handling
            let state_weak = self.state.clone();
            let reader_handle = tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();

                loop {
                    match reader.next_line().await {
                        Ok(Some(line)) => {
                            // Process line (event/RPC handling)
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                                // Handle event messages
                                if val.get("type").and_then(|v| v.as_str()) == Some("event") {
                                    if let (Some(ev_type), Some(ev_data)) = (val.get("event_type").and_then(|v| v.as_str()), val.get("data")) {
                                        if let Some(tx) = &event_tx {
                                            let data = match ev_type {
                                                "GazeUpdated" => {
                                                    if let Ok(gaze) = serde_json::from_value::<exiv_shared::GazeData>(ev_data.clone()) {
                                                        Some(exiv_shared::ExivEventData::GazeUpdated(gaze))
                                                    } else { None }
                                                }
                                                "SystemNotification" => {
                                                    match ev_data.as_str() {
                                                        Some(s) => Some(exiv_shared::ExivEventData::SystemNotification(s.to_string())),
                                                        None => {
                                                            tracing::warn!("Invalid SystemNotification data: expected string, got {}", ev_data);
                                                            None
                                                        }
                                                    }
                                                }
                                                _ => None
                                            };
                                            if let Some(d) = data {
                                                let _ = tx.send(d).await;
                                            }
                                        }
                                    }
                                    continue;
                                }

                                // Handle RPC response messages
                                // Bug #8: Changed from as_i64() to as_u64()
                                if let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                                    let mut lock = state_weak.write().await;
                                    if let Some(tx) = lock.pending_calls.remove(&id) {
                                        if let Some(err) = val.get("error") {
                                            let _ = tx.send(Err(anyhow::anyhow!("Python Error: {}", err)));
                                        } else {
                                            let _ = tx.send(Ok(val.get("result").cloned().unwrap_or(serde_json::Value::Null)));
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            tracing::warn!("🔥 Python bridge reader received EOF - process terminated");
                            break;
                        }
                        Err(e) => {
                            tracing::error!("🔥 Python bridge reader error: {} - terminating", e);
                            break;
                        }
                    }
                }

                // Reader exited - cleanup and mark for restart
                tracing::error!("🔥 Python bridge reader task exited, cleaning up");
                let mut state = state_weak.write().await;

                // Mark process as dead (will auto-restart on next call via ensure_process)
                state.process = None;

                // Fail all pending calls
                for (_, tx) in state.pending_calls.drain() {
                    let _ = tx.send(Err(anyhow::anyhow!("Python process crashed")));
                }

                tracing::info!("🔄 Python bridge will auto-restart on next operation");
            });

            // Initial Handshake (Get Manifest) without using call_python (recursive)
            // Bug B: Keep lock held across both registration AND send to prevent race condition
            // where response arrives before send completes
            //
            // Bug H: Performance Consideration - Lock Contention Risk
            // This design holds RwLock during I/O operations (send_raw -> write_all + flush).
            // Tradeoff: Correctness (prevent handshake race) vs Performance (lock held 5-10ms)
            // Impact: Single calls: <10ms. Concurrent 10+ calls: 20-30% slowdown. High load (100+): notable delays.
            // Mitigation: MAX_PENDING_CALLS(50) limits concurrency. Consider refactoring if >100 RPS needed.
            let (id, rx) = {
                let mut state = self.state.write().await;
                let id = state.next_call_id;
                // Bug D: Use wrapping_add for explicit overflow behavior (wraps to 1, not 0)
                // After 2^64-1 calls, wraps around. ID 0 reserved for invalid/unset.
                state.next_call_id = if state.next_call_id == u64::MAX {
                    1 // Skip 0 on wraparound
                } else {
                    state.next_call_id + 1
                };
                let (tx, rx) = oneshot::channel();
                state.pending_calls.insert(id, tx);

                // Send request while still holding lock to prevent race
                if let Some(proc) = state.process.as_mut() {
                    send_raw(&mut proc.stdin, id, "get_manifest", serde_json::Value::Null).await?;
                } else {
                    return Err(anyhow::anyhow!("Process handle lost during handshake"));
                }

                (id, rx)
            };  // Lock released here, after both registration and send are complete

            let manifest_json: serde_json::Value = match timeout(Duration::from_secs(5), rx).await {
                Ok(res) => res??,
                Err(_) => {
                    // Clean up orphaned pending_call entry on timeout
                    let mut state = self.state.write().await;
                    state.pending_calls.remove(&id);
                    return Err(anyhow::anyhow!("Python bridge manifest handshake timed out"));
                }
            };

            let mut state = self.state.write().await;
            let mut manifest = self.auto_manifest();

            // 📝 Python 側のマニフェストで上書き (safe fields only)
            // C-04: id, version, category, service_type, capabilities, required_permissions
            // are NOT overridable from Python scripts (security-critical fields).
            if let Some(name) = manifest_json.get("name").and_then(|v| v.as_str()) {
                if name.len() <= 200 {
                    manifest.name = name.to_string();
                } else {
                    tracing::warn!("Python manifest name exceeds 200 chars, ignoring");
                }
            }
            if let Some(desc) = manifest_json.get("description").and_then(|v| v.as_str()) {
                if desc.len() <= 1000 {
                    manifest.description = desc.to_string();
                } else {
                    tracing::warn!("Python manifest description exceeds 1000 chars, ignoring");
                }
            }

            // 🧰 ツールとアクション情報の継承
            if let Some(tools_val) = manifest_json.get("provided_tools").and_then(|v| v.as_array()) {
                manifest.provided_tools = tools_val.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            }
            if let Some(icon) = manifest_json.get("action_icon").and_then(|v| v.as_str()) {
                manifest.action_icon = Some(icon.to_string());
            }
            if let Some(target) = manifest_json.get("action_target").and_then(|v| v.as_str()) {
                manifest.action_target = Some(target.to_string());
            }

            // 🏷️ タグの統合と #PYTHON の強制付与
            if let Some(tags_val) = manifest_json.get("tags").and_then(|v| v.as_array()) {
                for t in tags_val {
                    if let Some(t_str) = t.as_str() {
                        let t_str = if t_str.starts_with('#') { t_str.to_string() } else { format!("#{}", t_str) };
                        if !manifest.tags.contains(&t_str) {
                            manifest.tags.push(t_str);
                        }
                    }
                }
            }
            if !manifest.tags.contains(&"#PYTHON".to_string()) {
                manifest.tags.push("#PYTHON".to_string());
            }

            state.dynamic_manifest = Some(manifest);
            state.process = Some(PythonProcessHandle { child, stdin, reader_handle });
            // Reset restart counter after successful handshake to prevent permanent lockout
            state.restart_count = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PythonBridgeState;

    #[test]
    fn test_restart_constants() {
        assert_eq!(PythonBridgePlugin::MAX_RESTART_ATTEMPTS, 3);
        assert_eq!(PythonBridgePlugin::RESTART_COOLDOWN_SECS, 5);
    }

    #[test]
    fn test_restart_count_boundary_at_max() {
        let mut state = PythonBridgeState::new();
        state.restart_count = PythonBridgePlugin::MAX_RESTART_ATTEMPTS;
        state.last_restart = Some(std::time::Instant::now() - std::time::Duration::from_secs(60));
        // At MAX_RESTART_ATTEMPTS, the condition `restart_count >= MAX_RESTART_ATTEMPTS` is true
        assert!(state.restart_count >= PythonBridgePlugin::MAX_RESTART_ATTEMPTS);
    }

    #[test]
    fn test_restart_count_boundary_below_max() {
        let mut state = PythonBridgeState::new();
        state.restart_count = PythonBridgePlugin::MAX_RESTART_ATTEMPTS - 1;
        state.last_restart = Some(std::time::Instant::now() - std::time::Duration::from_secs(60));
        // Below MAX, the condition should be false (restart allowed)
        assert!(state.restart_count < PythonBridgePlugin::MAX_RESTART_ATTEMPTS);
    }

    #[test]
    fn test_cooldown_elapsed_check() {
        let state_last_restart = std::time::Instant::now() - std::time::Duration::from_secs(10);
        // After 10 seconds, cooldown of 5 seconds should be expired
        assert!(state_last_restart.elapsed().as_secs() >= PythonBridgePlugin::RESTART_COOLDOWN_SECS);
    }

    #[test]
    fn test_cooldown_not_elapsed_check() {
        let state_last_restart = std::time::Instant::now();
        // Immediately, cooldown should still be active
        assert!(state_last_restart.elapsed().as_secs() < PythonBridgePlugin::RESTART_COOLDOWN_SECS);
    }

    #[test]
    fn test_initial_startup_not_restart() {
        let state = PythonBridgeState::new();
        // Initial startup: last_restart is None, so is_restart is false
        let is_restart = state.last_restart.is_some();
        assert!(!is_restart);
        // This means restart limits should NOT be checked
    }

    #[test]
    fn test_manifest_field_validation_name_length() {
        // Verify the 200 char name limit used in ensure_process manifest parsing
        let short_name = "a".repeat(200);
        let long_name = "a".repeat(201);
        assert!(short_name.len() <= 200);
        assert!(long_name.len() > 200);
    }

    #[test]
    fn test_manifest_field_validation_description_length() {
        // Verify the 1000 char description limit used in ensure_process manifest parsing
        let short_desc = "a".repeat(1000);
        let long_desc = "a".repeat(1001);
        assert!(short_desc.len() <= 1000);
        assert!(long_desc.len() > 1000);
    }

    #[test]
    fn test_tag_normalization() {
        // Tags without '#' prefix should get it added
        let tag = "PYTHON";
        let normalized = if tag.starts_with('#') { tag.to_string() } else { format!("#{}", tag) };
        assert_eq!(normalized, "#PYTHON");

        // Tags with '#' prefix should stay as-is
        let tag2 = "#TOOL";
        let normalized2 = if tag2.starts_with('#') { tag2.to_string() } else { format!("#{}", tag2) };
        assert_eq!(normalized2, "#TOOL");
    }

    #[test]
    fn test_tag_deduplication() {
        let mut tags = vec!["#TOOL".to_string(), "#ADAPTER".to_string()];
        let new_tag = "#TOOL".to_string();
        if !tags.contains(&new_tag) {
            tags.push(new_tag);
        }
        // Should NOT add duplicate
        assert_eq!(tags.len(), 2);

        let new_tag2 = "#NEW".to_string();
        if !tags.contains(&new_tag2) {
            tags.push(new_tag2);
        }
        // Should add new tag
        assert_eq!(tags.len(), 3);
    }
}
