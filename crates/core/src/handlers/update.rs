use axum::{
    extract::State,
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use sha2::{Sha256, Digest};
use std::sync::Arc;
use tracing::{info, error};

use crate::{AppState, AppResult, AppError};
use super::check_auth;

/// GET /api/system/version
/// Returns current Exiv version and build target (public, no auth).
pub async fn version_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_target": env!("TARGET"),
    }))
}

// --- GitHub API response types ---

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    html_url: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// GET /api/system/update/check
/// Checks GitHub Releases for a newer version (public, no auth required).
pub async fn check_handler(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let repo = &state.config.update_repo;
    let current_version = env!("CARGO_PKG_VERSION");
    let target = env!("TARGET");

    info!("🔍 Checking for updates from github.com/{}", repo);

    let client = reqwest::Client::new();
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);

    let response = client
        .get(&url)
        .header("User-Agent", format!("Exiv-System/{}", current_version))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to reach GitHub API: {}", e))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Json(serde_json::json!({
            "current_version": current_version,
            "update_available": false,
            "message": "No releases found in repository",
        })));
    }

    if !response.status().is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "GitHub API returned status {}",
            response.status()
        )));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse GitHub release: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v');

    // Bug #2: Use semver library for robust version comparison
    let (update_available, is_downgrade) = if let (Ok(cur), Ok(tgt)) = (
        semver::Version::parse(current_version),
        semver::Version::parse(latest_version)
    ) {
        let update_available = tgt != cur;
        let is_downgrade = tgt < cur;
        (update_available, is_downgrade)
    } else {
        // Bug E: Use numeric component comparison instead of lexicographic
        // Lexicographic comparison fails for "2.0.0" vs "10.0.0" (2 > 1 lexically)
        tracing::warn!(
            current = current_version,
            latest = latest_version,
            "Invalid semver format, attempting numeric component comparison"
        );

        // Try to parse as numeric components (e.g., "1.2.3" -> [1, 2, 3])
        let parse_components = |v: &str| -> Option<Vec<u32>> {
            v.split('.')
                .map(|part| part.trim().parse::<u32>().ok())
                .collect()
        };

        match (parse_components(current_version), parse_components(latest_version)) {
            (Some(cur), Some(tgt)) if !cur.is_empty() && !tgt.is_empty() => {
                use std::cmp::Ordering;
                let is_downgrade = matches!(cur.cmp(&tgt), Ordering::Greater);
                let update_available = cur != tgt;
                (update_available, is_downgrade)
            }
            _ => {
                // Ultimate fallback: string equality only, can't determine downgrade
                tracing::error!(
                    current = current_version,
                    latest = latest_version,
                    "Cannot parse version numbers, using string equality check only"
                );
                (latest_version != current_version, false)
            }
        }
    };

    // Find matching binary asset for this platform
    let expected_asset_name = format!("exiv_system-{}", target);
    let matching_assets: Vec<_> = release
        .assets
        .iter()
        .filter(|a| a.name.starts_with(&expected_asset_name) && !a.name.ends_with(".sha256"))
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "size": a.size,
                "download_url": a.browser_download_url,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "current_version": current_version,
        "latest_version": latest_version,
        "update_available": update_available,
        "is_downgrade": is_downgrade,
        "release_url": release.html_url,
        "release_name": release.name,
        "release_notes": release.body,
        "published_at": release.published_at,
        "build_target": target,
        "assets": matching_assets,
    })))
}

// --- Apply request ---

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub version: String,
}

