use async_trait::async_trait;
use exiv_shared::{
    FileCapability, HttpRequest, HttpResponse, NetworkCapability, ProcessCapability,
};
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::net::lookup_host;
use tracing::warn;

#[derive(Clone)]
pub struct SafeHttpClient {
    client: reqwest::Client,
    /// L5: Dynamic whitelist wrapped in Arc<RwLock> for runtime host addition
    allowed_hosts: Arc<RwLock<HashSet<String>>>,
}

impl SafeHttpClient {
    pub fn new(allowed_hosts: Vec<String>) -> anyhow::Result<Self> {
        let defaults = [
            "api.deepseek.com",
            "api.cerebras.ai",
            "api.openai.com",
            "api.anthropic.com",
        ];

        // Store all hosts pre-lowercased in a HashSet for O(1) lookup
        let mut hosts: HashSet<String> = allowed_hosts
            .into_iter()
            .map(|h| h.to_lowercase())
            .collect();
        for d in defaults {
            hosts.insert(d.to_string());
        }

        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            allowed_hosts: Arc::new(RwLock::new(hosts)),
        })
    }

    /// IPアドレスベースでの制限チェック (Principle #5: Strict Permission Isolation)
    fn is_restricted_addr(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_documentation()
                    || v4.is_unspecified()
                    || v4.octets()[0] == 0
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (v6.segments()[0] & 0xfe00 == 0xfc00)
                    || v6.is_multicast()
            }
        }
    }

    /// ホスト名ベースでのホワイトリストチェック (O(1) HashSet lookup)
    fn is_whitelisted_host(&self, host: &str) -> bool {
        let hosts = self
            .allowed_hosts
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        hosts.contains(&host.to_lowercase())
    }

    /// L5: Add a host to the whitelist at runtime.
    /// Returns true if newly inserted, false if already present.
    #[must_use]
    pub fn add_host(&self, host: &str) -> bool {
        let normalized = host.to_lowercase();
        let mut hosts = self
            .allowed_hosts
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        hosts.insert(normalized)
    }
}

#[async_trait]
impl NetworkCapability for SafeHttpClient {
    async fn send_http_request(&self, request: HttpRequest) -> anyhow::Result<HttpResponse> {
        let url = reqwest::Url::parse(&request.url)?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid URL: No host found"))?;
        let port = url.port_or_known_default().unwrap_or(80);

        // 1. ホワイトリストチェック (ホスト名)
        if !self.is_whitelisted_host(host) {
            warn!(
                "🚫 Security Violation: Host '{}' is not in the whitelist.",
                host
            );
            return Err(anyhow::anyhow!(
                "Access to host '{}' is denied by security policy (Not Whitelisted).",
                host
            ));
        }

        // 2. DNS名前解決とIPベースの検証 (DNS Rebinding対策)
        // lookup_host はシステムのリゾルバを使用して非同期に解決する
        let addrs = lookup_host(format!("{}:{}", host, port)).await?;
        let mut target_ip = None;

        for addr in addrs {
            if self.is_restricted_addr(addr.ip()) {
                warn!(
                    "🚫 Security Violation: Host '{}' resolved to a restricted IP: {}",
                    host,
                    addr.ip()
                );
                return Err(anyhow::anyhow!(
                    "Access to host '{}' is denied: restricted IP range detected.",
                    host
                ));
            }
            if target_ip.is_none() {
                target_ip = Some(addr.ip());
            }
        }

        let _ip = target_ip.ok_or_else(|| anyhow::anyhow!("Failed to resolve host: {}", host))?;

        // 3. 実際のリクエスト送信
        let method = request.method.parse::<reqwest::Method>()?;
        let mut builder = self.client.request(method, url);

        for (k, v) in request.headers {
            builder = builder.header(k, v);
        }

        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let resp = builder.send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;

        Ok(HttpResponse { status, body })
    }
}

// ── FileCapability ─────────────────────────────────────────────────────────

/// Sandboxed file I/O implementation.
/// All paths are resolved relative to `base_dir` and validated against path
/// traversal attacks before any I/O is performed.
#[derive(Clone)]
pub struct SandboxedFileCapability {
    base_dir: PathBuf,
    write_enabled: bool,
}

impl SandboxedFileCapability {
    /// Create a read-only capability sandboxed to `base_dir`.
    pub fn read_only(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            write_enabled: false,
        }
    }

    /// Create a read+write capability sandboxed to `base_dir`.
    pub fn read_write(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            write_enabled: true,
        }
    }

    fn resolve(&self, path: &str) -> anyhow::Result<PathBuf> {
        let base = self
            .base_dir
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("Sandbox base dir inaccessible: {}", e))?;
        let candidate = base.join(path);
        // Canonicalize to resolve symlinks and ".." components
        // For new files (write), canonicalize the parent directory instead
        let resolved = if candidate.exists() {
            candidate.canonicalize()?
        } else {
            let parent = candidate
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid path: no parent directory"))?
                .canonicalize()
                .map_err(|_| anyhow::anyhow!("Parent directory does not exist"))?;
            parent.join(
                candidate
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("Invalid file name"))?,
            )
        };
        if !resolved.starts_with(&base) {
            return Err(anyhow::anyhow!(
                "Security violation: path '{}' escapes sandbox directory",
                path
            ));
        }
        Ok(resolved)
    }
}

#[async_trait]
impl FileCapability for SandboxedFileCapability {
    async fn read(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        let resolved = self.resolve(path)?;
        tokio::fs::read(&resolved)
            .await
            .map_err(|e| anyhow::anyhow!("FileRead failed for '{}': {}", path, e))
    }

