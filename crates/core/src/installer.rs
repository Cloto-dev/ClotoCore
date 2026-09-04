use anyhow::Context;
use std::path::{Path, PathBuf};
use tracing::info;

/// Binary name for the current platform
fn binary_name() -> &'static str {
    if cfg!(windows) {
        "clotocore.exe"
    } else {
        "clotocore"
    }
}

/// Copy the marketplace install engine that ships beside the kernel binary
/// into the install prefix, so the installed kernel can find it next to
/// itself (`managers::installer` looks in the executable's directory).
///
/// Returns the installed path, or `None` when no engine was found beside
/// the source binary — an installation made from a bare `clotocore` file
/// rather than the release archive. That is reported, not fatal: the kernel
/// boots without the engine and refuses marketplace installs with a clear
/// error instead of failing silently.
pub(crate) fn stage_engine(src_dir: &Path, prefix: &Path) -> anyhow::Result<Option<PathBuf>> {
    let name = crate::managers::installer::binary_name();
    let src = src_dir.join(name);
    if !src.is_file() {
        return Ok(None);
    }
    let dst = prefix.join(name);
    if src == dst {
        return Ok(Some(dst));
    }
    std::fs::copy(&src, &dst)
        .with_context(|| format!("Failed to copy {} to {}", src.display(), dst.display()))?;
    crate::platform::set_executable_permission(&dst)?;
    Ok(Some(dst))
}

/// Hand the whole install prefix to the service user. The unit runs as that
/// user and must be able to read `.env` (0600) and write under `data/`.
#[cfg(unix)]
fn chown_recursive(prefix: &Path, user: &str) -> anyhow::Result<()> {
    let status = std::process::Command::new("chown")
        .args(["-R", user])
        .arg(prefix)
        .status()
        .with_context(|| format!("Failed to run chown -R {user} {}", prefix.display()))?;
    if !status.success() {
        anyhow::bail!(
            "chown -R {user} {} failed with exit code {:?}",
            prefix.display(),
            status.code()
        );
    }
    Ok(())
}

/// Home directory of `user`, as the system records it.
///
/// Read out of `getent passwd` rather than the environment: the install runs as
/// root, so `$HOME` here belongs to root and says nothing about the account the
/// unit will run as.
#[cfg(target_os = "linux")]
fn passwd_home(user: &str) -> anyhow::Result<PathBuf> {
    let out = std::process::Command::new("getent")
        .args(["passwd", user])
        .output()
        .with_context(|| format!("Failed to run getent passwd {user}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "service user {user} does not exist — create it before installing, \
             for example `useradd --system --shell /usr/sbin/nologin {user}`"
        );
    }
    let line = String::from_utf8_lossy(&out.stdout);
    parse_passwd_home(&line).ok_or_else(|| {
        anyhow::anyhow!(
            "service user {user} has no home directory of its own; the kernel derives its \
             data directory from that home, so give the account one (for example \
             `usermod --home /home/{user} --move-home {user}`) and install again"
        )
    })
}

/// Field 6 of a passwd line: `name:passwd:uid:gid:gecos:home:shell`.
///
/// An empty field, or the filesystem root, means the account was created
/// without a home of its own. Both are refused rather than turned into a
/// directory tree this install would then populate and chown.
#[cfg(target_os = "linux")]
fn parse_passwd_home(entry: &str) -> Option<PathBuf> {
    let home = entry.lines().next()?.split(':').nth(5)?;
    if home.is_empty() || home == "/" {
        return None;
    }
    Some(PathBuf::from(home))
}

/// Where the kernel reads and writes when it runs as the owner of `home`.
///
/// Mirrors the production branch of [`crate::config::data_dir`]: on Linux
/// `dirs::data_dir()` resolves to `$HOME/.local/share` and the application
/// appends its own directory name. Derived from the passwd home because the
/// installer cannot ask `dirs` about an account it is not running as — which
/// also means the two must be changed together.
#[cfg(target_os = "linux")]
fn service_user_data_dir(home: &Path) -> PathBuf {
    home.join(".local")
        .join("share")
        .join(crate::config::APP_DATA_DIR_NAME)
}

