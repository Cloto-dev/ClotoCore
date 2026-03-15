//! VOICEVOX Engine client — HTTP API for Japanese TTS with mora-level viseme timing.
//!
//! Uses `reqwest::blocking::Client` to communicate with the VOICEVOX Engine
//! running at a configurable URL (default: http://localhost:50021).
//!
//! Credit: VOICEVOX: ナースロボ＿タイプＴ

use base64::prelude::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

/// VOICEVOX configuration loaded from environment variables.
pub struct VoicevoxConfig {
    pub url: String,
    pub default_speaker: i64,
    pub speed: f64,
    pub output_dir: PathBuf,
    pub engine_path: Option<String>,
}

impl VoicevoxConfig {
    pub fn from_env() -> Self {
        let url = std::env::var("VOICEVOX_URL").unwrap_or_else(|_| "http://localhost:50021".into());
        let default_speaker = std::env::var("VOICEVOX_SPEAKER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(47); // ナースロボ タイプT ノーマル
        let speed = std::env::var("VOICEVOX_SPEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);
        let output_dir = resolve_output_dir();
        let engine_path = std::env::var("VOICEVOX_ENGINE_PATH").ok();

        Self {
            url,
            default_speaker,
            speed,
            output_dir,
            engine_path,
        }
    }
}

/// Resolve output directory matching the kernel's exe_dir()/data/speech/ strategy.
fn resolve_output_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data")
        .join("speech")
}

/// VOICEVOX Engine client with shared state.
pub struct VoicevoxClient {
    config: VoicevoxConfig,
    http: reqwest::blocking::Client,
    pub current_speaker: AtomicI64,
}

impl VoicevoxClient {
    pub fn new(config: VoicevoxConfig) -> Self {
        let speaker = config.default_speaker;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            http,
            current_speaker: AtomicI64::new(speaker),
        }
    }

    /// Synthesize text to WAV bytes + viseme timeline.
    /// Returns (wav_bytes, viseme_entries_json, total_duration_ms, audio_offset_ms).
    /// `audio_offset_ms` is the pre-phoneme silence in the WAV that visemes must skip.
    pub fn synthesize(
        &self,
        text: &str,
        speaker: Option<i64>,
        speed: Option<f64>,
    ) -> Result<(Vec<u8>, Vec<Value>, f64, f64), String> {
        let speaker = speaker.unwrap_or_else(|| self.current_speaker.load(Ordering::Relaxed));
        let speed = speed.unwrap_or(self.config.speed);

        // Step 1: Audio query (get phoneme timing)
        let query_url = format!("{}/audio_query", self.config.url);
        let resp = self
            .http
            .post(&query_url)
            .query(&[("text", text), ("speaker", &speaker.to_string())])
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(|e| {
                if e.is_connect() {
                    format!(
                        "VOICEVOX engine not reachable at {}. Please start VOICEVOX first.",
                        self.config.url
                    )
                } else {
                    format!("VOICEVOX audio_query failed: {e}")
                }
            })?;

        if !resp.status().is_success() {
            return Err(format!("VOICEVOX audio_query error: {}", resp.status()));
        }

        let mut query: Value = resp.json().map_err(|e| format!("JSON parse error: {e}"))?;

        // Apply speed scale
        query["speedScale"] = json!(speed);

        // Read pre-phoneme silence duration (default 0.1s = 100ms).
        // VOICEVOX adds this silence before the first phoneme in the WAV,
        // but it is NOT reflected in accentPhrases mora timing.
        // prePhonemeLength is a fixed pad — NOT affected by speedScale.
        let pre_phoneme_length = query
            .get("prePhonemeLength")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.1);
        let audio_offset_ms = pre_phoneme_length * 1000.0;

        // Extract viseme timeline from accent phrases
        // VOICEVOX uses snake_case for accent_phrases (not camelCase)
        let accent_phrases = query
            .get("accent_phrases")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut viseme_timeline = mora_to_viseme_timeline(&accent_phrases);

        // Adjust viseme timing for speed scale
        if (speed - 1.0).abs() > 0.01 {
            for entry in &mut viseme_timeline {
                if let (Some(start), Some(dur)) = (
                    entry.get("start_ms").and_then(serde_json::Value::as_f64),
                    entry.get("duration_ms").and_then(serde_json::Value::as_f64),
                ) {
                    entry["start_ms"] = json!((start / speed).round() as i64);
                    entry["duration_ms"] = json!((dur / speed).round() as i64);
                }
            }
        }

        let total_duration_ms = viseme_timeline
            .last()
            .and_then(|e| {
                let start = e.get("start_ms")?.as_f64()?;
                let dur = e.get("duration_ms")?.as_f64()?;
                Some(start + dur)
            })
            .unwrap_or(0.0);

        // Step 2: Synthesis (generate WAV)
        let synth_url = format!("{}/synthesis", self.config.url);
        let resp = self
            .http
            .post(&synth_url)
            .query(&[("speaker", &speaker.to_string())])
            .json(&query)
            .timeout(Duration::from_secs(30))
            .send()
            .map_err(|e| format!("VOICEVOX synthesis failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("VOICEVOX synthesis error: {}", resp.status()));
        }

        let wav_bytes = resp.bytes().map_err(|e| format!("WAV read error: {e}"))?;

        Ok((
            wav_bytes.to_vec(),
            viseme_timeline,
            total_duration_ms,
            audio_offset_ms,
        ))
    }

