//! Avatar control + VOICEVOX TTS tools.
//!
//! Each tool emits `notifications/mgp.event` via stdout so the kernel
//! automatically forwards to the dashboard via SSE.
//!
//! Tools:
//!   set_expression    — VRM facial expression control
//!   set_idle_behavior — Idle animation parameters
//!   speak             — VOICEVOX TTS + lip sync notification
//!   synthesize        — VOICEVOX TTS to WAV file
//!   list_speakers     — Available VOICEVOX speakers
//!   set_speaker       — Change active speaker

use crate::protocol::{JsonRpcNotification, McpTool};
use crate::voicevox::VoicevoxClient;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

static VOICEVOX: OnceLock<VoicevoxClient> = OnceLock::new();

fn voicevox() -> &'static VoicevoxClient {
    VOICEVOX.get_or_init(|| {
        let config = crate::voicevox::VoicevoxConfig::from_env();
        tracing::info!(
            "VOICEVOX client initialized: url={}, speaker={}, speed={}",
            config.url,
            config.default_speaker,
            config.speed
        );
        VoicevoxClient::new(config)
    })
}

/// Build tool schemas for `tools/list`.
pub fn tool_list() -> Vec<McpTool> {
    vec![
        set_expression_schema(),
        set_pose_schema(),
        set_idle_behavior_schema(),
        speak_schema(),
        synthesize_schema(),
        list_speakers_schema(),
        set_speaker_schema(),
    ]
}

/// Execute a tool call. Returns `(result_value, notifications_to_emit)`.
pub fn execute(tool_name: &str, args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    match tool_name {
        "set_expression" => execute_set_expression(args),
        "set_pose" => execute_set_pose(args),
        "set_idle_behavior" => execute_set_idle_behavior(args),
        "speak" => execute_speak(args),
        "synthesize" => execute_synthesize(args),
        "list_speakers" => execute_list_speakers(args),
        "set_speaker" => execute_set_speaker(args),
        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}

// ── Avatar Control Tools (existing) ──

fn set_expression_schema() -> McpTool {
    McpTool {
        name: "set_expression".into(),
        description: "Set a VRM facial expression on the avatar. Available expressions: happy, angry, sad, relaxed, surprised, neutral.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Target agent ID"
                },
                "expression": {
                    "type": "string",
                    "description": "Expression name (happy, angry, sad, relaxed, surprised, neutral)",
                    "enum": ["happy", "angry", "sad", "relaxed", "surprised", "neutral"]
                },
                "intensity": {
                    "type": "number",
                    "description": "Expression intensity from 0.0 to 1.0 (default: 1.0)",
                    "minimum": 0.0,
                    "maximum": 1.0
                }
            },
            "required": ["agent_id", "expression"]
        }),
    }
}

fn set_pose_schema() -> McpTool {
    McpTool {
        name: "set_pose".into(),
        description: "Change the avatar's body pose. Transitions smoothly. Poses: relaxed (default, arms at sides), attentive (forward lean, focused), thinking (hand on chin, head tilt), arms_crossed (arms folded, serious).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Target agent ID"
                },
                "pose": {
                    "type": "string",
                    "description": "Pose name",
                    "enum": ["relaxed", "attentive", "thinking", "arms_crossed"]
                },
                "transition": {
                    "type": "number",
                    "description": "Transition duration in seconds (default: 0.5)",
                    "minimum": 0.1,
                    "maximum": 3.0
                }
            },
            "required": ["agent_id", "pose"]
        }),
    }
}

fn execute_set_pose(args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or("agent_id is required")?;
    let pose = args
        .get("pose")
        .and_then(|v| v.as_str())
        .ok_or("pose is required")?;
    let transition = args
        .get("transition")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.5);

    let notif = JsonRpcNotification::new(
        "notifications/mgp.event",
        Some(json!({
            "channel": "avatar_set_pose",
            "data": {
                "agent_id": agent_id,
                "pose": pose,
                "transition": transition
            }
        })),
    );

    let result = json!({
        "content": [{
            "type": "text",
            "text": format!("Pose set: {} (transition: {:.1}s)", pose, transition)
        }]
    });

    Ok((result, vec![notif]))
}