/// The service user's data directory, plus the path to hand over once the
/// install has finished writing into it.
#[cfg(target_os = "linux")]
#[derive(Debug)]
struct ServiceUserData {
    /// Where the running kernel will keep its data.
    data_dir: PathBuf,
    /// What to `chown` at the end: the home when this install created it,
    /// otherwise only the data directory — an existing home may hold files
    /// that are not ours to re-own.
    chown_root: PathBuf,
}

/// Give the service user a home and a data directory it can write.
///
/// The kernel derives its data directory from the home of whoever runs it, so
/// an account created without one (`useradd -M`) leaves the daemon with nowhere
/// to write. Nothing later in the lifecycle can repair that: `/home` is
/// root-owned while the kernel runs as the service user. The install is the
/// only step that runs as root, so it is the only step that can.
///
/// Left undone, the install and the first start both still exit 0 while the
/// data directory, the install receipt and every marketplace install fail — the
/// last of them reporting only into a progress stream that an unattended daemon
/// has nobody to read.
#[cfg(target_os = "linux")]
fn prepare_service_user_data(user: &str) -> anyhow::Result<ServiceUserData> {
    let home = passwd_home(user)?;
    let home_existed = home.is_dir();
    let data_dir = service_user_data_dir(&home);
    std::fs::create_dir_all(&data_dir).with_context(|| {
        format!(
            "Failed to create the data directory {} for service user {user}",
            data_dir.display()
        )
    })?;
    let chown_root = if home_existed { data_dir.clone() } else { home };
    Ok(ServiceUserData {
        data_dir,
        chown_root,
    })
}

/// Generate .env file content
fn env_template(prefix: &Path, api_key: &str) -> String {
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let db_path = prefix.join("data").join("cloto_memories.db");

    format!(
        r#"# ============================================================
# ClotoCore Configuration
# Generated by ClotoCore install on {timestamp}
# ============================================================

# --- Server ---
PORT=8081
RUST_LOG=info

# --- Security (Principle #5: Strict Permission Isolation) ---
# Admin API key for protected endpoints (plugin config, permissions, shutdown).
# REQUIRED: Without this, all admin operations are denied in release builds.
# Pass via X-API-Key header.
CLOTO_API_KEY={api_key}

# --- Database ---
# SQLite database path. The data/ directory is created automatically.
DATABASE_URL=sqlite:{db_path}

# --- AI Provider API Keys ---
# Uncomment and set to enable reasoning engines.
# DEEPSEEK_API_KEY=
# CEREBRAS_API_KEY=

# --- Consensus (Principle #8: Dynamic Intelligence Orchestration) ---
# Multi-engine deliberation: a message prefixed with "consensus:" fans out to
# the agent's assigned reasoning engines, and a synthesizer engine merges the
# independent proposals into one answer delivered as the agent's response.
# Engines are sourced from the agent's granted engine servers — assign at
# least CONSENSUS_MIN_PROPOSALS engines (default 2) to the agent in the
# Dashboard. CONSENSUS_ENGINES only narrows that per-agent set
# (comma-separated IDs; empty = use all of the agent's engines) — it never
# adds an engine the agent was not assigned.
# CONSENSUS_ENGINES=

# --- Agent ---
# DEFAULT_AGENT_ID=agent.cloto_default

# --- Tuning ---
# MAX_EVENT_DEPTH=10
# PLUGIN_EVENT_TIMEOUT_SECS=30
# MEMORY_CONTEXT_LIMIT=10

# --- Remote Update (Principle #8: HITL) ---
# GitHub repository for update distribution (owner/repo).
# Used by GET /api/system/update/check and POST /api/system/update/apply.
# CLOTO_UPDATE_REPO=Cloto-dev/ClotoCore

# --- Network ---
# CORS origins (comma-separated). The embedded dashboard is served from
# the same origin, so CORS is only needed for external API clients.
# CORS_ORIGINS=http://localhost:5173

# --- MCP Servers ---
# Path to clotohub-servers/servers directory.
# Required for MCP server auto-discovery from mcp.toml.
# CLOTO_MCP_SERVERS=C:\path\to\clotohub-servers\servers

# --- Plugin Network Access (Principle #5) ---
# Additional hosts that plugins with NetworkAccess permission may reach.
# Default whitelist: api.deepseek.com, api.cerebras.ai, api.openai.com, api.anthropic.com
# ALLOWED_HOSTS=
"#,
        timestamp = timestamp,
        api_key = api_key,
        db_path = db_path.display(),
    )
}

