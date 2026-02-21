use exiv_shared::PluginConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::state::PythonBridgeState;
use crate::PythonBridgePlugin;

/// Resolve a script path: try exe-relative first (deployed), fall back to CWD (development).
pub(crate) fn resolve_script_path(relative: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(relative);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from(relative)
}

impl PythonBridgePlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let script_path = config
            .config_values
            .get("script_path")
            .cloned()
            .unwrap_or_else(|| "bridge_main.py".to_string());

        // Security: prevent path traversal attacks using canonical path validation
        // This prevents attacks like "scripts/../../../etc/passwd" or Windows "scripts\..\..\"
        let base_dir = PathBuf::from("scripts/");
        let candidate_path = base_dir.join(&script_path);

        // Canonicalize both paths to resolve all symlinks and ".." components
        // Bug #10: Granular error messages for different path validation failures
        let base_canonical = base_dir.canonicalize().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                anyhow::anyhow!("Scripts directory not found: {}", base_dir.display())
            }
            std::io::ErrorKind::PermissionDenied => anyhow::anyhow!(
                "Permission denied accessing scripts directory: {}",
                base_dir.display()
            ),
            _ => anyhow::anyhow!(
                "Failed to resolve scripts directory: {} ({})",
                base_dir.display(),
                e
            ),
        })?;
        let candidate_canonical = candidate_path.canonicalize().map_err(|e| {
            tracing::warn!(
                "Script path canonicalization failed for '{}': {}",
                script_path,
                e
            );
            match e.kind() {
                std::io::ErrorKind::NotFound => {
                    anyhow::anyhow!("Script file not found: {}", script_path)
                }
                std::io::ErrorKind::PermissionDenied => {
                    anyhow::anyhow!("Permission denied accessing script: {}", script_path)
                }
                _ => anyhow::anyhow!("Invalid script path '{}': {}", script_path, e),
            }
        })?;

        // Ensure the resolved path is still within the base directory
        if !candidate_canonical.starts_with(&base_canonical) {
            return Err(anyhow::anyhow!(
                "Security violation: Script path '{}' escapes allowed directory",
                script_path
            ));
        }

        Ok(Self {
            instance_id: config.id,
            script_path,
            state: Arc::new(RwLock::new(PythonBridgeState::new())),
            tool_name: Arc::new(std::sync::OnceLock::new()),
            tool_description: Arc::new(std::sync::OnceLock::new()),
            tool_schema: Arc::new(std::sync::OnceLock::new()),
        })
    }
}