fn set_idle_behavior_schema() -> McpTool {
    McpTool {
        name: "set_idle_behavior".into(),
        description:
            "Adjust the avatar's idle animation parameters (breathing, sway, blinking, pose)."
                .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Target agent ID"
                },
                "mode": {
                    "type": "string",
                    "description": "Idle mode: relaxed, alert, sleepy",
                    "enum": ["relaxed", "alert", "sleepy"]
                },
                "breathing_rate": {
                    "type": "number",
                    "description": "Breathing rate multiplier (0.5 = slow, 1.0 = normal, 2.0 = fast)",
                    "minimum": 0.1,
                    "maximum": 3.0
                },
                "sway_amplitude": {
                    "type": "number",
                    "description": "Micro-sway amplitude multiplier (0.0 = still, 1.0 = normal)",
                    "minimum": 0.0,
                    "maximum": 2.0
                },
                "blink_frequency": {
                    "type": "number",
                    "description": "Blink frequency multiplier (0.5 = rare, 1.0 = normal, 2.0 = frequent)",
                    "minimum": 0.1,
                    "maximum": 3.0
                },
                "pose": {
                    "type": "object",
                    "description": "Override default pose parameters",
                    "properties": {
                        "head_x": { "type": "number" },
                        "head_y": { "type": "number" },
                        "head_z": { "type": "number" },
                        "spine_x": { "type": "number" },
                        "spine_z": { "type": "number" }
                    }
                }
            },
            "required": ["agent_id"]
        }),
    }
}

fn execute_set_expression(args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or("agent_id is required")?;
    let expression = args
        .get("expression")
        .and_then(|v| v.as_str())
        .ok_or("expression is required")?;
    let intensity = args
        .get("intensity")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0);

    let notif = JsonRpcNotification::new(
        "notifications/mgp.event",
        Some(json!({
            "channel": "avatar_set_expression",
            "data": {
                "agent_id": agent_id,
                "expression": expression,
                "intensity": intensity
            }
        })),
    );

    let result = json!({
        "content": [{
            "type": "text",
            "text": format!("Expression set: {} (intensity: {:.1})", expression, intensity)
        }]
    });

    Ok((result, vec![notif]))
}

fn execute_set_idle_behavior(args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or("agent_id is required")?;

    let mut data = json!({ "agent_id": agent_id });
    let data_obj = data.as_object_mut().unwrap();

    for key in &[
        "mode",
        "breathing_rate",
        "sway_amplitude",
        "blink_frequency",
        "pose",
    ] {
        if let Some(val) = args.get(*key) {
            data_obj.insert((*key).to_string(), val.clone());
        }
    }

    let notif = JsonRpcNotification::new(
        "notifications/mgp.event",
        Some(json!({
            "channel": "avatar_set_idle_behavior",
            "data": data
        })),
    );

    let mut changes = Vec::new();
    if let Some(mode) = args.get("mode").and_then(|v| v.as_str()) {
        changes.push(format!("mode={mode}"));
    }
    if let Some(br) = args
        .get("breathing_rate")
        .and_then(serde_json::Value::as_f64)
    {
        changes.push(format!("breathing={br:.1}"));
    }
    if let Some(sa) = args
        .get("sway_amplitude")
        .and_then(serde_json::Value::as_f64)
    {
        changes.push(format!("sway={sa:.1}"));
    }
    if let Some(bf) = args
        .get("blink_frequency")
        .and_then(serde_json::Value::as_f64)
    {
        changes.push(format!("blink={bf:.1}"));
    }
    if args.get("pose").is_some() {
        changes.push("pose=custom".into());
    }

    let summary = if changes.is_empty() {
        "No parameters changed".to_string()
    } else {
        format!("Idle behavior updated: {}", changes.join(", "))
    };

    let result = json!({
        "content": [{
            "type": "text",
            "text": summary
        }]
    });

    Ok((result, vec![notif]))
}

// ── VOICEVOX TTS Tools ──

fn speak_schema() -> McpTool {
    McpTool {
        name: "speak".into(),
        description: "Synthesize Japanese text to speech using VOICEVOX and play on the avatar with lip sync. Credit: VOICEVOX: ナースロボ＿タイプＴ".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Japanese text to speak"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Target agent ID for avatar lip sync"
                },
                "speaker": {
                    "type": "integer",
                    "description": "VOICEVOX speaker ID (default: current speaker)"
                },
                "speed": {
                    "type": "number",
                    "description": "Speech speed multiplier (default: 1.0)",
                    "minimum": 0.5,
                    "maximum": 2.0
                }
            },
            "required": ["text", "agent_id"]
        }),
    }
}

fn synthesize_schema() -> McpTool {
    McpTool {
        name: "synthesize".into(),
        description: "Synthesize Japanese text to a WAV file with viseme timeline. Returns file path and timing data. Credit: VOICEVOX: ナースロボ＿タイプＴ".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Japanese text to synthesize"
                },
                "speaker": {
                    "type": "integer",
                    "description": "VOICEVOX speaker ID (default: current speaker)"
                },
                "speed": {
                    "type": "number",
                    "description": "Speech speed multiplier (default: 1.0)"
                }
            },
            "required": ["text"]
        }),
    }
}

