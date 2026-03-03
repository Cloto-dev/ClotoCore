// Shared handler utilities: validation constants, MIME helpers, auth helpers

use crate::{AppError, AppResult, AppState};

// ── Password Verification ───────────────────────────────────

/// Verify an agent's password if one is set.
/// Returns Ok(()) if no password is set or if the provided password matches.
pub async fn verify_agent_password(
    state: &AppState,
    agent_id: &str,
    password: Option<&str>,
    operation: &str,
) -> AppResult<()> {
    let password_hash = state.agent_manager.get_password_hash(agent_id).await?;
    if let Some(ref hash) = password_hash {
        match password {
            Some(pw) => {
                if !crate::managers::AgentManager::verify_password(pw, hash)? {
                    return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                        cloto_shared::Permission::AdminAccess,
                    )));
                }
            }
            None => {
                return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
                    format!("Password required to {operation}"),
                )));
            }
        }
    }
    Ok(())
}

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
#[must_use]
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
#[must_use]
pub fn mime_to_ext_or(mime: &str, default: &'static str) -> &'static str {
    mime_to_ext(mime).unwrap_or(default)
}

/// Convert a file extension to its MIME type.
#[must_use]
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
