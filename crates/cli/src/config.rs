use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_url() -> String {
    "http://127.0.0.1:8081".to_string()
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            url: default_url(),
            api_key: None,
        }
    }
}

impl CliConfig {
    /// Config file path: ~/.config/exiv/cli.toml
    pub fn path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Cannot determine config directory")?
            .join("exiv");
        Ok(config_dir.join("cli.toml"))
    }

    /// Load config from file, falling back to defaults.
    /// Environment variables override file values.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", path.display()))?
        } else {
            Self::default()
        };

        // Environment variable overrides
        if let Ok(url) = std::env::var("EXIV_URL") {
            config.url = url;
        }
        if let Ok(key) = std::env::var("EXIV_API_KEY") {
            config.api_key = Some(key);
        }

        Ok(config)
    }

    /// Save current config to file.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Load config from file only (without env var overrides).
    /// Used by `set()` to avoid writing env-var values back to disk.
    fn load_file_only() -> Result<Self> {
        let path = Self::path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", path.display()))
        } else {
            Ok(Self::default())
        }
    }

    /// Set a single config key and save.
    /// bug-027: Loads from file only (not env vars) to prevent writing
    /// environment credentials to disk.
    pub fn set(key: &str, value: &str) -> Result<()> {
        let mut config = Self::load_file_only()?;
        match key {
            "url" => config.url = value.to_string(),
            "api_key" => config.api_key = Some(value.to_string()),
            _ => anyhow::bail!("Unknown config key: {key}. Valid keys: url, api_key"),
        }
        config.save()
    }
}
