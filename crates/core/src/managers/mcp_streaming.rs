//! MGP Tier 3 — Streaming support (§12).
//!
//! Provides stream chunk assembly, gap detection, cancellation, and pacing
//! for streaming tool calls.

use super::mcp::McpClientManager;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::debug;

/// Tracks per-request stream state for gap detection.
pub(super) struct StreamAssembler {
    trackers: Mutex<HashMap<(String, i64), StreamTracker>>,
}

struct StreamTracker {
    expected_index: u32,
    received_count: u32,
    gaps: Vec<u32>,
}

impl StreamAssembler {
    pub fn new() -> Self {
        Self {
            trackers: Mutex::new(HashMap::new()),
        }
    }

    /// Record a received chunk and check for gaps.
    /// Returns `Some(gap_indices)` if gaps were detected, `None` if in order.
    pub fn record_chunk(&self, server_id: &str, request_id: i64, index: u32) -> Option<Vec<u32>> {
        let mut trackers = self.trackers.lock().unwrap();
        let key = (server_id.to_string(), request_id);
        let tracker = trackers.entry(key).or_insert(StreamTracker {
            expected_index: 0,
            received_count: 0,
            gaps: Vec::new(),
        });

        tracker.received_count += 1;

        if index != tracker.expected_index {
            // Record gap: all indices between expected and received
            let mut gap_indices = Vec::new();
            for i in tracker.expected_index..index {
                tracker.gaps.push(i);
                gap_indices.push(i);
            }
            tracker.expected_index = index + 1;
            if gap_indices.is_empty() {
                None // duplicate (index < expected)
            } else {
                Some(gap_indices)
            }
        } else {
            tracker.expected_index = index + 1;
            None
        }
    }

    /// Check if a chunk is a duplicate (index already received).
    pub fn is_duplicate(&self, server_id: &str, request_id: i64, index: u32) -> bool {
        let trackers = self.trackers.lock().unwrap();
        let key = (server_id.to_string(), request_id);
        trackers
            .get(&key)
            .is_some_and(|t| index < t.expected_index && !t.gaps.contains(&index))
    }

    /// Remove tracking state for a completed or cancelled stream.
    pub fn remove(&self, server_id: &str, request_id: i64) {
        let mut trackers = self.trackers.lock().unwrap();
        trackers.remove(&(server_id.to_string(), request_id));
    }
}

/// Cancel an active streaming tool call by sending `mgp/stream/cancel` RPC to the server (§12.7).
pub(super) async fn cancel_stream(
    manager: &McpClientManager,
    server_id: &str,
    request_id: i64,
    reason: &str,
) -> Result<serde_json::Value> {
    let servers = manager.servers.read().await;
    let handle = servers
        .get(server_id)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", server_id))?;
    let client = handle
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not connected", server_id))?;

    let params = serde_json::json!({
        "request_id": request_id,
        "reason": reason,
    });
    let result = client.call("mgp/stream/cancel", Some(params)).await?;

    debug!(server = %server_id, request_id = %request_id, reason = %reason, "Stream cancel sent");
    Ok(result)
}

/// Send a `notifications/mgp.stream.gap` notification to request retransmission of missing chunks (§12.9).
pub(super) async fn send_gap_notification(
    manager: &McpClientManager,
    server_id: &str,
    request_id: i64,
    missing_indices: Vec<u32>,
) -> Result<()> {
    let servers = manager.servers.read().await;
    let handle = servers
        .get(server_id)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", server_id))?;
    let client = handle
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not connected", server_id))?;

    let params = serde_json::json!({
        "request_id": request_id,
        "missing_indices": missing_indices,
    });
    client
        .send_notification("notifications/mgp.stream.gap", Some(params))
        .await?;

    debug!(server = %server_id, request_id = %request_id, missing = ?missing_indices, "Stream gap notification sent");
    Ok(())
}

/// Send a `notifications/mgp.stream.pace` notification to control server output rate (§12.8).
pub(super) async fn send_pace(
    manager: &McpClientManager,
    server_id: &str,
    request_id: i64,
    max_chunks_per_second: u32,
    reason: Option<&str>,
) -> Result<()> {
    let servers = manager.servers.read().await;
    let handle = servers
        .get(server_id)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", server_id))?;
    if !handle.status.is_operational() {
        return Err(anyhow::anyhow!("Server '{}' not operational", server_id));
    }
    let client = handle
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not connected", server_id))?;

    let params = serde_json::json!({
        "request_id": request_id,
        "max_chunks_per_second": max_chunks_per_second,
        "reason": reason,
    });
    client
        .send_notification("notifications/mgp.stream.pace", Some(params))
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembler_in_order_no_gaps() {
        let asm = StreamAssembler::new();
        assert!(asm.record_chunk("s1", 1, 0).is_none());
        assert!(asm.record_chunk("s1", 1, 1).is_none());
        assert!(asm.record_chunk("s1", 1, 2).is_none());
    }

    #[test]
    fn assembler_detects_gap() {
        let asm = StreamAssembler::new();
        assert!(asm.record_chunk("s1", 1, 0).is_none());
        let gaps = asm.record_chunk("s1", 1, 3);
        assert_eq!(gaps, Some(vec![1, 2]));
    }

    #[test]
    fn assembler_detects_duplicate() {
        let asm = StreamAssembler::new();
        asm.record_chunk("s1", 1, 0);
        asm.record_chunk("s1", 1, 1);
        assert!(asm.is_duplicate("s1", 1, 0));
        assert!(!asm.is_duplicate("s1", 1, 2));
    }

    #[test]
    fn assembler_separate_streams() {
        let asm = StreamAssembler::new();
        asm.record_chunk("s1", 1, 0);
        asm.record_chunk("s1", 2, 0);
        assert!(asm.record_chunk("s1", 1, 1).is_none());
        assert!(asm.record_chunk("s1", 2, 1).is_none());
    }

    #[test]
    fn assembler_remove_cleans_up() {
        let asm = StreamAssembler::new();
        asm.record_chunk("s1", 1, 0);
        asm.remove("s1", 1);
        // After remove, should start fresh
        assert!(asm.record_chunk("s1", 1, 0).is_none());
    }
}