/// Install Cloto to the specified prefix directory
pub async fn install(prefix: PathBuf, service: bool, user: Option<String>) -> anyhow::Result<()> {
    println!("=== ClotoCore Installer ===");
    println!("  Prefix:  {}", prefix.display());
    println!("  Service: {}", service);
    println!();

    // 1. Create directories
    let scripts_dir = prefix.join("scripts");
    let data_dir = prefix.join("data");
    std::fs::create_dir_all(&scripts_dir)
        .with_context(|| format!("Failed to create {}", scripts_dir.display()))?;
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("Failed to create {}", data_dir.display()))?;
    info!("📁 Created directories");

    // 2. Copy self (binary) to prefix
    let src_exe = std::env::current_exe().context("Cannot determine current executable path")?;
    let dst_exe = prefix.join(binary_name());
    if src_exe == dst_exe {
        info!("📦 Binary already in place");
    } else {
        std::fs::copy(&src_exe, &dst_exe)
            .with_context(|| format!("Failed to copy binary to {}", dst_exe.display()))?;
        crate::platform::set_executable_permission(&dst_exe)?;
        info!("📦 Installed binary: {}", dst_exe.display());
    }

    // 2b. Marketplace install engine: ships beside the kernel in the release
    //     archive and must end up beside it in the prefix too.
    let engine_src_dir = src_exe
        .parent()
        .map_or_else(|| prefix.clone(), Path::to_path_buf);
    let engine = stage_engine(&engine_src_dir, &prefix)?;
    if let Some(p) = &engine {
        info!("📦 Installed marketplace engine: {}", p.display());
    } else {
        tracing::warn!(
            "marketplace install engine ({}) not found beside {}; marketplace installs will be refused until it is placed in {}",
            crate::managers::installer::binary_name(),
            engine_src_dir.display(),
            prefix.display()
        );
    }

    // 3. Generate .env (skip if exists)
    let env_path = prefix.join(".env");
    if env_path.exists() {
        info!("ℹ️  .env already exists, skipping");
    } else {
        let api_key = crate::apikey::generate();
        let env_content = env_template(&prefix, &api_key);
        std::fs::write(&env_path, env_content).context("Failed to write .env")?;
        // Restrict .env permissions (contains CLOTO_API_KEY)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600))
                .context("Failed to set .env permissions to 0600")?;
        }
        // bug-463: the Unix 0600 hardening had no Windows counterpart, leaving
        // the admin-key file readable by the local Users group under the default
        // ProgramData ACL. Strip inheritance and grant only the current user.
        #[cfg(windows)]
        {
            if let Ok(user) = std::env::var("USERNAME") {
                if !user.is_empty() {
                    let status = std::process::Command::new("icacls")
                        .args([
                            &env_path.display().to_string(),
                            "/inheritance:r",
                            "/grant:r",
                            &format!("{user}:F"),
                        ])
                        .status();
                    if !matches!(status, Ok(s) if s.success()) {
                        tracing::warn!("Failed to tighten .env ACL via icacls; secret may be readable by other local users");
                    }
                }
            }
        }
        info!("🔑 Generated .env with CLOTO_API_KEY");
        println!("  CLOTO_API_KEY has been auto-generated. Save it securely:");
        println!("  {}", api_key);
    }

    // 3b. A service user other than the installer's must own what it runs.
    #[cfg(unix)]
    if let Some(u) = user.as_deref() {
        chown_recursive(&prefix, u)?;
        info!("👤 Prefix owned by {u}");
    }

    // 3c. That user also needs somewhere of its own to write: the kernel derives
    //     its data directory from the home of the account it runs as, not from
    //     the prefix. Linux-only, matching `--user` itself — it is the only
    //     platform whose service unit runs as another account, and `getent` is
    //     not portable to the other unixes.
    #[cfg(target_os = "linux")]
    let service_user_data = match user.as_deref() {
        Some(u) => {
            let prepared = prepare_service_user_data(u)?;
            info!("📁 Data directory for {u}: {}", prepared.data_dir.display());
            Some(prepared)
        }
        None => None,
    };

    // 4. Register service (optional)
    if service {
        crate::platform::install_service(&prefix, user.as_deref())?;
    }

    // 4b. Defender install receipt (DEFENDER_DESIGN.md §3): ledger every
    // path this install created. Best-effort — never fails the install.
    {
        use crate::defender::footprint::ReceiptEntry;
        let mut entries = vec![ReceiptEntry::file("binary", &dst_exe)];
        if let Some(p) = &engine {
            entries.push(ReceiptEntry::file("engine", p));
        }
        entries.extend([
            ReceiptEntry::dir("install_prefix", &prefix),
            ReceiptEntry::dir("install_scripts", &scripts_dir),
            ReceiptEntry::dir("install_data", &data_dir),
            ReceiptEntry::file("env", &env_path).secret(),
        ]);
        if service {
            entries.push(ReceiptEntry::service("service", service_name()));
        }
        // The receipt belongs where the running kernel will look for it. Recorded
        // against the installer's own data directory instead, it lands in root's
        // tree — which on a stock Debian does not exist, so the ledger the
        // defender calls its canonical source is simply never written.
        #[cfg(target_os = "linux")]
        let receipt_dir = service_user_data
            .as_ref()
            .map_or_else(crate::config::data_dir, |d| d.data_dir.clone());
        #[cfg(not(target_os = "linux"))]
        let receipt_dir = crate::config::data_dir();
        crate::defender::footprint::record(&receipt_dir, entries);
    }

    // 4c. Hand over the service user's own tree last, so it owns the receipt
    //     just written into it.
    #[cfg(target_os = "linux")]
    if let (Some(d), Some(u)) = (&service_user_data, user.as_deref()) {
        chown_recursive(&d.chown_root, u)?;
    }

    // 5. Summary
    println!();
    println!("=== Installation complete ===");
    println!();
    println!(
        "  To run manually:  cd {} && ./{}",
        prefix.display(),
        binary_name()
    );
    if service {
        if cfg!(windows) {
            println!("  As service:       sc.exe start Cloto");
        } else {
            println!("  As service:       sudo systemctl start cloto");
        }
    }
    println!("  Dashboard:        http://localhost:8081");
    println!();

    Ok(())
}

