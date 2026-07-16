use async_trait::async_trait;
use cloto_shared::{
    FileCapability, HttpRequest, HttpResponse, NetworkCapability, ProcessCapability,
};
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::net::lookup_host;
use tracing::warn;

/// Default HTTP request timeout for capability-driven outbound HTTP. Most
/// third-party APIs (OpenAI, Anthropic, registry probes) comfortably fit in
/// this window, and longer-running work should go through dedicated paths.
const CAPABILITY_HTTP_PROBE_TIMEOUT_SECS: u64 = 30;

/// Whitelist-independent SSRF IP guard: `true` when `ip` falls in a range that
/// outbound requests must never reach — loopback, private, link-local (incl. the
/// cloud-metadata 169.254.0.0/16), broadcast, documentation, unspecified, and the
/// IPv4-mapped / unique-local / multicast IPv6 equivalents. A free function (not
/// only the `SafeHttpClient` method) so non-whitelist callers such as the
/// marketplace `raw_url` download (bug-431) reuse the exact same block-list
/// without holding a client instance.
#[must_use]
pub fn is_restricted_ip(ip: IpAddr) -> bool {
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
            // IPv4-mapped addresses (::ffff:a.b.c.d) must be checked against the
            // IPv4 rules — otherwise ::ffff:127.0.0.1 / ::ffff:169.254.169.254
            // bypass the loopback / link-local guards the V4 branch enforces.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_restricted_ip(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || v6.is_multicast()
        }
    }
}

/// Resolve `host:port` and reject if ANY resolved address is restricted
/// (`is_restricted_ip`). Returns the full resolved address set so the caller can
/// pin a `reqwest` connection to exactly the validated IPs (DNS-rebinding /
/// TOCTOU defense — bug-407). This is the whitelist-independent core of the
/// `SafeHttpClient` SSRF guard, shared with streaming download paths (marketplace
/// `raw_url` install, bug-431) that authorize hosts by catalog provenance rather
/// than the LLM outbound whitelist and cannot use `send_http_request` (which
/// buffers the whole body as a `String`).
pub async fn resolve_unrestricted_addrs(
    host: &str,
    port: u16,
) -> anyhow::Result<Vec<std::net::SocketAddr>> {
    let resolved: Vec<std::net::SocketAddr> =
        lookup_host(format!("{host}:{port}")).await?.collect();

    if resolved.is_empty() {
        return Err(anyhow::anyhow!("Failed to resolve host: {host}"));
    }

    for addr in &resolved {
        if is_restricted_ip(addr.ip()) {
            warn!(
                "🚫 Security Violation: Host '{}' resolved to a restricted IP: {}",
                host,
                addr.ip()
            );
            return Err(anyhow::anyhow!(
                "Access to host '{host}' is denied: restricted IP range detected."
            ));
        }
    }

    Ok(resolved)
}

#[derive(Clone)]
pub struct SafeHttpClient {
    /// L5: Dynamic whitelist wrapped in Arc<RwLock> for runtime host addition
    allowed_hosts: Arc<RwLock<HashSet<String>>>,
}

impl SafeHttpClient {
    pub fn new(allowed_hosts: Vec<String>) -> anyhow::Result<Self> {
        // P1: Hosts are now fully config-driven (no hard-coded defaults)
        // The caller passes default_allowed_api_hosts from AppConfig,
        // which includes the same defaults unless overridden via env var.
        //
        // bug-407: the outbound client is built per request inside
        // `send_http_request` (pinned to the validated IPs via
        // `resolve_to_addrs`) rather than once here, so a shared client cannot
        // re-resolve a whitelisted hostname to an unvalidated address at connect
        // time. No long-lived `reqwest::Client` is held on the struct.
        let hosts: HashSet<String> = allowed_hosts
            .into_iter()
            .map(|h| h.to_lowercase())
            .collect();

        Ok(Self {
            allowed_hosts: Arc::new(RwLock::new(hosts)),
        })
    }

