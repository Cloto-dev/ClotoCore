use async_trait::async_trait;
use tracing::warn;
use exiv_shared::{HttpRequest, HttpResponse, NetworkCapability};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use tokio::net::lookup_host;

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
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_broadcast() || v4.is_documentation() || v4.is_unspecified() || v4.octets()[0] == 0
            }
            IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00 == 0xfc00) || v6.is_multicast()
            }
        }
    }

    /// ホスト名ベースでのホワイトリストチェック (O(1) HashSet lookup)
    fn is_whitelisted_host(&self, host: &str) -> bool {
        let hosts = self.allowed_hosts.read()
            .expect("SafeHttpClient whitelist lock poisoned");
        hosts.contains(&host.to_lowercase())
    }

    /// L5: Add a host to the whitelist at runtime.
    /// Returns true if newly inserted, false if already present.
    pub fn add_host(&self, host: &str) -> bool {
        let normalized = host.to_lowercase();
        let mut hosts = self.allowed_hosts.write()
            .expect("SafeHttpClient whitelist lock poisoned");
        hosts.insert(normalized)
    }
}

#[async_trait]
impl NetworkCapability for SafeHttpClient {
    async fn send_http_request(&self, request: HttpRequest) -> anyhow::Result<HttpResponse> {
        let url = reqwest::Url::parse(&request.url)?;
        let host = url.host_str().ok_or_else(|| anyhow::anyhow!("Invalid URL: No host found"))?;
        let port = url.port_or_known_default().unwrap_or(80);

        // 1. ホワイトリストチェック (ホスト名)
        if !self.is_whitelisted_host(host) {
            warn!("🚫 Security Violation: Host '{}' is not in the whitelist.", host);
            return Err(anyhow::anyhow!("Access to host '{}' is denied by security policy (Not Whitelisted).", host));
        }

        // 2. DNS名前解決とIPベースの検証 (DNS Rebinding対策)
        // lookup_host はシステムのリゾルバを使用して非同期に解決する
        let addrs = lookup_host(format!("{}:{}", host, port)).await?;
        let mut target_ip = None;

        for addr in addrs {
            if self.is_restricted_addr(addr.ip()) {
                warn!("🚫 Security Violation: Host '{}' resolved to a restricted IP: {}", host, addr.ip());
                return Err(anyhow::anyhow!("Access to host '{}' is denied: restricted IP range detected.", host));
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
        ]).unwrap();

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

        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
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

        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255))));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_unspecified() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
    }

    #[test]
    fn test_is_restricted_addr_ipv4_public() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        // Public IPs should NOT be restricted
        assert!(!client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)))); // Google DNS
        assert!(!client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)))); // Cloudflare DNS
        assert!(!client.is_restricted_addr(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)))); // example.com
    }

    #[test]
    fn test_is_restricted_addr_ipv6_loopback() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))));
    }

    #[test]
    fn test_is_restricted_addr_ipv6_unspecified() {
        let client = SafeHttpClient::new(vec![]).unwrap();

        assert!(client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0))));
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
        assert!(!client.is_restricted_addr(IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)))); // Google DNS
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