    /// List all available speakers from VOICEVOX Engine.
    pub fn list_speakers(&self) -> Result<Value, String> {
        let url = format!("{}/speakers", self.config.url);
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(|e| {
                if e.is_connect() {
                    format!(
                        "VOICEVOX engine not reachable at {}. Please start VOICEVOX first.",
                        self.config.url
                    )
                } else {
                    format!("VOICEVOX speakers request failed: {e}")
                }
            })?;

        if !resp.status().is_success() {
            return Err(format!("VOICEVOX speakers error: {}", resp.status()));
        }

        let speakers: Value = resp.json().map_err(|e| format!("JSON parse error: {e}"))?;
        Ok(speakers)
    }

    /// Save WAV bytes to the output directory. Returns (absolute_path, filename).
    pub fn save_wav(&self, wav_bytes: &[u8]) -> Result<(String, String), String> {
        std::fs::create_dir_all(&self.config.output_dir)
            .map_err(|e| format!("Failed to create speech directory: {e}"))?;

        let filename = generate_filename();
        let filepath = self.config.output_dir.join(&filename);

        std::fs::write(&filepath, wav_bytes)
            .map_err(|e| format!("Failed to write WAV file: {e}"))?;

        let abs_path = filepath
            .canonicalize()
            .unwrap_or(filepath)
            .to_string_lossy()
            .to_string();

        Ok((abs_path, filename))
    }
}

// ── Mora → Viseme Mapping ──

fn vowel_to_viseme(vowel: &str) -> &'static str {
    match vowel {
        "a" => "aa",
        "i" => "ih",
        "u" => "ou",
        "e" => "ee",
        "o" => "oh",
        _ => "neutral",
    }
}

/// Convert VOICEVOX AccentPhrase[] to viseme timeline entries.
fn mora_to_viseme_timeline(accent_phrases: &[Value]) -> Vec<Value> {
    let mut entries = Vec::new();
    let mut cursor_ms: f64 = 0.0;

    for phrase in accent_phrases {
        // Process pause_mora (between phrases)
        if let Some(pause) = phrase.get("pause_mora") {
            let vowel_len = pause
                .get("vowel_length")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let duration_ms = vowel_len * 1000.0;
            if duration_ms > 0.0 {
                entries.push(json!({
                    "viseme": "neutral",
                    "start_ms": cursor_ms.round() as i64,
                    "duration_ms": duration_ms.round() as i64,
                }));
                cursor_ms += duration_ms;
            }
        }

        if let Some(moras) = phrase.get("moras").and_then(|v| v.as_array()) {
            for mora in moras {
                let consonant_len = mora
                    .get("consonant_length")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                let vowel_len = mora
                    .get("vowel_length")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                let vowel = mora.get("vowel").and_then(|v| v.as_str()).unwrap_or("");

                // Consonant portion → neutral
                let consonant_ms = consonant_len * 1000.0;
                if consonant_ms > 5.0 {
                    entries.push(json!({
                        "viseme": "neutral",
                        "start_ms": cursor_ms.round() as i64,
                        "duration_ms": consonant_ms.round() as i64,
                    }));
                    cursor_ms += consonant_ms;
                }

                // Vowel portion → mapped viseme
                let vowel_ms = vowel_len * 1000.0;
                if vowel_ms > 0.0 {
                    let viseme = vowel_to_viseme(vowel);
                    entries.push(json!({
                        "viseme": viseme,
                        "start_ms": cursor_ms.round() as i64,
                        "duration_ms": vowel_ms.round() as i64,
                    }));
                    cursor_ms += vowel_ms;
                }
            }
        }
    }

    entries
}

/// Generate a safe filename for WAV output.
fn generate_filename() -> String {
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let rand_suffix = &uuid::Uuid::new_v4().to_string()[..6];
    format!("vvox_{ts}_{rand_suffix}.wav")
}

// ── WAV → OGG/Opus Encoding ──

/// Extract raw PCM i16 samples from VOICEVOX WAV bytes (24kHz 16-bit mono).
fn extract_pcm_samples(wav_bytes: &[u8]) -> Result<Vec<i16>, String> {
    if wav_bytes.len() < 44 {
        return Err("WAV data too short".into());
    }
    if &wav_bytes[0..4] != b"RIFF" || &wav_bytes[8..12] != b"WAVE" {
        return Err("Invalid WAV header".into());
    }

    // Find 'data' chunk
    let mut pos = 12;
    while pos + 8 <= wav_bytes.len() {
        let chunk_id = &wav_bytes[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(
            wav_bytes[pos + 4..pos + 8]
                .try_into()
                .map_err(|_| "WAV chunk size parse error")?,
        ) as usize;

        if chunk_id == b"data" {
            let data_end = (pos + 8 + chunk_size).min(wav_bytes.len());
            let data = &wav_bytes[pos + 8..data_end];
            return Ok(data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect());
        }

        pos += 8 + chunk_size;
        if !chunk_size.is_multiple_of(2) {
            pos += 1; // word-align
        }
    }

    Err("No data chunk found in WAV".into())
}

/// Encode WAV bytes to OGG/Opus and return as base64 string.
/// VOICEVOX outputs 24kHz 16-bit mono PCM — matches `ogg_opus::encode::<24000, 1>`.
pub fn wav_to_opus_base64(wav_bytes: &[u8]) -> Result<String, String> {
    let samples = extract_pcm_samples(wav_bytes)?;
    let ogg_bytes =
        ogg_opus::encode::<24000, 1>(&samples).map_err(|e| format!("Opus encode error: {e}"))?;

    tracing::debug!(
        "Opus encoded: WAV {}KB → OGG/Opus {}KB ({:.0}x compression)",
        wav_bytes.len() / 1024,
        ogg_bytes.len() / 1024,
        wav_bytes.len() as f64 / ogg_bytes.len().max(1) as f64
    );

    Ok(BASE64_STANDARD.encode(&ogg_bytes))
}
