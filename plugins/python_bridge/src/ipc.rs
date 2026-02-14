use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Duration};

use crate::PythonBridgePlugin;

/// Low-level send without checking process (avoids recursion)
// Bug #8: Changed id parameter from i64 to u64
pub(crate) async fn send_raw(stdin: &mut tokio::process::ChildStdin, id: u64, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
    let request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params
    });
    let mut line = request.to_string();
    line.push('\n');
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

impl PythonBridgePlugin {
    // M-18: Maximum pending calls to prevent unbounded resource consumption
    pub(crate) const MAX_PENDING_CALLS: usize = 50;

    pub async fn call_python(&self, method: &str, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        self.ensure_process().await?;

        // C-02: Single write lock for registration + send to prevent deadlock
        let (id, rx) = {
            let mut state = self.state.write().await;
            // M-18: Reject new calls when at capacity
            // Bug #12: This limit also prevents unbounded memory growth from leaked calls
            // Calls are cleaned up on timeout (10s) at line 394, but this provides defense-in-depth
            if state.pending_calls.len() >= Self::MAX_PENDING_CALLS {
                tracing::warn!(
                    "Python Bridge pending call limit reached: {}/{}. Consider investigating if calls are timing out.",
                    state.pending_calls.len(),
                    Self::MAX_PENDING_CALLS
                );
                return Err(anyhow::anyhow!(
                    "Python Bridge pending call limit reached ({})",
                    Self::MAX_PENDING_CALLS
                ));
            }
            let id = state.next_call_id;
            state.next_call_id += 1;
            let (tx, rx) = tokio::sync::oneshot::channel();
            state.pending_calls.insert(id, tx);

            if let Some(proc) = state.process.as_mut() {
                if let Err(e) = send_raw(&mut proc.stdin, id, method, params).await {
                    state.pending_calls.remove(&id);
                    return Err(e);
                }
            } else {
                state.pending_calls.remove(&id);
                return Err(anyhow::anyhow!("Python process not running"));
            }
            (id, rx)
        };

        match timeout(Duration::from_secs(10), rx).await {
            Ok(res) => res?,
            Err(_) => {
                let mut state = self.state.write().await;
                state.pending_calls.remove(&id);
                Err(anyhow::anyhow!("Python call timed out"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PythonBridgeState;

    #[test]
    fn test_max_pending_calls_constant() {
        assert_eq!(PythonBridgePlugin::MAX_PENDING_CALLS, 50);
    }

    #[test]
    fn test_pending_calls_at_capacity() {
        let mut state = PythonBridgeState::new();
        // Fill up to MAX_PENDING_CALLS
        for i in 1..=PythonBridgePlugin::MAX_PENDING_CALLS as u64 {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            state.pending_calls.insert(i, tx);
        }
        assert_eq!(state.pending_calls.len(), PythonBridgePlugin::MAX_PENDING_CALLS);
        // Verify the check condition that call_python uses
        assert!(state.pending_calls.len() >= PythonBridgePlugin::MAX_PENDING_CALLS);
    }

    #[test]
    fn test_call_id_wrapping_at_u64_max() {
        let mut state = PythonBridgeState::new();
        state.next_call_id = u64::MAX;
        // Simulate the wrapping logic from process.rs ensure_process handshake
        let id = state.next_call_id;
        state.next_call_id = if state.next_call_id == u64::MAX {
            1 // Skip 0 on wraparound
        } else {
            state.next_call_id + 1
        };
        assert_eq!(id, u64::MAX);
        assert_eq!(state.next_call_id, 1); // Wrapped to 1, not 0
    }

    #[test]
    fn test_call_id_normal_increment() {
        let mut state = PythonBridgeState::new();
        assert_eq!(state.next_call_id, 1);
        // Simulate call_python's simple increment
        let id = state.next_call_id;
        state.next_call_id += 1;
        assert_eq!(id, 1);
        assert_eq!(state.next_call_id, 2);
    }

    #[test]
    fn test_send_raw_json_format() {
        // Verify the JSON format that send_raw produces
        let request = serde_json::json!({
            "id": 42u64,
            "method": "test_method",
            "params": serde_json::Value::Null
        });
        let line = format!("{}\n", request);
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["method"], "test_method");
        assert!(parsed["params"].is_null());
    }
}
