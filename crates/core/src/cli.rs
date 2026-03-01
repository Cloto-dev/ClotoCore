use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "cloto_system",
    version = env!("CARGO_PKG_VERSION"),
    about = "Cloto System - AI Agent Orchestration Platform"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Install Cloto to a directory (self-install)
    Install {
        /// Installation directory
        #[arg(long, default_value_os_t = default_prefix())]
        prefix: PathBuf,
        /// Register as OS service (systemd on Linux, sc.exe on Windows)
        #[arg(long)]
        service: bool,
        /// Service user (Linux only, default: current user)
        #[arg(long)]
        user: Option<String>,
    },
    /// Uninstall Cloto
    Uninstall {
        /// Installation directory to remove
        #[arg(long, default_value_os_t = default_prefix())]
        prefix: PathBuf,
    },
    /// Manage OS service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Check for updates and optionally apply them
    Update {
        /// Only check for updates without applying
        #[arg(long)]
        check: bool,
        /// Specific version to install (e.g. "0.2.0")
        #[arg(long)]
        version: Option<String>,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Print version and build information
    Version,
    /// Internal: perform exe swap after parent exits (used by update mechanism)
    #[command(hide = true)]
    SwapExe {
        #[arg(long)]
        target: PathBuf,
        #[arg(long)]
        pid: u32,
    },
}

#[derive(Subcommand)]
pub enum ServiceAction {
    /// Register Cloto as an OS service
    Install {
        #[arg(long, default_value_os_t = default_prefix())]
        prefix: PathBuf,
        #[arg(long)]
        user: Option<String>,
    },
    /// Remove Cloto OS service
    Uninstall,
    /// Start the Cloto service
    Start,
    /// Stop the Cloto service
    Stop,
    /// Show Cloto service status
    Status,
}

fn default_prefix() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\ProgramData\Cloto")
    } else {
        PathBuf::from("/opt/cloto")
    }
}

/// Dispatch CLI subcommands
pub async fn dispatch(cmd: Commands) -> anyhow::Result<()> {
    match cmd {
        Commands::Install {
            prefix,
            service,
            user,
        } => {
            info!("📦 Installing Cloto to {}", prefix.display());
            crate::installer::install(prefix, service, user).await
        }
        Commands::Uninstall { prefix } => {
            info!("🗑️  Uninstalling Cloto from {}", prefix.display());
            crate::installer::uninstall(prefix).await
        }
        Commands::Service { action } => match action {
            ServiceAction::Install { prefix, user } => {
                crate::platform::install_service(&prefix, user.as_deref())
            }
            ServiceAction::Uninstall => crate::platform::uninstall_service(),
            ServiceAction::Start => crate::platform::start_service(),
            ServiceAction::Stop => crate::platform::stop_service(),
            ServiceAction::Status => {
                let status = crate::platform::service_status()?;
                println!("{}", status);
                Ok(())
            }
        },
        Commands::Update {
            check,
            version,
            yes,
        } => update_command(check, version, yes).await,
        Commands::Version => {
            println!("Cloto System v{}", env!("CARGO_PKG_VERSION"));
            println!("Build target: {}", env!("TARGET"));
            Ok(())
        }
        Commands::SwapExe { target, pid } => crate::platform::execute_swap(target, pid),
    }
}

// --- GitHub API types (shared with handlers/update.rs) ---

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize)]
struct GitHubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// H-10: Compare semantic versions. Returns true if `target` is older than `current`.
fn is_downgrade(current: &str, target: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|p| p.split('-').next().and_then(|n| n.parse().ok()))
            .collect()
    };
    let cur = parse(current);
    let tgt = parse(target);
    for i in 0..cur.len().max(tgt.len()) {
        let c = cur.get(i).copied().unwrap_or(0);
        let t = tgt.get(i).copied().unwrap_or(0);
        if t < c {
            return true;
        }
        if t > c {
            return false;
        }
    }
    false
}

