// Shared handler utilities: validation constants, MIME helpers

// ── Validation Constants ─────────────────────────────────────

pub const AGENT_NAME_MIN: usize = 1;
pub const AGENT_NAME_MAX: usize = 200;
pub const AGENT_DESC_MIN: usize = 1;
pub const AGENT_DESC_MAX: usize = 1000;
pub const AGENT_METADATA_MAX_PAIRS: usize = 50;
pub const AGENT_METADATA_KEY_MAX: usize = 200;
pub const AGENT_METADATA_VALUE_MAX: usize = 5000;
pub const AVATAR_MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB
pub const CONTENT_BLOCK_MAX_ITEMS: usize = 20;

// ── MIME Helpers ─────────────────────────────────────────────

/// Convert a MIME type to a file extension.
/// Returns `None` for unsupported image types.
pub fn mime_to_ext(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}

/// Convert a MIME type to a file extension with a fallback default.
pub fn mime_to_ext_or(mime: &str, default: &'static str) -> &'static str {
    mime_to_ext(mime).unwrap_or(default)
}

/// Convert a file extension to its MIME type.
pub fn ext_to_mime(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