/// POST /api/system/update/apply
/// Downloads, verifies (SHA256), and installs a specific version (admin auth required).
/// After successful installation, triggers a restart via the same mechanism as shutdown.
pub async fn apply_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ApplyRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let repo = &state.config.update_repo;
    let current_version = env!("CARGO_PKG_VERSION");
    let target = env!("TARGET");
    let requested_version = &payload.version;

    info!(
        "📦 Update apply requested: {} → {} (repo: {})",
        current_version, requested_version, repo
    );

    // Validate version format: only allow semver-like strings (alphanumeric, dots, hyphens, 'v' prefix)
    {
        let v = requested_version.strip_prefix('v').unwrap_or(requested_version);
        let is_valid = !v.is_empty()
            && v.len() <= 40
            && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && v.contains('.');
        if !is_valid {
            return Err(AppError::Internal(anyhow::anyhow!("Invalid version format")));
        }
    }

    // 1. Fetch the specific release from GitHub
    let tag = if requested_version.starts_with('v') {
        requested_version.clone()
    } else {
        format!("v{}", requested_version)
    };

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        repo, tag
    );

    let response = client
        .get(&url)
        .header("User-Agent", format!("Exiv-System/{}", current_version))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to reach GitHub API: {}", e))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(format!(
            "Release {} not found in {}",
            tag, repo
        )));
    }

    if !response.status().is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "GitHub API returned status {}",
            response.status()
        )));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse GitHub release: {}", e))?;

    // 2. Find matching binary asset for this platform
    let expected_asset_name = format!("exiv_system-{}", target);
    let binary_asset = release
        .assets
        .iter()
        .find(|a| a.name == expected_asset_name)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "No binary asset '{}' found in release {}",
                expected_asset_name, tag
            ))
        })?;

    // 3. Find SHA256 checksum asset
    let sha256_asset_name = format!("{}.sha256", expected_asset_name);
    let sha256_asset = release
        .assets
        .iter()
        .find(|a| a.name == sha256_asset_name)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "No checksum file '{}' found in release {} (required for verification)",
                sha256_asset_name, tag
            ))
        })?;

    // 4. Download the SHA256 checksum first
    info!("🔐 Downloading checksum: {}", sha256_asset.name);
    let expected_hash = client
        .get(&sha256_asset.browser_download_url)
        .header("User-Agent", format!("Exiv-System/{}", current_version))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to download checksum: {}", e))?
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read checksum: {}", e))?;

    // Parse checksum (format: "hash  filename" or just "hash")
    let expected_hash = expected_hash
        .trim()
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Checksum file is empty: {}", sha256_asset_name))?
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No hash found in checksum file: {}", sha256_asset_name))?
        .to_lowercase();

    // Bug F: Explicit validation for empty hash and format
    if expected_hash.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Checksum hash is empty in file: {}",
            sha256_asset_name
        )));
    }

    if expected_hash.len() != 64 || !expected_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Invalid SHA256 checksum format in {} (expected 64 hex characters, got {})",
            sha256_asset_name, expected_hash.len()
        )));
    }

    // 5. Download the binary
    info!(
        "📥 Downloading binary: {} ({} bytes)",
        binary_asset.name, binary_asset.size
    );
    let binary_data = client
        .get(&binary_asset.browser_download_url)
        .header("User-Agent", format!("Exiv-System/{}", current_version))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to download binary: {}", e))?
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read binary data: {}", e))?;

    // 6. Verify SHA256
    let mut hasher = Sha256::new();
    hasher.update(&binary_data);
    let computed_hash = format!("{:x}", hasher.finalize());

    if computed_hash != expected_hash {
        error!(
            "❌ SHA256 mismatch! Expected: {}, Got: {}",
            expected_hash, computed_hash
        );
        return Err(AppError::Internal(anyhow::anyhow!(
            "SHA256 verification failed. Binary may be corrupted or tampered with."
        )));
    }

    info!("✅ SHA256 verified: {}", computed_hash);

    // 7. Determine install paths
    let exe_path = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Cannot determine current executable path: {}", e))?;

    let new_path = exe_path.with_extension("new");
    let old_path = exe_path.with_extension("old");

    // 8. Write new binary to disk
    tokio::fs::write(&new_path, &binary_data)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write new binary: {}", e))?;

    // Set executable permission (platform-abstracted)
    crate::platform::set_executable_permission(&new_path)
        .map_err(|e| anyhow::anyhow!("Failed to set executable permission: {}", e))?;

    // 9. Platform-abstracted binary swap: current → .old, .new → current
    crate::platform::swap_running_binary(&new_path, &exe_path, &old_path)
        .map_err(|e| anyhow::anyhow!("Binary swap failed: {}", e))?;

    info!(
        "📦 Binary updated: {} → {} (sha256: {})",
        current_version, requested_version, computed_hash
    );

    // 10. Write audit log
    let audit_entry = crate::db::AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event_type: "SYSTEM_UPDATE".to_string(),
        actor_id: Some("admin".to_string()),
        target_id: Some("exiv_system".to_string()),
        permission: None,
        result: "SUCCESS".to_string(),
        reason: format!(
            "System updated from {} to {}",
            current_version, requested_version
        ),
        metadata: Some(serde_json::json!({
            "previous_version": current_version,
            "new_version": requested_version,
            "sha256": computed_hash,
            "asset": binary_asset.name,
        })),
        trace_id: None,
    };

    let pool = state.pool.clone();
    // Write audit log synchronously before restart
    if let Err(e) = crate::db::write_audit_log(&pool, audit_entry).await {
        error!("Failed to write update audit log: {}", e);
    }

    // 11. Broadcast system notification
    let envelope = crate::EnvelopedEvent::system(
        exiv_shared::ExivEventData::SystemNotification(format!(
            "System update applied: {} → {}. Restarting...",
            current_version, requested_version
        )),
    );
    let _ = state.event_tx.send(envelope).await;

    // 12. Trigger restart (same pattern as shutdown_handler)
    let shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Atomic write to prevent symlink attacks
        let maint = crate::config::exe_dir().join(".maintenance");
        let maint_tmp = crate::config::exe_dir().join(".maintenance.tmp");
        match std::fs::write(&maint_tmp, "updating")
            .and_then(|()| std::fs::rename(&maint_tmp, &maint))
        {
            Ok(()) => info!("🚧 Maintenance mode engaged for update."),
            Err(e) => error!("❌ Failed to create .maintenance file: {}", e),
        }

        info!("🔄 Restarting with new version...");
        shutdown.notify_one();
    });

    Ok(Json(serde_json::json!({
        "status": "updating",
        "previous_version": current_version,
        "new_version": requested_version,
        "sha256": computed_hash,
        "message": "Update applied successfully. System is restarting...",
    })))
}