#[allow(clippy::too_many_lines)]
async fn update_command(
    check_only: bool,
    target_version: Option<String>,
    yes: bool,
) -> anyhow::Result<()> {
    let repo =
        std::env::var("CLOTO_UPDATE_REPO").unwrap_or_else(|_| "Cloto-dev/ClotoCore".to_string());
    let current_version = env!("CARGO_PKG_VERSION");
    let target = env!("TARGET");

    println!("Cloto System v{} ({})", current_version, target);
    println!("Update repository: github.com/{}", repo);
    println!();

    let client = reqwest::Client::new();
    let ua = format!("Cloto-System/{}", current_version);

    // Resolve the release to check
    let release: GitHubRelease = if let Some(ref ver) = target_version {
        let tag = if ver.starts_with('v') {
            ver.clone()
        } else {
            format!("v{}", ver)
        };
        let url = format!(
            "https://api.github.com/repos/{}/releases/tags/{}",
            repo, tag
        );
        let resp = client
            .get(&url)
            .header("User-Agent", &ua)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("Release {} not found in {}", tag, repo);
        }
        resp.error_for_status()?.json().await?
    } else {
        let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let resp = client
            .get(&url)
            .header("User-Agent", &ua)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            println!("No releases found in repository.");
            return Ok(());
        }
        resp.error_for_status()?.json().await?
    };

    let latest_version = release.tag_name.trim_start_matches('v');

    if latest_version == current_version && target_version.is_none() {
        println!("Already up to date (v{}).", current_version);
        return Ok(());
    }

    println!("  Current: v{}", current_version);
    println!("  Latest:  v{}", latest_version);
    if let Some(ref name) = release.name {
        println!("  Release: {}", name);
    }
    if let Some(ref date) = release.published_at {
        println!("  Date:    {}", date);
    }
    if let Some(ref body) = release.body {
        let notes: String = body.lines().take(5).collect::<Vec<_>>().join("\n");
        if !notes.is_empty() {
            println!("\n  Release notes:\n  {}", notes.replace('\n', "\n  "));
        }
    }
    println!();

    // H-10: Warn on version downgrade
    if is_downgrade(current_version, latest_version) {
        println!(
            "⚠️  WARNING: This would DOWNGRADE from v{} to v{}",
            current_version, latest_version
        );
        println!("   Downgrading may cause compatibility issues.");
        println!();
    }

    if check_only {
        if latest_version != current_version {
            println!("Update available. Run `cloto_system update` to apply.");
        }
        return Ok(());
    }

    // Find matching binary asset
    let expected_name = format!("cloto_system-{}", target);
    let binary_asset = release
        .assets
        .iter()
        .find(|a| a.name == expected_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No binary '{}' found in release v{}. Your platform may not be supported.",
                expected_name,
                latest_version
            )
        })?;

    let sha256_name = format!("{}.sha256", expected_name);
    let sha256_asset = release
        .assets
        .iter()
        .find(|a| a.name == sha256_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No checksum '{}' found in release v{}",
                sha256_name,
                latest_version
            )
        })?;

    println!(
        "Binary:   {} ({:.1} MB)",
        binary_asset.name,
        binary_asset.size as f64 / 1_048_576.0
    );

    // Confirm unless --yes
    if !yes {
        print!(
            "Apply update v{} -> v{}? [y/N] ",
            current_version, latest_version
        );
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Download checksum
    print!("Downloading checksum... ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let expected_hash = client
        .get(&sha256_asset.browser_download_url)
        .header("User-Agent", &ua)
        .send()
        .await?
        .text()
        .await?;
    let expected_hash = expected_hash
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Checksum file is empty or malformed"))?
        .trim()
        .to_lowercase();
    if expected_hash.len() != 64 || !expected_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("Invalid SHA256 checksum format");
    }
    println!("OK");

    // Download binary
    print!("Downloading binary... ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let binary_data = client
        .get(&binary_asset.browser_download_url)
        .header("User-Agent", &ua)
        .send()
        .await?
        .bytes()
        .await?;
    println!("OK ({:.1} MB)", binary_data.len() as f64 / 1_048_576.0);

    // Verify SHA256
    print!("Verifying checksum... ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut hasher = Sha256::new();
    hasher.update(&binary_data);
    let computed_hash = format!("{:x}", hasher.finalize());
    if computed_hash != expected_hash {
        anyhow::bail!(
            "SHA256 mismatch!\n  Expected: {}\n  Got:      {}",
            expected_hash,
            computed_hash
        );
    }
    println!("OK");

    // Write and swap binary
    let exe_path = std::env::current_exe()?;
    let new_path = exe_path.with_extension("new");
    let old_path = exe_path.with_extension("old");

    std::fs::write(&new_path, &binary_data)?;
    crate::platform::set_executable_permission(&new_path)?;
    crate::platform::swap_running_binary(&new_path, &exe_path, &old_path)?;

    println!();
    println!(
        "Updated successfully: v{} -> v{}",
        current_version, latest_version
    );
    println!("SHA256: {}", computed_hash);
    println!();
    println!("Restart the service to use the new version:");
    println!("  cloto_system service stop && cloto_system service start");

    Ok(())
}
