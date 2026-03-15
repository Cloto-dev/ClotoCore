//! MGP Tier 3 — Event Subscriptions & Callbacks (§13).
//!
//! Manages event subscription routing, in-memory event buffering for replay,
//! and callback request/response flow.

use super::mcp::McpClientManager;
use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, info};

// ============================================================
// Event Subscriptions
// ============================================================

#[derive(Debug, Clone)]
pub struct EventSubscription {
    pub id: String,
    pub server_id: String,
    pub channels: Vec<String>,
    pub filter: Option<Value>,
}

#[derive(Debug, Clone)]
struct BufferedEvent {
    seq: u64,
    channel: String,
    data: Value,
    timestamp: String,
}

const MAX_BUFFER_SIZE: usize = 1000;

// ============================================================
// Callbacks
// ============================================================

#[derive(Debug)]
struct PendingCallback {
    server_id: String,
    callback_type: String,
    message: String,
    options: Option<Vec<String>>,
    created_at: Instant,
    responded: bool,
    /// Stored response for §13.4 dedup re-send
    recorded_response: Option<String>,
}

// ============================================================
// EventManager
// ============================================================

pub struct EventManager {
    pub(super) subscriptions: Mutex<HashMap<String, EventSubscription>>,
    event_buffer: Mutex<VecDeque<BufferedEvent>>,
    event_seq: AtomicU64,
    callbacks: Mutex<HashMap<String, PendingCallback>>,
}

impl EventManager {
    pub fn new() -> Self {
        Self {
            subscriptions: Mutex::new(HashMap::new()),
            event_buffer: Mutex::new(VecDeque::new()),
            event_seq: AtomicU64::new(0),
            callbacks: Mutex::new(HashMap::new()),
        }
    }

    /// Add a new event subscription.
    pub fn add_subscription(&self, sub: EventSubscription) -> String {
        let id = sub.id.clone();
        let mut subs = self.subscriptions.lock().unwrap();
        subs.insert(id.clone(), sub);
        id
    }

    /// Remove an event subscription.
    pub fn remove_subscription(&self, subscription_id: &str) -> bool {
        let mut subs = self.subscriptions.lock().unwrap();
        subs.remove(subscription_id).is_some()
    }

    /// Get all subscriptions that match a given channel.
    pub fn matching_subscriptions(&self, channel: &str) -> Vec<EventSubscription> {
        let subs = self.subscriptions.lock().unwrap();
        subs.values()
            .filter(|s| s.channels.iter().any(|c| c == channel || c == "*"))
            .cloned()
            .collect()
    }

    /// Buffer an event for replay capability. Returns (seq, timestamp).
    pub fn buffer_event(&self, channel: &str, data: &Value) -> (u64, String) {
        let seq = self.event_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let timestamp = chrono::Utc::now().to_rfc3339();
        let mut buffer = self.event_buffer.lock().unwrap();
        buffer.push_back(BufferedEvent {
            seq,
            channel: channel.to_string(),
            data: data.clone(),
            timestamp: timestamp.clone(),
        });
        // Evict oldest if over limit
        while buffer.len() > MAX_BUFFER_SIZE {
            buffer.pop_front();
        }
        (seq, timestamp)
    }

    /// Replay buffered events with seq > after_seq, up to limit.
    /// If `channels` is provided, only events matching those channels are returned.
    pub fn replay_events(
        &self,
        after_seq: u64,
        limit: usize,
        channels: Option<&[String]>,
    ) -> Vec<(u64, String, Value, String)> {
        let buffer = self.event_buffer.lock().unwrap();
        buffer
            .iter()
            .filter(|e| e.seq > after_seq)
            .filter(|e| channels.is_none_or(|chs| chs.iter().any(|c| c == &e.channel || c == "*")))
            .take(limit)
            .map(|e| {
                (
                    e.seq,
                    e.channel.clone(),
                    e.data.clone(),
                    e.timestamp.clone(),
                )
            })
            .collect()
    }

    /// Return the minimum buffered sequence number (for truncation detection).
    pub fn min_buffered_seq(&self) -> Option<u64> {
        self.event_buffer.lock().unwrap().front().map(|e| e.seq)
    }

    /// Register a pending callback request.
    /// Returns false if callback_id already exists (deduplication).
    pub fn register_callback(
        &self,
        callback_id: &str,
        server_id: &str,
        callback_type: &str,
        message: &str,
        options: Option<Vec<String>>,
    ) -> bool {
        let mut cbs = self.callbacks.lock().unwrap();
        if cbs.contains_key(callback_id) {
            return false; // dedup
        }
        cbs.insert(
            callback_id.to_string(),
            PendingCallback {
                server_id: server_id.to_string(),
                callback_type: callback_type.to_string(),
                message: message.to_string(),
                options,
                created_at: Instant::now(),
                responded: false,
                recorded_response: None,
            },
        );
        true
    }

