//! MGP Avatar Server — VRM expression and idle behavior control.
//!
//! Runs as a separate process using stdio JSON-RPC transport.
//! Server ID: `output.avatar` (PROJECT_VISION.md §5, Layer 5).
//!
//! Communication flow:
//!   LLM → tools/call → Kernel → stdio → this server
//!   this server → notifications/mgp.event → Kernel → SSE → Dashboard

mod engine;
mod protocol;
mod tools;
mod voicevox;

use protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    // Initialize tracing to stderr (stdout is reserved for JSON-RPC)
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("mgp_avatar=info".parse().unwrap()),
        )
        .init();

    tracing::info!("MGP Avatar Server starting (stdio transport)");

    // Auto-start VOICEVOX Engine if not running
    let config = voicevox::VoicevoxConfig::from_env();
    let engine_guard =
        match engine::VoicevoxEngine::ensure_running(&config.url, config.engine_path.as_deref()) {
            Ok(engine) => Some(engine),
            Err(e) => {
                tracing::warn!("VOICEVOX Engine unavailable: {e}");
                tracing::warn!("speak/synthesize tools will fail until VOICEVOX is started");
                None
            }
        };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("stdin read error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Invalid JSON-RPC: {e}");
                let err_resp = JsonRpcResponse::err(None, -32700, "Parse error");
                write_message(&mut stdout_lock, &err_resp);
                continue;
            }
        };

        // Notifications (no id) — acknowledge silently
        if request.id.is_none() {
            tracing::debug!("Received notification: {}", request.method);
            continue;
        }

        let (response, notifications) = dispatch(&request);
        // Emit notifications BEFORE the response so the kernel receives them in order
        for notif in &notifications {
            write_message(&mut stdout_lock, notif);
        }
        write_message(&mut stdout_lock, &response);
    }

    // engine_guard is dropped here, which shuts down VOICEVOX if we started it
    drop(engine_guard);
    tracing::info!("MGP Avatar Server shutting down");
}

fn dispatch(request: &JsonRpcRequest) -> (JsonRpcResponse, Vec<JsonRpcNotification>) {
    match request.method.as_str() {
        "initialize" => (handle_initialize(request), vec![]),
        "tools/list" => (handle_tools_list(request), vec![]),
        "tools/call" => handle_tools_call(request),
        _ => (
            JsonRpcResponse::err(
                request.id.clone(),
                -32601,
                format!("Method not found: {}", request.method),
            ),
            vec![],
        ),
    }
}

fn handle_initialize(request: &JsonRpcRequest) -> JsonRpcResponse {
    tracing::info!("Handling initialize");

    JsonRpcResponse::ok(
        request.id.clone(),
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "mgp-avatar",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(request: &JsonRpcRequest) -> JsonRpcResponse {
    let tools: Vec<Value> = tools::tool_list()
        .into_iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();

    JsonRpcResponse::ok(
        request.id.clone(),
        json!({
            "tools": tools
        }),
    )
}

fn handle_tools_call(request: &JsonRpcRequest) -> (JsonRpcResponse, Vec<JsonRpcNotification>) {
    let Some(params) = &request.params else {
        return (
            JsonRpcResponse::err(request.id.clone(), -32602, "Missing params"),
            vec![],
        );
    };

    let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) else {
        return (
            JsonRpcResponse::err(request.id.clone(), -32602, "Missing tool name"),
            vec![],
        );
    };

    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    match tools::execute(tool_name, &args) {
        Ok((result, notifications)) => (
            JsonRpcResponse::ok(request.id.clone(), result),
            notifications,
        ),
        Err(msg) => (
            JsonRpcResponse::err(request.id.clone(), -32000, msg),
            vec![],
        ),
    }
}

fn write_message<W: Write, T: serde::Serialize>(writer: &mut W, msg: &T) {
    if let Ok(json_str) = serde_json::to_string(msg) {
        let _ = writeln!(writer, "{json_str}");
        let _ = writer.flush();
    }
}
