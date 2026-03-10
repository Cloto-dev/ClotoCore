//! MGP §8-10 OS-Level Isolation — Phase 1 (soft isolation, environment-only).
//!
//! Derives per-server [`IsolationProfile`]s from trust level, granted permissions,
//! and optional per-server overrides in `mcp.toml`.  Phase 1 records the profile
//! but does **not** enforce OS-level constraints (cgroups, seccomp, etc.); that is
//! deferred to Phase 2.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};

use super::mcp_mgp::TrustLevel;

// ============================================================
// Scope Enums
// ============================================================

/// Filesystem isolation scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesystemScope {
    /// No filesystem restrictions.
    Unrestricted,
    /// Process can only access its sandbox directory.
    Sandbox,
    /// Read-only access (Phase 2: enforced by OS).
    Readonly,
    /// No filesystem access (Phase 2: enforced by OS).
    None,
}

impl FilesystemScope {
    /// Numeric rank for comparison (higher = more permissive).
    fn rank(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Readonly => 1,
            Self::Sandbox => 2,
            Self::Unrestricted => 3,
        }
    }

    fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_lowercase().as_str() {
            "unrestricted" => Ok(Self::Unrestricted),
            "sandbox" => Ok(Self::Sandbox),
            "readonly" => Ok(Self::Readonly),
            "none" => Ok(Self::None),
            other => bail!("unknown filesystem scope: {other}"),
        }
    }
}

/// Network isolation scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkScope {
    /// No network restrictions.
    Unrestricted,
    /// HTTP only via kernel LLM proxy.
    ProxyOnly,
    /// No network access.
    None,
}

impl NetworkScope {
    fn rank(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::ProxyOnly => 1,
            Self::Unrestricted => 2,
        }
    }

    fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_lowercase().as_str() {
            "unrestricted" => Ok(Self::Unrestricted),
            "proxyonly" | "proxy_only" => Ok(Self::ProxyOnly),
            "none" => Ok(Self::None),
            other => bail!("unknown network scope: {other}"),
        }
    }
}

// ============================================================
// Profile & Config
// ============================================================

/// Complete isolation profile for one MCP server process.
/// Immutable after process spawn (design invariant 2).
#[derive(Debug, Clone)]
pub struct IsolationProfile {
    pub trust_level: TrustLevel,
    pub sandbox_dir: PathBuf,
    pub filesystem_scope: FilesystemScope,
    pub network_scope: NetworkScope,
    /// Phase 2: memory limit in MB (not enforced in Phase 1).
    pub memory_limit_mb: Option<u32>,
    /// Phase 2: max child processes (not enforced in Phase 1).
    pub max_child_processes: Option<u32>,
    /// Phase 2: open file descriptor limit (not enforced in Phase 1).
    pub open_file_limit: Option<u32>,
}

/// Per-server isolation config override from `mcp.toml` `[servers.isolation]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IsolationConfig {
    pub memory_limit_mb: Option<u32>,
    pub cpu_time_limit_secs: Option<u32>,
    pub filesystem_scope: Option<String>,
    pub network_scope: Option<String>,
    pub max_child_processes: Option<u32>,
}

// ============================================================
// Default profile matrix (§3.2)
// ============================================================

struct Defaults {
    filesystem: FilesystemScope,
    network: NetworkScope,
    memory_limit_mb: Option<u32>,
    max_child_processes: Option<u32>,
}

fn defaults_for(trust: &TrustLevel) -> Defaults {
    match trust {
        TrustLevel::Core => Defaults {
            filesystem: FilesystemScope::Unrestricted,
            network: NetworkScope::Unrestricted,
            memory_limit_mb: Option::None,
            max_child_processes: Option::None,
        },
        TrustLevel::Standard => Defaults {
            filesystem: FilesystemScope::Sandbox,
            network: NetworkScope::ProxyOnly,
            memory_limit_mb: Some(512),
            max_child_processes: Some(5),
        },
        TrustLevel::Experimental => Defaults {
            filesystem: FilesystemScope::Sandbox,
            network: NetworkScope::ProxyOnly,
            memory_limit_mb: Some(256),
            max_child_processes: Some(2),
        },
        TrustLevel::Untrusted => Defaults {
            filesystem: FilesystemScope::Sandbox,
            network: NetworkScope::None,
            memory_limit_mb: Some(128),
            max_child_processes: Some(0),
        },
    }
}