    /// Mark a callback as responded and return the server_id for routing.
    pub fn resolve_callback(&self, callback_id: &str, response: &str) -> Option<String> {
        let mut cbs = self.callbacks.lock().unwrap();
        let cb = cbs.get_mut(callback_id)?;
        if cb.responded {
            return None; // already responded
        }
        cb.responded = true;
        cb.recorded_response = Some(response.to_string());
        Some(cb.server_id.clone())
    }

    /// Get the recorded response for a previously responded callback (§13.4 dedup re-send).
    pub fn get_recorded_response(&self, callback_id: &str) -> Option<(String, String)> {
        let cbs = self.callbacks.lock().unwrap();
        let cb = cbs.get(callback_id)?;
        if !cb.responded {
            return None;
        }
        cb.recorded_response
            .as_ref()
            .map(|r| (cb.server_id.clone(), r.clone()))
    }

    /// Check if a callback type is llm_completion (§13.4).
    pub fn is_llm_completion(&self, callback_id: &str) -> bool {
        let cbs = self.callbacks.lock().unwrap();
        cbs.get(callback_id)
            .is_some_and(|cb| cb.callback_type == "llm_completion")
    }

    /// Return info about all pending (unresponded) callbacks.
    #[allow(clippy::type_complexity)]
    pub fn pending_callbacks(&self) -> Vec<(String, String, String, Option<Vec<String>>, u64)> {
        let cbs = self.callbacks.lock().unwrap();
        cbs.iter()
            .filter(|(_, cb)| !cb.responded)
            .map(|(id, cb)| {
                (
                    id.clone(),
                    cb.server_id.clone(),
                    cb.message.clone(),
                    cb.options.clone(),
                    cb.created_at.elapsed().as_secs(),
                )
            })
            .collect()
    }

    /// Remove callbacks that have been responded to and are older than `timeout`.
    pub fn cleanup_stale_callbacks(&self, timeout: std::time::Duration) -> usize {
        let mut cbs = self.callbacks.lock().unwrap();
        let before = cbs.len();
        cbs.retain(|_, cb| !cb.responded || cb.created_at.elapsed() < timeout);
        before - cbs.len()
    }
}

// ============================================================
// Kernel Tool Executors
// ============================================================

/// Execute mgp.events.subscribe — register event subscription.
pub(super) async fn subscribe(manager: &McpClientManager, args: Value) -> Result<Value> {
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .unwrap_or("kernel");
    let channels: Vec<String> = args
        .get("channels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let filter = args.get("filter").cloned();

    if channels.is_empty() {
        return Err(anyhow::anyhow!("channels must not be empty"));
    }

    let sub_id = format!(
        "sub-{}-{}",
        server_id,
        chrono::Utc::now().timestamp_millis()
    );
    let sub = EventSubscription {
        id: sub_id.clone(),
        server_id: server_id.to_string(),
        channels: channels.clone(),
        filter,
    };
    manager.events.add_subscription(sub);

    info!(
        subscription_id = %sub_id,
        server = %server_id,
        channels = ?channels,
        "Event subscription created"
    );

    Ok(serde_json::json!({
        "subscribed": channels,
        "unsupported": [],
        "subscription_id": sub_id,
    }))
}

/// Execute mgp.events.unsubscribe — remove event subscription.
pub(super) async fn unsubscribe(manager: &McpClientManager, args: Value) -> Result<Value> {
    let sub_id = args
        .get("subscription_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: subscription_id"))?;

    let removed = manager.events.remove_subscription(sub_id);

    Ok(serde_json::json!({
        "subscription_id": sub_id,
        "removed": removed,
    }))
}

/// Execute mgp.events.replay — replay buffered events scoped by subscription (§13.6).
pub(super) async fn replay(manager: &McpClientManager, args: Value) -> Result<Value> {
    let sub_id = args
        .get("subscription_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: subscription_id"))?;
    let after_seq = args
        .get("after_seq")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(100)
        .min(1000) as usize;

    // Look up the subscription's channels to scope the replay
    let channels = {
        let subs = manager.events.subscriptions.lock().unwrap();
        subs.get(sub_id).map(|s| s.channels.clone())
    };
    let channels =
        channels.ok_or_else(|| anyhow::anyhow!("Subscription '{}' not found", sub_id))?;

    let events = manager
        .events
        .replay_events(after_seq, limit + 1, Some(&channels));
    let has_more = events.len() > limit;
    let events_json: Vec<Value> = events
        .into_iter()
        .take(limit)
        .map(|(seq, channel, data, timestamp)| {
            serde_json::json!({
                "_mgp.seq": seq,
                "channel": channel,
                "data": data,
                "timestamp": timestamp,
            })
        })
        .collect();

    let truncated = after_seq > 0
        && manager
            .events
            .min_buffered_seq()
            .is_some_and(|min| min > after_seq + 1);

    Ok(serde_json::json!({
        "subscription_id": sub_id,
        "events": events_json,
        "has_more": has_more,
        "truncated": truncated,
    }))
}

