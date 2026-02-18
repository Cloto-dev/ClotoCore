use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::oneshot;

pub(crate) struct PythonBridgeState {
    pub(crate) process: Option<PythonProcessHandle>,
    pub(crate) dynamic_manifest: Option<exiv_shared::PluginManifest>,
    pub(crate) allowed_permissions: Vec<exiv_shared::Permission>,
    pub(crate) http_client: Option<Arc<dyn exiv_shared::NetworkCapability>>,
    // Bug #8: Use u64 for call IDs to prevent negative values and double the range
    pub(crate) pending_calls: HashMap<u64, oneshot::Sender<anyhow::Result<serde_json::Value>>>,
    pub(crate) next_call_id: u64,
    pub(crate) event_tx: Option<tokio::sync::mpsc::Sender<exiv_shared::ExivEventData>>,
    pub(crate) restart_count: u32,
    pub(crate) last_restart: Option<std::time::Instant>,
}

pub(crate) struct PythonProcessHandle {
    #[allow(dead_code)] // Held to keep subprocess alive; dropping would kill it
    pub(crate) child: Child,
    pub(crate) stdin: tokio::process::ChildStdin,
    #[allow(dead_code)] // Held to keep stdout reader task alive
    pub(crate) reader_handle: tokio::task::JoinHandle<()>,
}

impl PythonBridgeState {
    pub(crate) fn new() -> Self {
        Self {
            process: None,
            dynamic_manifest: None,
            allowed_permissions: vec![],
            http_client: None,
            pending_calls: HashMap::new(),
            next_call_id: 1u64,
            event_tx: None,
            restart_count: 0,
            last_restart: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_new_defaults() {
        let state = PythonBridgeState::new();
        assert!(state.process.is_none());
        assert!(state.dynamic_manifest.is_none());
        assert!(state.allowed_permissions.is_empty());
        assert!(state.http_client.is_none());
        assert!(state.pending_calls.is_empty());
        assert_eq!(state.next_call_id, 1);
        assert!(state.event_tx.is_none());
        assert_eq!(state.restart_count, 0);
        assert!(state.last_restart.is_none());
    }

    #[test]
    fn test_call_id_starts_at_one() {
        let state = PythonBridgeState::new();
        // ID 0 is reserved for invalid/unset, so first valid ID is 1
        assert_eq!(state.next_call_id, 1u64);
        assert_ne!(state.next_call_id, 0u64);
    }

    #[test]
    fn test_pending_calls_insert_and_remove() {
        let mut state = PythonBridgeState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.pending_calls.insert(1, tx);
        assert_eq!(state.pending_calls.len(), 1);

        let removed = state.pending_calls.remove(&1);
        assert!(removed.is_some());
        assert!(state.pending_calls.is_empty());
    }

    #[test]
    fn test_pending_calls_drain_on_crash() {
        let mut state = PythonBridgeState::new();
        for i in 1..=5 {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            state.pending_calls.insert(i, tx);
        }
        assert_eq!(state.pending_calls.len(), 5);

        // Simulate crash cleanup: drain all and send errors
        for (_, tx) in state.pending_calls.drain() {
            let _ = tx.send(Err(anyhow::anyhow!("Python process crashed")));
        }
        assert!(state.pending_calls.is_empty());
    }
}