fn list_speakers_schema() -> McpTool {
    McpTool {
        name: "list_speakers".into(),
        description: "List all available VOICEVOX speakers and their style IDs.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn set_speaker_schema() -> McpTool {
    McpTool {
        name: "set_speaker".into(),
        description:
            "Change the active VOICEVOX speaker. Default: 47 (ナースロボ タイプT ノーマル)".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "speaker": {
                    "type": "integer",
                    "description": "Speaker style ID"
                }
            },
            "required": ["speaker"]
        }),
    }
}

fn execute_speak(args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or("text is required")?;
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or("agent_id is required")?;
    let speaker = args.get("speaker").and_then(serde_json::Value::as_i64);
    let speed = args.get("speed").and_then(serde_json::Value::as_f64);

    let client = voicevox();
    let (wav_bytes, viseme_timeline, total_duration_ms, audio_offset_ms) =
        client.synthesize(text, speaker, speed)?;

    // Encode WAV → OGG/Opus → base64 for inline delivery (no disk I/O, no HTTP round-trip)
    let audio_base64 = crate::voicevox::wav_to_opus_base64(&wav_bytes)?;

    let actual_speaker = speaker.unwrap_or_else(|| client.current_speaker.load(Ordering::Relaxed));

    let notif = JsonRpcNotification::new(
        "notifications/mgp.event",
        Some(json!({
            "channel": "avatar_speech_play",
            "data": {
                "agent_id": agent_id,
                "audio_data": audio_base64,
                "audio_mime": "audio/ogg; codecs=opus",
                "viseme_timeline": viseme_timeline,
                "total_duration_ms": total_duration_ms.round() as i64,
                "audio_offset_ms": audio_offset_ms.round() as i64,
                "speaker": actual_speaker,
                "text": text
            }
        })),
    );

    let result = json!({
        "content": [{
            "type": "text",
            "text": format!("Speech synthesized: {} ({:.0}ms, {} visemes)", text, total_duration_ms, viseme_timeline.len())
        }]
    });

    tracing::info!(
        "speak: text={}, speaker={}, duration={:.0}ms, offset={:.0}ms, opus_base64_len={}",
        text,
        actual_speaker,
        total_duration_ms,
        audio_offset_ms,
        audio_base64.len()
    );

    Ok((result, vec![notif]))
}

fn execute_synthesize(args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or("text is required")?;
    let speaker = args.get("speaker").and_then(serde_json::Value::as_i64);
    let speed = args.get("speed").and_then(serde_json::Value::as_f64);

    let client = voicevox();
    let (wav_bytes, viseme_timeline, total_duration_ms, audio_offset_ms) =
        client.synthesize(text, speaker, speed)?;
    let (abs_path, filename) = client.save_wav(&wav_bytes)?;

    let result = json!({
        "content": [{
            "type": "text",
            "text": json!({
                "status": "ok",
                "path": abs_path,
                "filename": filename,
                "duration_ms": total_duration_ms.round() as i64,
                "audio_offset_ms": audio_offset_ms.round() as i64,
                "viseme_count": viseme_timeline.len(),
                "viseme_timeline": viseme_timeline
            }).to_string()
        }]
    });

    Ok((result, vec![]))
}

fn execute_list_speakers(_args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let client = voicevox();
    let speakers_raw = client.list_speakers()?;

    let mut result_speakers = Vec::new();
    if let Some(speakers) = speakers_raw.as_array() {
        for speaker in speakers {
            let name = speaker.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let styles: Vec<Value> = speaker
                .get("styles")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|s| {
                            json!({
                                "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                "id": s.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            result_speakers.push(json!({ "name": name, "styles": styles }));
        }
    }

    let current = client.current_speaker.load(Ordering::Relaxed);

    let result = json!({
        "content": [{
            "type": "text",
            "text": json!({
                "speakers": result_speakers,
                "current_speaker": current,
                "count": result_speakers.len()
            }).to_string()
        }]
    });

    Ok((result, vec![]))
}

fn execute_set_speaker(args: &Value) -> Result<(Value, Vec<JsonRpcNotification>), String> {
    let speaker_id = args
        .get("speaker")
        .and_then(serde_json::Value::as_i64)
        .ok_or("speaker is required")?;

    let client = voicevox();
    client.current_speaker.store(speaker_id, Ordering::Relaxed);

    let result = json!({
        "content": [{
            "type": "text",
            "text": format!("Speaker changed to ID {}", speaker_id)
        }]
    });

    Ok((result, vec![]))
}