/// Deliver an event to all matching subscribers via `notifications/mgp.event`.
/// Check if event data matches a subscription filter (§8.3).
/// For each key in the filter object, the event data must contain the same value.
fn event_matches_filter(data: &Value, filter: &Value) -> bool {
    let Some(filter_obj) = filter.as_object() else {
        return true;
    };
    let Some(data_obj) = data.as_object() else {
        return false;
    };
    filter_obj.iter().all(|(k, v)| data_obj.get(k) == Some(v))
}

pub(super) async fn deliver_event(manager: &McpClientManager, channel: &str, data: &Value) {
    let (seq, timestamp) = manager.events.buffer_event(channel, data);
    let matching = manager.events.matching_subscriptions(channel);

    let state = manager.state.read().await;
    for sub in matching {
        // Apply subscription filter (§8.3)
        if let Some(ref filter) = sub.filter {
            if !event_matches_filter(data, filter) {
                continue;
            }
        }
        let Some(handle) = state.servers.get(&sub.server_id) else {
            continue;
        };
        if !handle.status.is_operational() {
            continue;
        }
        let Some(client) = &handle.client else {
            continue;
        };
        let params = serde_json::json!({
            "_mgp.seq": seq,
            "subscription_id": sub.id,
            "channel": channel,
            "data": data,
            "timestamp": timestamp,
        });
        if let Err(e) = client
            .send_notification("notifications/mgp.event", Some(params))
            .await
        {
            debug!(
                server = %sub.server_id,
                error = %e,
                "Failed to deliver event"
            );
        }
    }
}

/// Result of handling an incoming callback request.
pub enum CallbackHandleResult {
    /// New callback — emit event to UI.
    NewCallback(Box<cloto_shared::ClotoEventData>),
    /// Duplicate callback with a previously recorded response — re-send it.
    DuplicateWithResponse {
        server_id: String,
        callback_id: String,
        response: String,
    },
    /// Duplicate callback but no response recorded yet.
    DuplicateNoResponse,
}

/// Handle an incoming `notifications/mgp.callback.request` from a server.
/// Returns a `CallbackHandleResult` indicating how the caller should proceed.
pub fn handle_callback_request(
    manager: &McpClientManager,
    server_id: &str,
    params: &Value,
) -> CallbackHandleResult {
    let Some(callback_id) = params.get("callback_id").and_then(|v| v.as_str()) else {
        return CallbackHandleResult::DuplicateNoResponse;
    };
    let callback_type = params
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("generic");
    let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let options = params.get("options").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    });

    let is_new = manager.events.register_callback(
        callback_id,
        server_id,
        callback_type,
        message,
        options.clone(),
    );

    if !is_new {
        debug!(callback_id = %callback_id, "Duplicate callback request — will re-send recorded response if available");
        if let Some((server_id, response)) = manager.events.get_recorded_response(callback_id) {
            return CallbackHandleResult::DuplicateWithResponse {
                server_id,
                callback_id: callback_id.to_string(),
                response,
            };
        }
        return CallbackHandleResult::DuplicateNoResponse;
    }

    info!(
        callback_id = %callback_id,
        server = %server_id,
        callback_type = %callback_type,
        "Callback request received"
    );

    CallbackHandleResult::NewCallback(Box::new(
        cloto_shared::ClotoEventData::McpCallbackRequested {
            callback_id: callback_id.to_string(),
            server_id: server_id.to_string(),
            callback_type: callback_type.to_string(),
            message: message.to_string(),
            options,
        },
    ))
}