/// OS service identifier as registered by `crate::platform::install_service`.
fn service_name() -> &'static str {
    if cfg!(windows) {
        "Cloto"
    } else if cfg!(target_os = "macos") {
        "com.cloto.system"
    } else {
        "cloto"
    }
}

/// Uninstall Cloto from the specified prefix directory
pub async fn uninstall(prefix: PathBuf) -> anyhow::Result<()> {
    println!("=== ClotoCore Uninstaller ===");

    // Stop and remove service
    let _ = crate::platform::uninstall_service();

    if prefix.exists() {
        println!("Removing {}...", prefix.display());
        std::fs::remove_dir_all(&prefix)
            .with_context(|| format!("Failed to remove {}", prefix.display()))?;
    }

    // Defender install receipt: drop the entries this path removed
    // (best-effort; the user-data dir itself is untouched by this command).
    let receipt_dir = crate::config::data_dir();
    for id in [
        "binary",
        "install_prefix",
        "install_scripts",
        "install_data",
        "env",
        "service",
    ] {
        crate::defender::footprint::remove(&receipt_dir, id);
    }

    println!("Cloto uninstalled.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_name() -> &'static str {
        crate::managers::installer::binary_name()
    }

    /// The gecos field is empty in a `useradd --system` entry, so an
    /// off-by-one read lands on it and returns `None` rather than a wrong
    /// path — which would look like "no home" instead of a misparse. The
    /// shell in the last field is a real absolute path, so reading one field
    /// too far yields a plausible-looking directory: both neighbours have to
    /// be wrong answers for this to have any power.
    #[cfg(target_os = "linux")]
    #[test]
    fn passwd_home_reads_the_home_field_not_its_neighbours() {
        let entry = "cloto:x:999:989:Cloto service:/home/cloto:/usr/sbin/nologin\n";
        assert_eq!(
            parse_passwd_home(entry),
            Some(PathBuf::from("/home/cloto")),
            "field 6 is the home directory"
        );
        // Guard the asymmetry the case above depends on: neither neighbour may
        // equal the answer, or this fixture would pass a misparse.
        let fields: Vec<&str> = entry.trim_end().split(':').collect();
        assert_ne!(fields[4], "/home/cloto");
        assert_ne!(fields[6], "/home/cloto");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn passwd_home_refuses_an_account_without_one() {
        // `useradd --system` without `--create-home` leaves the field empty on
        // some distributions and `/` on others. Accepting either would make the
        // install populate and chown a directory that is not the account's.
        assert_eq!(parse_passwd_home("svc:x:1:1:::/usr/sbin/nologin"), None);
        assert_eq!(parse_passwd_home("svc:x:1:1::/:/usr/sbin/nologin"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn passwd_home_takes_the_first_entry_only() {
        // getent prints one line per match. A duplicate from a second name
        // service must not silently redirect the data directory to another
        // account's home.
        let two = "cloto:x:999:989::/home/cloto:/usr/sbin/nologin\n\
                   other:x:1000:1000::/home/other:/bin/sh\n";
        assert_eq!(parse_passwd_home(two), Some(PathBuf::from("/home/cloto")));
    }

    /// Pins the path rule this file has to keep in step with
    /// `config::data_dir`'s production branch. It cannot be compared against
    /// that function directly: under `cargo test` the running binary sits in a
    /// Cargo workspace, so `data_dir()` takes its dev-layout branch and never
    /// evaluates the `$HOME/.local/share` rule this mirrors.
    #[cfg(target_os = "linux")]
    #[test]
    fn service_user_data_dir_is_where_the_kernel_will_look() {
        assert_eq!(
            service_user_data_dir(Path::new("/home/cloto")),
            Path::new("/home/cloto/.local/share").join(crate::config::APP_DATA_DIR_NAME)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_service_user_data_refuses_an_unknown_account() {
        // Fail closed: inventing a home for an account that does not exist
        // would create a tree nothing can chown to it.
        let err = prepare_service_user_data("cloto-nonexistent-test-account")
            .expect_err("an unknown account must not resolve to a data directory");
        let msg = err.to_string();
        assert!(
            msg.contains("does not exist"),
            "the error must say the account is missing, got: {msg}"
        );
    }

    #[test]
    fn stage_engine_copies_the_engine_beside_the_kernel() {
        let src = tempfile::tempdir().unwrap();
        let prefix = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join(engine_name()), b"engine").unwrap();

        let staged = stage_engine(src.path(), prefix.path()).unwrap();
        let dst = prefix.path().join(engine_name());
        assert_eq!(staged.as_deref(), Some(dst.as_path()));
        assert_eq!(std::fs::read(&dst).unwrap(), b"engine");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(&dst).unwrap().permissions().mode() & 0o111,
                0
            );
        }
    }

    #[test]
    fn stage_engine_reports_a_missing_engine_instead_of_failing() {
        let src = tempfile::tempdir().unwrap();
        let prefix = tempfile::tempdir().unwrap();
        assert!(stage_engine(src.path(), prefix.path()).unwrap().is_none());
        assert!(!prefix.path().join(engine_name()).exists());
    }

    #[test]
    fn stage_engine_is_a_no_op_when_already_in_place() {
        let prefix = tempfile::tempdir().unwrap();
        std::fs::write(prefix.path().join(engine_name()), b"engine").unwrap();
        let staged = stage_engine(prefix.path(), prefix.path()).unwrap();
        assert!(staged.is_some());
        assert_eq!(
            std::fs::read(prefix.path().join(engine_name())).unwrap(),
            b"engine"
        );
    }
}
