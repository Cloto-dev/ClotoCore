//! Viseme timeline generation from text.
//!
//! Converts Japanese/English text into a sequence of viseme entries
//! suitable for VRM lip-sync animation.
//!
//! Japanese: lindera (ipadic) morphological analysis → kana → vowel mapping
//! English: rule-based vowel detection (a,e,i,o,u)

use serde::{Deserialize, Serialize};

// ── Public Types ──

/// A single viseme in the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisemeEntry {
    /// Viseme name: "aa", "ih", "ou", "ee", "oh", "neutral"
    pub viseme: String,
    /// Start time in milliseconds from the beginning of the timeline.
    pub start_ms: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Complete viseme timeline for a text utterance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisemeTimeline {
    pub entries: Vec<VisemeEntry>,
    pub total_duration_ms: u64,
}

// ── Constants ──

/// Base mora duration in milliseconds.
const MORA_MS: u64 = 120;
/// Duration for ん (moraic nasal).
const N_MS: u64 = 80;
/// Duration for っ (geminate/pause).
const SOKUON_MS: u64 = 100;
/// Duration for comma/mid-sentence pause.
const COMMA_PAUSE_MS: u64 = 200;
/// Duration for period/end-of-sentence pause.
const PERIOD_PAUSE_MS: u64 = 400;
/// Duration per English character (approximation).
const ENGLISH_CHAR_MS: u64 = 80;
/// Smoothing: minimum gap between entries to avoid overlap.
const MIN_GAP_MS: u64 = 10;

// ── Language Detection ──

#[derive(Debug, Clone, Copy, PartialEq)]
enum Language {
    Japanese,
    English,
}

/// Segment of text with detected language.
#[derive(Debug)]
struct LangSegment {
    text: String,
    lang: Language,
}

/// Check if a character is in CJK/Kana/Katakana ranges.
fn is_japanese_char(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{309F}' |  // Hiragana
        '\u{30A0}'..='\u{30FF}' |  // Katakana
        '\u{4E00}'..='\u{9FFF}' |  // CJK Unified
        '\u{3400}'..='\u{4DBF}' |  // CJK Extension A
        '\u{FF00}'..='\u{FFEF}' |  // Fullwidth forms
        '\u{3000}'..='\u{303F}'    // CJK Symbols
    )
}

fn is_punctuation(c: char) -> bool {
    matches!(c, '。' | '.' | '！' | '!' | '？' | '?' | '、' | ',' | '；' | ';'
        | '：' | ':' | '…' | '─' | '—' | '\n')
}

/// Split text into segments by language.
fn split_language_segments(text: &str) -> Vec<LangSegment> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_lang: Option<Language> = None;

    for c in text.chars() {
        if is_punctuation(c) {
            // Flush current segment, then add punctuation as its own segment
            if !current.is_empty() {
                segments.push(LangSegment {
                    text: std::mem::take(&mut current),
                    lang: current_lang.unwrap_or(Language::Japanese),
                });
                current_lang = None;
            }
            segments.push(LangSegment {
                text: c.to_string(),
                lang: Language::Japanese, // punctuation handled uniformly
            });
            continue;
        }

        let lang = if is_japanese_char(c) {
            Language::Japanese
        } else if c.is_ascii_alphabetic() {
            Language::English
        } else {
            // Whitespace or other: continue with current language
            current.push(c);
            continue;
        };

        if current_lang.is_some() && current_lang != Some(lang) {
            // Language boundary: flush
            segments.push(LangSegment {
                text: std::mem::take(&mut current),
                lang: current_lang.unwrap(),
            });
        }
        current_lang = Some(lang);
        current.push(c);
    }

    if !current.is_empty() {
        segments.push(LangSegment {
            text: current,
            lang: current_lang.unwrap_or(Language::Japanese),
        });
    }

    segments
}

// ── Japanese Viseme Generation ──