/// Execute mgp.callback.respond — respond to a pending callback.
pub(super) async fn respond_to_callback(manager: &McpClientManager, args: Value) -> Result<Value> {
    let callback_id = args
        .get("callback_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: callback_id"))?;
    let response = args
        .get("response")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: response"))?;

    let server_id = manager
        .events
        .resolve_callback(callback_id, response)
        .ok_or_else(|| {
            anyhow::anyhow!("Callback '{}' not found or already responded", callback_id)
        })?;

    let is_llm = manager.events.is_llm_completion(callback_id);
    if is_llm {
        debug!(callback_id = %callback_id, "LLM completion callback — routing response");
    }

    // Send response to the originating server
    let state = manager.state.read().await;
    let handle = state
        .servers
        .get(&server_id)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", server_id))?;
    let client = handle
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not connected", server_id))?;

    let params = serde_json::json!({
        "callback_id": callback_id,
        "response": response,
    });
    client
        .call("mgp/callback/respond", Some(params))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send callback response: {}", e))?;

    info!(
        callback_id = %callback_id,
        server = %server_id,
        "Callback responded"
    );

    Ok(serde_json::json!({
        "callback_id": callback_id,
        "server_id": server_id,
        "status": "responded",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_match() {
        let em = EventManager::new();
        let sub = EventSubscription {
            id: "sub-1".to_string(),
            server_id: "s1".to_string(),
            channels: vec!["lifecycle".to_string(), "tools".to_string()],
            filter: None,
        };
        em.add_subscription(sub);

        let matched = em.matching_subscriptions("lifecycle");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "sub-1");

        let matched2 = em.matching_subscriptions("tools");
        assert_eq!(matched2.len(), 1);

        let matched3 = em.matching_subscriptions("unknown");
        assert!(matched3.is_empty());
    }

    #[test]
    fn wildcard_subscription() {
        let em = EventManager::new();
        let sub = EventSubscription {
            id: "sub-wild".to_string(),
            server_id: "s1".to_string(),
            channels: vec!["*".to_string()],
            filter: None,
        };
        em.add_subscription(sub);

        assert_eq!(em.matching_subscriptions("anything").len(), 1);
        assert_eq!(em.matching_subscriptions("lifecycle").len(), 1);
    }

    #[test]
    fn unsubscribe_removes() {
        let em = EventManager::new();
        em.add_subscription(EventSubscription {
            id: "sub-1".to_string(),
            server_id: "s1".to_string(),
            channels: vec!["lifecycle".to_string()],
            filter: None,
        });
        assert!(em.remove_subscription("sub-1"));
        assert!(!em.remove_subscription("sub-1"));
        assert!(em.matching_subscriptions("lifecycle").is_empty());
    }

    #[test]
    fn buffer_and_replay() {
        let em = EventManager::new();
        let d1 = serde_json::json!({"event": "a"});
        let d2 = serde_json::json!({"event": "b"});
        let d3 = serde_json::json!({"event": "c"});

        let (s1, _) = em.buffer_event("ch1", &d1);
        let (s2, _) = em.buffer_event("ch1", &d2);
        let (_s3, _) = em.buffer_event("ch2", &d3);

        // Replay all (no channel filter)
        let events = em.replay_events(0, 100, None);
        assert_eq!(events.len(), 3);

        // Replay after s1
        let events = em.replay_events(s1, 100, None);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, s2);

        // Replay with limit
        let events = em.replay_events(0, 2, None);
        assert_eq!(events.len(), 2);

        // Replay filtered by channel
        let ch1_only = vec!["ch1".to_string()];
        let events = em.replay_events(0, 100, Some(&ch1_only));
        assert_eq!(events.len(), 2);

        let ch2_only = vec!["ch2".to_string()];
        let events = em.replay_events(0, 100, Some(&ch2_only));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn buffer_eviction() {
        let em = EventManager::new();
        let data = serde_json::json!({"x": 1});
        for _ in 0..1100 {
            em.buffer_event("ch", &data);
        }
        let events = em.replay_events(0, 2000, None);
        assert_eq!(events.len(), MAX_BUFFER_SIZE);
    }

    #[test]
    fn callback_deduplication() {
        let em = EventManager::new();
        assert!(em.register_callback("cb-1", "s1", "confirm", "Are you sure?", None));
        assert!(!em.register_callback("cb-1", "s1", "confirm", "Are you sure?", None));
    }

    #[test]
    fn callback_resolve() {
        let em = EventManager::new();
        em.register_callback("cb-1", "s1", "confirm", "msg", None);

        let server_id = em.resolve_callback("cb-1", "confirmed");
        assert_eq!(server_id, Some("s1".to_string()));

        // Double resolve returns None
        assert!(em.resolve_callback("cb-1", "confirmed").is_none());

        // Recorded response available for re-send
        let recorded = em.get_recorded_response("cb-1");
        assert_eq!(recorded, Some(("s1".to_string(), "confirmed".to_string())));

        // Unknown callback
        assert!(em.resolve_callback("cb-unknown", "x").is_none());
    }

    #[test]
    fn llm_completion_detection() {
        let em = EventManager::new();
        em.register_callback("llm-1", "s1", "llm_completion", "generate", None);
        assert!(em.is_llm_completion("llm-1"));
        em.register_callback("cb-2", "s1", "confirm", "msg", None);
        assert!(!em.is_llm_completion("cb-2"));
    }
}