// ============================================================
// Permission-driven upgrades (§3.5)
// ============================================================

/// Upgrade scopes based on granted permissions.  Upgrades only widen access,
/// they never restrict it below the default.
fn apply_permission_upgrades(
    permissions: &[String],
    fs_scope: &mut FilesystemScope,
    net_scope: &mut NetworkScope,
) {
    for perm in permissions {
        match perm.as_str() {
            "network.outbound" => {
                if net_scope.rank() < NetworkScope::ProxyOnly.rank() {
                    *net_scope = NetworkScope::ProxyOnly;
                }
            }
            "filesystem.write" => {
                if fs_scope.rank() < FilesystemScope::Sandbox.rank() {
                    *fs_scope = FilesystemScope::Sandbox;
                }
            }
            "filesystem.read" => {
                if fs_scope.rank() < FilesystemScope::Readonly.rank() {
                    *fs_scope = FilesystemScope::Readonly;
                }
            }
            _ => {}
        }
    }
}

// ============================================================
// Override validation (§5.1)
// ============================================================

/// Validate that isolation overrides are permitted for the given trust level.
fn validate_overrides(trust_level: &TrustLevel, overrides: &IsolationConfig) -> anyhow::Result<()> {
    match trust_level {
        // Core: any override allowed.
        TrustLevel::Core => Ok(()),

        TrustLevel::Standard => {
            // Cannot set filesystem or network to Unrestricted.
            if let Some(ref fs) = overrides.filesystem_scope {
                let scope = FilesystemScope::parse(fs)?;
                ensure!(
                    scope != FilesystemScope::Unrestricted,
                    "Standard servers cannot override filesystem to Unrestricted"
                );
            }
            if let Some(ref net) = overrides.network_scope {
                let scope = NetworkScope::parse(net)?;
                ensure!(
                    scope != NetworkScope::Unrestricted,
                    "Standard servers cannot override network to Unrestricted"
                );
            }
            Ok(())
        }

        TrustLevel::Experimental => {
            // Cannot exceed Standard defaults (memory 512, max_procs 5).
            if let Some(mem) = overrides.memory_limit_mb {
                ensure!(
                    mem <= 512,
                    "Experimental servers cannot exceed 512 MB memory limit (requested {mem})"
                );
            }
            if let Some(procs) = overrides.max_child_processes {
                ensure!(
                    procs <= 5,
                    "Experimental servers cannot exceed 5 child processes (requested {procs})"
                );
            }
            // Cannot set filesystem or network to Unrestricted (same as Standard).
            if let Some(ref fs) = overrides.filesystem_scope {
                let scope = FilesystemScope::parse(fs)?;
                ensure!(
                    scope != FilesystemScope::Unrestricted,
                    "Experimental servers cannot override filesystem to Unrestricted"
                );
            }
            if let Some(ref net) = overrides.network_scope {
                let scope = NetworkScope::parse(net)?;
                ensure!(
                    scope != NetworkScope::Unrestricted,
                    "Experimental servers cannot override network to Unrestricted"
                );
            }
            Ok(())
        }

        TrustLevel::Untrusted => {
            // No overrides allowed.
            let has_any = overrides.memory_limit_mb.is_some()
                || overrides.cpu_time_limit_secs.is_some()
                || overrides.filesystem_scope.is_some()
                || overrides.network_scope.is_some()
                || overrides.max_child_processes.is_some();
            ensure!(
                !has_any,
                "Untrusted servers cannot have isolation overrides"
            );
            Ok(())
        }
    }
}

// ============================================================
// Public API
// ============================================================