/// Map a katakana character to its vowel viseme.
fn katakana_to_viseme(c: char) -> &'static str {
    match c {
        // ア行
        'ア' | 'カ' | 'サ' | 'タ' | 'ナ' | 'ハ' | 'マ' | 'ヤ' | 'ラ' | 'ワ' | 'ガ' | 'ザ'
        | 'ダ' | 'バ' | 'パ' | 'ャ' | 'ヴ' => "aa",
        // イ行
        'イ' | 'キ' | 'シ' | 'チ' | 'ニ' | 'ヒ' | 'ミ' | 'リ' | 'ギ' | 'ジ' | 'ヂ' | 'ビ'
        | 'ピ' => "ih",
        // ウ行
        'ウ' | 'ク' | 'ス' | 'ツ' | 'ヌ' | 'フ' | 'ム' | 'ユ' | 'ル' | 'グ' | 'ズ' | 'ヅ'
        | 'ブ' | 'プ' | 'ュ' => "ou",
        // エ行
        'エ' | 'ケ' | 'セ' | 'テ' | 'ネ' | 'ヘ' | 'メ' | 'レ' | 'ゲ' | 'ゼ' | 'デ' | 'ベ'
        | 'ペ' => "ee",
        // オ行
        'オ' | 'コ' | 'ソ' | 'ト' | 'ノ' | 'ホ' | 'モ' | 'ヨ' | 'ロ' | 'ヲ' | 'ゴ' | 'ゾ'
        | 'ド' | 'ボ' | 'ポ' | 'ョ' => "oh",
        // ン (moraic nasal)
        'ン' => "neutral",
        // ッ (geminate consonant)
        'ッ' => "neutral",
        // Small kana (ァ,ィ,ゥ,ェ,ォ)
        'ァ' => "aa",
        'ィ' => "ih",
        'ゥ' => "ou",
        'ェ' => "ee",
        'ォ' => "oh",
        // Long vowel mark
        'ー' => "neutral", // extend previous viseme (handled in timeline)
        _ => "neutral",
    }
}

/// Duration for a katakana character.
fn katakana_duration(c: char) -> u64 {
    match c {
        'ン' => N_MS,
        'ッ' => SOKUON_MS,
        'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ャ' | 'ュ' | 'ョ' => MORA_MS / 2, // small kana: half mora
        'ー' => MORA_MS / 2, // extension
        _ => MORA_MS,
    }
}

/// Generate visemes from Japanese text using lindera tokenization.
fn japanese_to_visemes(text: &str) -> Vec<VisemeEntry> {
    use lindera::dictionary::load_dictionary;
    use lindera::mode::Mode;
    use lindera::segmenter::Segmenter;
    use lindera::tokenizer::Tokenizer;

    let dictionary = match load_dictionary("embedded://ipadic") {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Failed to load ipadic dictionary: {}", e);
            return english_to_visemes(text); // fallback
        }
    };
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    let tokenizer = Tokenizer::new(segmenter);

    let mut tokens = match tokenizer.tokenize(text) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Tokenization failed: {}", e);
            return english_to_visemes(text);
        }
    };

    let mut entries = Vec::new();
    let mut cursor_ms: u64 = 0;

    for token in &mut tokens {
        // ipadic details: [品詞, 品詞細分類1, 品詞細分類2, 品詞細分類3, 活用型, 活用形, 原形, 読み, 発音]
        // Reading (読み) is at index 7 in katakana
        let details = token.details();
        let reading = if details.len() > 7 {
            details[7].to_string()
        } else {
            // No reading available — use surface as-is
            token.surface.to_string()
        };

        // Convert reading (katakana) to visemes
        for c in reading.chars() {
            let viseme = katakana_to_viseme(c);
            let duration = katakana_duration(c);

            // Handle long vowel mark: extend previous entry instead of adding neutral
            if c == 'ー' && !entries.is_empty() {
                let last: &mut VisemeEntry = entries.last_mut().unwrap();
                last.duration_ms += duration;
                cursor_ms += duration;
                continue;
            }

            entries.push(VisemeEntry {
                viseme: viseme.to_string(),
                start_ms: cursor_ms,
                duration_ms: duration,
            });
            cursor_ms += duration;
        }
    }

    entries
}

// ── English Viseme Generation ──

/// Map English vowel character to viseme.
fn english_char_to_viseme(c: char) -> &'static str {
    match c.to_ascii_lowercase() {
        'a' => "aa",
        'e' => "ee",
        'i' => "ih",
        'o' => "oh",
        'u' => "ou",
        _ => "neutral", // consonants
    }
}

/// Generate visemes from English text using rule-based vowel detection.
fn english_to_visemes(text: &str) -> Vec<VisemeEntry> {
    let mut entries: Vec<VisemeEntry> = Vec::new();
    let mut cursor_ms: u64 = 0;

    for c in text.chars() {
        if !c.is_ascii_alphabetic() {
            continue;
        }

        let viseme = english_char_to_viseme(c);
        // Skip consecutive neutrals (consonant clusters)
        if viseme == "neutral" {
            if let Some(last) = entries.last() {
                if last.viseme == "neutral" {
                    // Extend previous neutral instead of adding another
                    let last_mut: &mut VisemeEntry = entries.last_mut().unwrap();
                    last_mut.duration_ms += ENGLISH_CHAR_MS / 2;
                    cursor_ms += ENGLISH_CHAR_MS / 2;
                    continue;
                }
            }
        }

        entries.push(VisemeEntry {
            viseme: viseme.to_string(),
            start_ms: cursor_ms,
            duration_ms: ENGLISH_CHAR_MS,
        });
        cursor_ms += ENGLISH_CHAR_MS;
    }

    entries
}