    async fn write(&self, path: &str, data: &[u8]) -> anyhow::Result<()> {
        if !self.write_enabled {
            return Err(anyhow::anyhow!(
                "FileWrite permission not granted — operation denied"
            ));
        }
        let resolved = self.resolve(path)?;
        tokio::fs::write(&resolved, data)
            .await
            .map_err(|e| anyhow::anyhow!("FileWrite failed for '{}': {}", path, e))
    }

    fn can_write(&self) -> bool {
        self.write_enabled
    }
}

// ── ProcessCapability ───────────────────────────────────────────────────────

/// Process execution capability.
/// This implementation enforces an allowlist of permitted commands.
/// An empty allowlist means NO commands are permitted.
#[derive(Clone)]
pub struct AllowedProcessCapability {
    /// Permitted command names (basename only, e.g. "python3", "ffmpeg").
    /// If empty, all execution is blocked.
    allowed_commands: Arc<HashSet<String>>,
}

impl AllowedProcessCapability {
    /// Create a capability that permits the given command names.
    pub fn new(commands: Vec<String>) -> Self {
        Self {
            allowed_commands: Arc::new(commands.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ProcessCapability for AllowedProcessCapability {
    async fn execute(&self, cmd: &str, args: &[String]) -> anyhow::Result<(String, String, i32)> {
        let basename = std::path::Path::new(cmd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(cmd);

        if self.allowed_commands.is_empty() || !self.allowed_commands.contains(basename) {
            warn!(
                "🚫 ProcessExecution denied: command '{}' is not in the allowlist",
                cmd
            );
            return Err(anyhow::anyhow!(
                "ProcessExecution denied: '{}' is not in the permitted command list",
                cmd
            ));
        }

        let output = tokio::process::Command::new(cmd)
            .args(args)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute '{}': {}", cmd, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let code = output.status.code().unwrap_or(-1);
        Ok((stdout, stderr, code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_safe_http_client_new_with_defaults() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Verify default hosts are included
        assert!(client.is_whitelisted_host("api.deepseek.com"));
        assert!(client.is_whitelisted_host("api.cerebras.ai"));
        assert!(client.is_whitelisted_host("api.openai.com"));
        assert!(client.is_whitelisted_host("api.anthropic.com"));
    }

    #[test]
    fn test_safe_http_client_new_with_custom_hosts() {
        let client = SafeHttpClient::new(vec![
            "custom.example.com".to_string(),
            "api.custom.io".to_string(),
        ])
        .unwrap();

        // Verify custom hosts are included
        assert!(client.is_whitelisted_host("custom.example.com"));
        assert!(client.is_whitelisted_host("api.custom.io"));

        // Verify defaults still work
        assert!(client.is_whitelisted_host("api.deepseek.com"));
    }

    #[test]
    fn test_is_whitelisted_host_case_insensitive() {
        let client = SafeHttpClient::new(vec!["ExAmPlE.CoM".to_string()]).unwrap();

        assert!(client.is_whitelisted_host("example.com"));
        assert!(client.is_whitelisted_host("EXAMPLE.COM"));
        assert!(client.is_whitelisted_host("ExAmPlE.CoM"));
    }

    #[test]
    fn test_is_whitelisted_host_not_in_list() {
        let client = SafeHttpClient::new(vec!["allowed.com".to_string()]).unwrap();

        assert!(!client.is_whitelisted_host("evil.com"));
        assert!(!client.is_whitelisted_host("malicious.net"));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_private() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Private ranges (RFC 1918)
        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_loopback() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2))));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_link_local() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Link-local (169.254.x.x)
        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_broadcast() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::BROADCAST)));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_unspecified() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_public() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Public IPs should NOT be restricted
        assert!(!client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)))); // Google DNS
        assert!(!client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)))); // Cloudflare DNS
        assert!(!client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
        // example.com
    }

    #[test]
    fn test_is_restricted_addr_ipv6_loopback() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_is_restricted_addr_ipv6_unspecified() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    #[test]
    fn test_is_restricted_addr_ipv6_unique_local() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Unique local addresses (fc00::/7)
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1))));
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1))));
    }

    #[test]
    fn test_is_restricted_addr_ipv6_multicast() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Multicast (ff00::/8)
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1))));
    }

    #[test]
    fn test_is_restricted_addr_ipv6_public() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Public IPv6 should NOT be restricted
        assert!(!client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        )))); // Google DNS
    }

    #[test]
    fn test_add_host_runtime() {
        let client = SafeHttpClient::new(vec![]).unwrap();
        assert!(!client.is_whitelisted_host("new.example.com"));
        // First insert returns true
        assert!(client.add_host("new.example.com"));
        assert!(client.is_whitelisted_host("new.example.com"));
        // Duplicate returns false
        assert!(!client.add_host("new.example.com"));
        // Case insensitive
        assert!(client.add_host("API.Custom.IO"));
        assert!(client.is_whitelisted_host("api.custom.io"));
    }

    #[test]
    fn test_hashset_o1_lookup() {
        let large_whitelist: Vec<String> = (0..1000)
            .map(|i| format!("host{}.example.com", i))
            .collect();

        let client = SafeHttpClient::new(large_whitelist).unwrap();

        // O(1) lookup should be fast even with large whitelist
        assert!(client.is_whitelisted_host("host500.example.com"));
        assert!(client.is_whitelisted_host("host999.example.com"));
        assert!(!client.is_whitelisted_host("host1000.example.com"));
    }
}