/// Derive an isolation profile from trust level, permissions, and optional overrides.
///
/// The sandbox directory is placed at `<sandbox_base>/<server_id>/`.
pub fn derive_isolation_profile(
    trust_level: TrustLevel,
    permissions: &[String],
    overrides: Option<&IsolationConfig>,
    server_id: &str,
    sandbox_base: &Path,
) -> anyhow::Result<IsolationProfile> {
    // 1. Validate overrides first.
    if let Some(ovr) = overrides {
        validate_overrides(&trust_level, ovr)?;
    }

    // 2. Start from trust-level defaults.
    let defs = defaults_for(&trust_level);
    let mut fs_scope = defs.filesystem;
    let mut net_scope = defs.network;
    let mut memory_limit_mb = defs.memory_limit_mb;
    let mut max_child_processes = defs.max_child_processes;

    // 3. Apply permission-driven upgrades.
    apply_permission_upgrades(permissions, &mut fs_scope, &mut net_scope);

    // 4. Apply validated overrides.
    if let Some(ovr) = overrides {
        if let Some(ref fs) = ovr.filesystem_scope {
            fs_scope = FilesystemScope::parse(fs)?;
        }
        if let Some(ref net) = ovr.network_scope {
            net_scope = NetworkScope::parse(net)?;
        }
        if let Some(mem) = ovr.memory_limit_mb {
            memory_limit_mb = Some(mem);
        }
        if let Some(procs) = ovr.max_child_processes {
            max_child_processes = Some(procs);
        }
    }

    // 5. Build sandbox path.
    let sandbox_dir = sandbox_base.join(server_id);

    Ok(IsolationProfile {
        trust_level,
        sandbox_dir,
        filesystem_scope: fs_scope,
        network_scope: net_scope,
        memory_limit_mb,
        max_child_processes,
        open_file_limit: Option::None, // Phase 2
    })
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn base() -> &'static Path {
        Path::new("/tmp/sandbox")
    }

    // ---- Default profiles per trust level ----

    #[test]
    fn default_profile_core() {
        let p = derive_isolation_profile(TrustLevel::Core, &[], None, "core-srv", base()).unwrap();
        assert_eq!(p.filesystem_scope, FilesystemScope::Unrestricted);
        assert_eq!(p.network_scope, NetworkScope::Unrestricted);
        assert_eq!(p.memory_limit_mb, None);
        assert_eq!(p.max_child_processes, None);
    }

    #[test]
    fn default_profile_standard() {
        let p =
            derive_isolation_profile(TrustLevel::Standard, &[], None, "std-srv", base()).unwrap();
        assert_eq!(p.filesystem_scope, FilesystemScope::Sandbox);
        assert_eq!(p.network_scope, NetworkScope::ProxyOnly);
        assert_eq!(p.memory_limit_mb, Some(512));
        assert_eq!(p.max_child_processes, Some(5));
    }

    #[test]
    fn default_profile_experimental() {
        let p = derive_isolation_profile(TrustLevel::Experimental, &[], None, "exp-srv", base())
            .unwrap();
        assert_eq!(p.filesystem_scope, FilesystemScope::Sandbox);
        assert_eq!(p.network_scope, NetworkScope::ProxyOnly);
        assert_eq!(p.memory_limit_mb, Some(256));
        assert_eq!(p.max_child_processes, Some(2));
    }

    #[test]
    fn default_profile_untrusted() {
        let p = derive_isolation_profile(TrustLevel::Untrusted, &[], None, "unt-srv", base())
            .unwrap();
        assert_eq!(p.filesystem_scope, FilesystemScope::Sandbox);
        assert_eq!(p.network_scope, NetworkScope::None);
        assert_eq!(p.memory_limit_mb, Some(128));
        assert_eq!(p.max_child_processes, Some(0));
    }

    // ---- Permission-driven upgrades ----

    #[test]
    fn permission_network_outbound_upgrades_untrusted() {
        let perms = vec!["network.outbound".to_string()];
        let p =
            derive_isolation_profile(TrustLevel::Untrusted, &perms, None, "srv", base()).unwrap();
        // Network upgraded from None to ProxyOnly.
        assert_eq!(p.network_scope, NetworkScope::ProxyOnly);
        // Filesystem unchanged.
        assert_eq!(p.filesystem_scope, FilesystemScope::Sandbox);
    }

    #[test]
    fn permission_filesystem_write_ensures_sandbox() {
        // Start with a trust level whose default is already Sandbox; still Sandbox.
        let perms = vec!["filesystem.write".to_string()];
        let p =
            derive_isolation_profile(TrustLevel::Standard, &perms, None, "srv", base()).unwrap();
        assert_eq!(p.filesystem_scope, FilesystemScope::Sandbox);
    }

    #[test]
    fn permission_filesystem_read_ensures_readonly() {
        // Untrusted default filesystem is Sandbox (rank 2), Readonly is rank 1 —
        // upgrade only applies when current rank is below Readonly.
        // Simulate a hypothetical scenario via direct unit test on the helper.
        let mut fs = FilesystemScope::None;
        let mut net = NetworkScope::None;
        apply_permission_upgrades(
            &["filesystem.read".to_string()],
            &mut fs,
            &mut net,
        );
        assert_eq!(fs, FilesystemScope::Readonly);
    }

    #[test]
    fn multiple_permissions_combine() {
        let perms = vec![
            "network.outbound".to_string(),
            "filesystem.read".to_string(),
        ];
        let p =
            derive_isolation_profile(TrustLevel::Untrusted, &perms, None, "srv", base()).unwrap();
        assert_eq!(p.network_scope, NetworkScope::ProxyOnly);
        // Untrusted default is Sandbox (rank 2) which is already above Readonly (rank 1).
        assert_eq!(p.filesystem_scope, FilesystemScope::Sandbox);
    }

    // ---- Override validation ----

    #[test]
    fn untrusted_rejects_any_override() {
        let ovr = IsolationConfig {
            memory_limit_mb: Some(256),
            ..Default::default()
        };
        let result =
            derive_isolation_profile(TrustLevel::Untrusted, &[], Some(&ovr), "srv", base());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Untrusted servers cannot have isolation overrides")
        );
    }

    #[test]
    fn core_accepts_any_override() {
        let ovr = IsolationConfig {
            memory_limit_mb: Some(4096),
            filesystem_scope: Some("unrestricted".to_string()),
            network_scope: Some("unrestricted".to_string()),
            max_child_processes: Some(100),
            ..Default::default()
        };
        let p =
            derive_isolation_profile(TrustLevel::Core, &[], Some(&ovr), "srv", base()).unwrap();
        assert_eq!(p.memory_limit_mb, Some(4096));
        assert_eq!(p.max_child_processes, Some(100));
    }

    #[test]
    fn standard_rejects_unrestricted_filesystem() {
        let ovr = IsolationConfig {
            filesystem_scope: Some("unrestricted".to_string()),
            ..Default::default()
        };
        let result =
            derive_isolation_profile(TrustLevel::Standard, &[], Some(&ovr), "srv", base());
        assert!(result.is_err());
    }

    #[test]
    fn standard_rejects_unrestricted_network() {
        let ovr = IsolationConfig {
            network_scope: Some("unrestricted".to_string()),
            ..Default::default()
        };
        let result =
            derive_isolation_profile(TrustLevel::Standard, &[], Some(&ovr), "srv", base());
        assert!(result.is_err());
    }

    #[test]
    fn experimental_rejects_excessive_memory() {
        let ovr = IsolationConfig {
            memory_limit_mb: Some(1024),
            ..Default::default()
        };
        let result =
            derive_isolation_profile(TrustLevel::Experimental, &[], Some(&ovr), "srv", base());
        assert!(result.is_err());
    }

    #[test]
    fn experimental_accepts_within_standard_limits() {
        let ovr = IsolationConfig {
            memory_limit_mb: Some(512),
            max_child_processes: Some(5),
            ..Default::default()
        };
        let p = derive_isolation_profile(TrustLevel::Experimental, &[], Some(&ovr), "srv", base())
            .unwrap();
        assert_eq!(p.memory_limit_mb, Some(512));
        assert_eq!(p.max_child_processes, Some(5));
    }

    // ---- Sandbox directory path ----

    #[test]
    fn sandbox_dir_uses_server_id() {
        let p = derive_isolation_profile(
            TrustLevel::Standard,
            &[],
            None,
            "my-server",
            Path::new("/var/cloto/sandbox"),
        )
        .unwrap();
        assert_eq!(p.sandbox_dir, PathBuf::from("/var/cloto/sandbox/my-server"));
    }

    #[test]
    fn sandbox_dir_nested_base() {
        let p = derive_isolation_profile(
            TrustLevel::Untrusted,
            &[],
            None,
            "plugin-x",
            Path::new("/home/user/.cloto/sandbox"),
        )
        .unwrap();
        assert_eq!(
            p.sandbox_dir,
            PathBuf::from("/home/user/.cloto/sandbox/plugin-x")
        );
    }
}