// ── Punctuation Handling ──

fn punctuation_duration(c: char) -> u64 {
    match c {
        '。' | '.' | '！' | '!' | '？' | '?' | '\n' => PERIOD_PAUSE_MS,
        '、' | ',' | '；' | ';' | '：' | ':' => COMMA_PAUSE_MS,
        '…' => PERIOD_PAUSE_MS,
        _ => COMMA_PAUSE_MS,
    }
}

// ── Public API ──

/// Generate a viseme timeline from text input.
///
/// Handles mixed Japanese/English text by splitting into language segments,
/// generating visemes for each, and merging with punctuation pauses.
pub fn generate_timeline(text: &str) -> VisemeTimeline {
    let segments = split_language_segments(text);
    let mut all_entries = Vec::new();
    let mut cursor_ms: u64 = 0;

    for segment in &segments {
        // Check if this is a punctuation segment
        if segment.text.len() <= 4 {
            // Single char (up to 4 bytes for unicode)
            if let Some(c) = segment.text.chars().next() {
                if is_punctuation(c) {
                    let pause = punctuation_duration(c);
                    all_entries.push(VisemeEntry {
                        viseme: "neutral".to_string(),
                        start_ms: cursor_ms,
                        duration_ms: pause,
                    });
                    cursor_ms += pause;
                    continue;
                }
            }
        }

        let segment_entries = match segment.lang {
            Language::Japanese => japanese_to_visemes(&segment.text),
            Language::English => english_to_visemes(&segment.text),
        };

        // Offset entries by current cursor position
        for mut entry in segment_entries {
            entry.start_ms += cursor_ms;
            all_entries.push(entry);
        }

        // Update cursor to end of last entry
        if let Some(last) = all_entries.last() {
            cursor_ms = last.start_ms + last.duration_ms + MIN_GAP_MS;
        }
    }

    let total = all_entries
        .last()
        .map(|e| e.start_ms + e.duration_ms)
        .unwrap_or(0);

    VisemeTimeline {
        entries: all_entries,
        total_duration_ms: total,
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_language_segments() {
        let segments = split_language_segments("こんにちはhello世界");
        assert!(segments.len() >= 3);
        assert_eq!(segments[0].lang, Language::Japanese);
        assert_eq!(segments[1].lang, Language::English);
        assert_eq!(segments[2].lang, Language::Japanese);
    }

    #[test]
    fn test_katakana_vowel_mapping() {
        assert_eq!(katakana_to_viseme('ア'), "aa");
        assert_eq!(katakana_to_viseme('イ'), "ih");
        assert_eq!(katakana_to_viseme('ウ'), "ou");
        assert_eq!(katakana_to_viseme('エ'), "ee");
        assert_eq!(katakana_to_viseme('オ'), "oh");
        assert_eq!(katakana_to_viseme('カ'), "aa");
        assert_eq!(katakana_to_viseme('キ'), "ih");
        assert_eq!(katakana_to_viseme('ン'), "neutral");
    }

    #[test]
    fn test_english_visemes() {
        let entries = english_to_visemes("hello");
        assert!(!entries.is_empty());
        // h=neutral, e=ee, l=neutral, l=extend, o=oh
        let visemes: Vec<&str> = entries.iter().map(|e| e.viseme.as_str()).collect();
        assert!(visemes.contains(&"ee"));
        assert!(visemes.contains(&"oh"));
    }

    #[test]
    fn test_generate_timeline_japanese() {
        let timeline = generate_timeline("こんにちは");
        assert!(!timeline.entries.is_empty());
        assert!(timeline.total_duration_ms > 0);
    }

    #[test]
    fn test_generate_timeline_english() {
        let timeline = generate_timeline("hello world");
        assert!(!timeline.entries.is_empty());
        assert!(timeline.total_duration_ms > 0);
    }

    #[test]
    fn test_generate_timeline_mixed() {
        let timeline = generate_timeline("Hello、世界！");
        assert!(!timeline.entries.is_empty());
        // Should have pauses for 、 and ！
        let neutrals: Vec<_> = timeline
            .entries
            .iter()
            .filter(|e| e.viseme == "neutral" && e.duration_ms >= COMMA_PAUSE_MS)
            .collect();
        assert!(!neutrals.is_empty(), "Should have punctuation pauses");
    }

    #[test]
    fn test_empty_text() {
        let timeline = generate_timeline("");
        assert!(timeline.entries.is_empty());
        assert_eq!(timeline.total_duration_ms, 0);
    }

    #[test]
    fn test_punctuation_only() {
        let timeline = generate_timeline("。");
        assert_eq!(timeline.entries.len(), 1);
        assert_eq!(timeline.entries[0].viseme, "neutral");
        assert_eq!(timeline.entries[0].duration_ms, PERIOD_PAUSE_MS);
    }
}