    /// IPアドレスベースでの制限チェック (Principle #5: Strict Permission Isolation).
    /// Delegates to the free `is_restricted_ip`; kept as a method (test-only) to
    /// preserve the `client.is_restricted_addr(..)` test surface after the guard
    /// logic moved to the free function. The production paths call `is_restricted_ip`
    /// / `resolve_unrestricted_addrs` directly, so the method is `#[cfg(test)]`.
    #[cfg(test)]
    #[allow(clippy::unused_self)]
    fn is_restricted_addr(&self, ip: IpAddr) -> bool {
        is_restricted_ip(ip)
    }

    /// ホスト名ベースでのホワイトリストチェック (O(1) HashSet lookup)
    fn is_whitelisted_host(&self, host: &str) -> bool {
        let hosts = self.allowed_hosts.read().unwrap_or_else(|e| {
            tracing::warn!("RwLock poisoned on allowed_hosts read — recovering");
            e.into_inner()
        });
        hosts.contains(&host.to_lowercase())
    }

    /// L5: Add a host to the whitelist at runtime.
    /// Returns true if newly inserted, false if already present.
    #[must_use]
    pub fn add_host(&self, host: &str) -> bool {
        let normalized = host.to_lowercase();
        let mut hosts = self.allowed_hosts.write().unwrap_or_else(|e| {
            tracing::warn!("RwLock poisoned on allowed_hosts write — recovering");
            e.into_inner()
        });
        let inserted = hosts.insert(normalized.clone());
        if inserted {
            tracing::warn!(host = %normalized, "Host added to whitelist at runtime");
        }
        inserted
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

        // 2. DNS resolution + IP validation, then PIN the connection to the
        //    validated addresses (DNS-rebinding / TOCTOU defense — bug-407).
        //
        //    Previously the resolved IP was validated and then discarded, and
        //    the request was issued against the hostname — so reqwest performed
        //    its OWN second DNS resolution at connect time. A whitelisted domain
        //    the attacker controls could answer with a public IP at check time
        //    and 127.0.0.1 / 169.254.169.254 at connect time, defeating the
        //    guard. We now reject if ANY resolved address is restricted, then
        //    pin the client to exactly the addresses we validated. SNI and the
        //    Host header keep using the hostname (reqwest only overrides the IP
        //    lookup), so legitimate multi-IP / HTTPS hosts do not regress.
        let resolved = resolve_unrestricted_addrs(host, port).await?;

        // 3. Build a per-request client pinned to the validated addresses, then
        //    send. The connection can only reach the IPs we checked above.
        //
        // bug-414: redirects are DISABLED. reqwest follows up to 10 redirects by
        // default, and `resolve_to_addrs` only pins the ORIGINAL hostname — a
        // redirect target is re-resolved through the system resolver and never
        // re-runs the whitelist / is_restricted_addr guard. So a whitelisted host
        // the attacker controls could answer `302 Location: http://169.254.169.254/`
        // and reqwest would follow it straight to cloud-metadata / loopback,
        // bypassing every check in one hop. Capability HTTP probes don't need
        // transparent redirect-following; a 3xx is returned to the caller as-is.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                CAPABILITY_HTTP_PROBE_TIMEOUT_SECS,
            ))
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &resolved)
            .build()?;

        let method = request.method.parse::<reqwest::Method>()?;
        let mut builder = client.request(method, url);

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
    #[must_use]
    pub fn read_only(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            write_enabled: false,
        }
    }

    /// Create a read+write capability sandboxed to `base_dir`.
    #[must_use]
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
    #[must_use]
    pub fn new(commands: Vec<String>) -> Self {
        Self {
            allowed_commands: Arc::new(commands.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ProcessCapability for AllowedProcessCapability {
    async fn execute(&self, cmd: &str, args: &[String]) -> anyhow::Result<(String, String, i32)> {
        // bug-473: reject any path-qualified command. The allowlist matches the
        // basename, but the spawn below runs the raw `cmd` — accepting
        // "/writable/dir/python3" would pass the "python3" allowlist check yet
        // execute an attacker-planted binary. Only bare command names may pass
        // (resolved via the process PATH).
        if cmd.contains('/') || cmd.contains('\\') {
            warn!(
                "🚫 ProcessExecution denied: command '{}' must be a bare name, not a path",
                cmd
            );
            return Err(anyhow::anyhow!(
                "ProcessExecution denied: '{}' must be a bare command name, not a path",
                cmd
            ));
        }

        if self.allowed_commands.is_empty() || !self.allowed_commands.contains(cmd) {
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
    use std::collections::HashMap;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_safe_http_client_new_with_caller_hosts() {
        // P1: hosts are fully config-driven — caller passes them, no hard-coded defaults
        let client = SafeHttpClient::new(vec![
            "api.provider-a.test".to_string(),
            "api.provider-b.test".to_string(),
            "api.provider-c.test".to_string(),
        ])
        .unwrap();

        assert!(client.is_whitelisted_host("api.provider-a.test"));
        assert!(client.is_whitelisted_host("api.provider-b.test"));
        assert!(client.is_whitelisted_host("api.provider-c.test"));
    }

    #[test]
    fn test_safe_http_client_new_with_custom_hosts() {
        let client = SafeHttpClient::new(vec![
            "custom.example.com".to_string(),
            "api.custom.io".to_string(),
        ])
        .unwrap();

        assert!(client.is_whitelisted_host("custom.example.com"));
        assert!(client.is_whitelisted_host("api.custom.io"));

        // Hosts not in the list are rejected (no built-in defaults)
        assert!(!client.is_whitelisted_host("api.other-provider.test"));
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
    fn test_is_restricted_addr_ipv6_link_local() {
        // bug-403: fe80::/10 link-local was previously not blocked on the V6 arm.
        let client = SafeHttpClient::new(vec![]).unwrap();
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))));
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0xfebf, 0, 0, 0, 0, 0, 0, 1))));
    }

    #[test]
    fn test_is_restricted_addr_ipv6_mapped_ipv4() {
        // bug-403: ::ffff:a.b.c.d must be checked against the IPv4 rules, otherwise
        // ::ffff:127.0.0.1 / ::ffff:169.254.169.254 reach loopback / cloud metadata.
        let client = SafeHttpClient::new(vec![]).unwrap();
        // ::ffff:127.0.0.1
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001
        ))));
        // ::ffff:169.254.169.254 (cloud metadata endpoint)
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0xffff, 0xa9fe, 0xa9fe
        ))));
        // ::ffff:10.0.0.1 (private)
        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001
        ))));
        // ::ffff:8.8.8.8 (public) must remain allowed
        assert!(!client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0xffff, 0x0808, 0x0808
        ))));
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

    #[tokio::test]
    async fn test_send_http_request_rejects_non_whitelisted_host() {
        // The whitelist gate fires before any DNS resolution.
        let client = SafeHttpClient::new(vec!["allowed.example".to_string()]).unwrap();
        let req = HttpRequest {
            method: "GET".to_string(),
            url: "http://evil.example/".to_string(),
            headers: HashMap::new(),
            body: None,
        };
        let err = client.send_http_request(req).await.unwrap_err().to_string();
        assert!(err.contains("Not Whitelisted"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_send_http_request_blocks_whitelisted_host_resolving_to_loopback() {
        // bug-407 regression: being whitelisted is not sufficient — the send
        // path MUST resolve the host and reject restricted IPs (then pin the
        // connection to the validated set so reqwest cannot re-resolve to an
        // unvalidated address at connect time). `localhost` resolves to loopback
        // on every platform, which is_restricted_addr rejects, so the request is
        // denied before any connection is attempted.
        let client = SafeHttpClient::new(vec!["localhost".to_string()]).unwrap();
        let req = HttpRequest {
            method: "GET".to_string(),
            url: "http://localhost/".to_string(),
            headers: HashMap::new(),
            body: None,
        };
        let err = client.send_http_request(req).await.unwrap_err().to_string();
        assert!(
            err.contains("restricted IP range detected"),
            "expected restricted-IP denial, got: {err}"
        );
    }
}
