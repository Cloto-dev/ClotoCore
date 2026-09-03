use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "clotocore",
    version = env!("CARGO_PKG_VERSION"),
    about = "ClotoCore - AI Agent Orchestration Platform"
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
        /// Service user (Linux only, default: root; the prefix is chowned to this user)
        #[arg(long)]
        user: Option<String>,
    },
    /// Uninstall Cloto
    // --plan and --execute are the two halves of the enumerated uninstall
    // (DEFENDER_DESIGN.md §7); --tier qualifies whichever one is present, and
    // neither is implied by a bare `uninstall`.
    #[command(group = clap::ArgGroup::new("purge_mode").args(["plan", "execute"]))]
    Uninstall {
        /// Installation directory to remove
        #[arg(long, default_value_os_t = default_prefix())]
        prefix: PathBuf,
        /// Enumerate the full footprint and print it without removing
        /// anything (install receipt + platform artifacts + stray data dirs)
        #[arg(long)]
        plan: bool,
        /// Remove everything the plan lists, after showing it and asking for
        /// confirmation (the enumerated uninstall)
        #[arg(long, conflicts_with = "plan")]
        execute: bool,
        /// Scope tier for --plan / --execute: 1 = application only (default),
        /// 2 = + user data, 3 = + assets and MCP servers, 4 = + everything
        #[arg(long, default_value_t = 1, requires = "purge_mode")]
        tier: u8,
        /// Output the plan (--plan) or the removal report (--execute) as JSON
        #[arg(long, requires = "purge_mode")]
        json: bool,
        /// Skip the confirmation prompt (--execute only)
        #[arg(long, short = 'y', requires = "execute")]
        yes: bool,
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
    /// Diagnose the installation (read-only) — works without a running kernel
    Doctor {
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
        /// Skip checks that need the network (advisory feed)
        #[arg(long)]
        offline: bool,
    },
    /// Repair fixable issues non-destructively (swap remnants, install
    /// receipt, venv) — never touches user data
    Repair {
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
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
    /// Internal: execute a purge plan after the parent exits (used by the uninstall flow)
    #[command(hide = true)]
    PurgeExec {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        json: bool,
        /// Directory tree the plan may touch. Repeatable. This is the half of
        /// the boundary the plan file cannot state about itself: the kernel
        /// passes it here, on a channel that a process which can rewrite the
        /// plan on disk cannot reach. Omitted entirely, the helper derives the
        /// set from its own environment — never from the plan.
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
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
        Commands::Uninstall {
            prefix,
            plan,
            execute,
            tier,
            json,
            yes,
        } => {
            if plan {
                // Read-only enumeration (DEFENDER_DESIGN.md §7). The plan is
                // the capability boundary of the uninstall path, so it is
                // reviewable on its own, before anything can act on it. The
                // prefix is passed through so the dry run describes what the
                // real `uninstall` would remove, receipt or not.
                return crate::defender::purge::run_cli(tier, Some(prefix), json);
            }
            if execute {
                // Same plan, now applied — the plan is shown first and the
                // removal is plan-bound (§8.5).
                return crate::defender::purge_exec::run_uninstall(tier, Some(prefix), yes, json);
            }
            // Legacy path, unchanged: prefix-only removal for installs that
            // predate the receipt.
            info!("🗑️  Uninstalling Cloto from {}", prefix.display());
            crate::installer::uninstall(prefix).await
        }
        Commands::Service { action } => match action {
            ServiceAction::Install { prefix, user } => {
                crate::platform::install_service(&prefix, user.as_deref())
            }
            // `uninstall_service` now distinguishes "removed" from "there was
            // none"; the CLI only needs the failure, which it no longer hides.
            ServiceAction::Uninstall => crate::platform::uninstall_service().map(|_| ()),
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
        Commands::Doctor { json, offline } => crate::defender::doctor::run(json, offline).await,
        Commands::Repair { json } => crate::defender::repair::run_cli(json).await,
        Commands::Version => {
            println!("ClotoCore v{}", env!("CARGO_PKG_VERSION"));
            println!("Build target: {}", env!("TARGET"));
            Ok(())
        }
        Commands::SwapExe { target, pid } => crate::platform::execute_swap(target, pid),
        Commands::PurgeExec {
            plan,
            pid,
            json,
            roots,
        } => crate::defender::purge_exec::run_cli(
            &plan,
            pid,
            json,
            crate::defender::purge_exec::PurgeRoots::from_paths(roots),
        ),
    }
}

/// Run a hidden helper subcommand, if this process was started as one.
///
/// The desktop app embeds the kernel in a GUI binary that never parses a
/// command line — but the uninstall handoff (`DEFENDER_DESIGN.md` §7) copies
/// *the running binary* to a temp directory and re-launches it as
/// `purge-exec`. Whatever binary the kernel lives in therefore has to honour
/// that subcommand, or a desktop installation can stage an uninstall it cannot
/// carry out.
///
/// Returns `None` when this is an ordinary launch, so the caller proceeds
/// exactly as before.
#[must_use]
pub fn run_detached_helper_if_requested() -> Option<anyhow::Result<()>> {
    // Only the helper subcommands. Everything else stays with whichever binary
    // owns the real CLI; a GUI must not start interpreting `uninstall`.
    if std::env::args_os().nth(1)? != *"purge-exec" {
        return None;
    }
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return Some(Err(anyhow::anyhow!("{e}"))),
    };
    match cli.command {
        // Called directly rather than through `dispatch`, which is async: the
        // helper has no runtime, and starting one to run a synchronous file
        // walk would be the only reason this binary needed one.
        Some(Commands::PurgeExec {
            plan,
            pid,
            json,
            roots,
        }) => Some(crate::defender::purge_exec::run_cli(
            &plan,
            pid,
            json,
            crate::defender::purge_exec::PurgeRoots::from_paths(roots),
        )),
        _ => None,
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

    println!("ClotoCore v{} ({})", current_version, target);
    println!("Update repository: github.com/{}", repo);
    println!();

    let client = reqwest::Client::new();
    let ua = format!("ClotoCore/{}", current_version);

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
            println!("Update available. Run `clotocore update` to apply.");
        }
        return Ok(());
    }

    // Find matching binary asset
    let expected_name = format!("clotocore-{}", target);
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

    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS.txt")
        .ok_or_else(|| anyhow::anyhow!("No SHA256SUMS.txt found in release v{}", latest_version))?;

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

    // Download checksums manifest
    print!("Downloading checksums... ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let sums_text = client
        .get(&sums_asset.browser_download_url)
        .header("User-Agent", &ua)
        .send()
        .await?
        .text()
        .await?;
    let expected_hash = sums_text
        .lines()
        .find(|line| line.ends_with(&expected_name))
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| anyhow::anyhow!("No checksum for '{}' in SHA256SUMS.txt", expected_name))?
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

    // Defender install receipt: record the swapped binary (best-effort;
    // app_version refreshes when the new binary boots).
    crate::defender::footprint::record(
        &crate::config::data_dir(),
        vec![crate::defender::footprint::ReceiptEntry::file(
            "binary", &exe_path,
        )],
    );

    println!();
    println!(
        "Updated successfully: v{} -> v{}",
        current_version, latest_version
    );
    println!("SHA256: {}", computed_hash);
    println!();
    println!("Restart the service to use the new version:");
    println!("  clotocore service stop && clotocore service start");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn command_definition_is_internally_consistent() {
        // Catches an unresolvable `requires`/`conflicts_with` id, which clap
        // only reports when the offending flag is actually used.
        Cli::command().debug_assert();
    }

    #[test]
    fn a_bare_uninstall_still_takes_the_legacy_path() {
        let cli = parse(&["clotocore", "uninstall"]).expect("bare uninstall must keep working");
        match cli.command {
            Some(Commands::Uninstall { plan, execute, .. }) => {
                assert!(!plan, "the enumerated dry run must stay opt-in");
                assert!(!execute, "the enumerated removal must stay opt-in");
            }
            _ => panic!("expected the uninstall subcommand"),
        }
    }

    #[test]
    fn tier_qualifies_a_mode_and_the_two_modes_are_exclusive() {
        assert!(
            parse(&["clotocore", "uninstall", "--tier", "4"]).is_err(),
            "a tier without --plan or --execute would silently mean nothing"
        );
        assert!(parse(&["clotocore", "uninstall", "--plan", "--tier", "4"]).is_ok());
        assert!(parse(&["clotocore", "uninstall", "--execute", "--tier", "4"]).is_ok());
        assert!(
            parse(&["clotocore", "uninstall", "--plan", "--execute"]).is_err(),
            "a dry run that also removes things is not a dry run"
        );
        assert!(
            parse(&["clotocore", "uninstall", "--yes"]).is_err(),
            "--yes only waives the confirmation of --execute"
        );
    }

    #[test]
    fn purge_exec_requires_a_plan_and_accepts_an_optional_parent() {
        assert!(
            parse(&["clotocore", "purge-exec"]).is_err(),
            "the helper has no enumeration of its own; without a plan it has nothing to do"
        );
        let cli = parse(&["clotocore", "purge-exec", "--plan", "/tmp/p.json"]).unwrap();
        match cli.command {
            Some(Commands::PurgeExec {
                plan,
                pid,
                json,
                roots,
            }) => {
                assert_eq!(plan, PathBuf::from("/tmp/p.json"));
                assert_eq!(pid, None, "a directly invoked helper waits for nobody");
                assert!(!json);
                assert!(
                    roots.is_empty(),
                    "no roots means the helper derives them itself, not that it has none"
                );
            }
            _ => panic!("expected the purge-exec subcommand"),
        }
        assert!(parse(&[
            "clotocore",
            "purge-exec",
            "--plan",
            "/tmp/p.json",
            "--pid",
            "1234",
            "--json"
        ])
        .is_ok());
    }

    #[test]
    fn the_containment_roots_are_repeatable() {
        // The kernel passes one `--root` per tree it will allow. If clap took
        // only the last, every uninstall would silently narrow to a single
        // root and refuse most of its own plan.
        let cli = parse(&[
            "clotocore",
            "purge-exec",
            "--plan",
            "/tmp/p.json",
            "--root",
            "/opt/cloto",
            "--root",
            "/home/u/.local/share/ClotoCore",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::PurgeExec { roots, .. }) => assert_eq!(
                roots,
                vec![
                    PathBuf::from("/opt/cloto"),
                    PathBuf::from("/home/u/.local/share/ClotoCore"),
                ]
            ),
            _ => panic!("expected the purge-exec subcommand"),
        }
    }

    #[test]
    fn json_qualifies_either_purge_mode() {
        // `--execute --json` was rejected because `json` required `plan`, which
        // left the in-process removal with no machine-readable report at all.
        assert!(parse(&["clotocore", "uninstall", "--execute", "--json"]).is_ok());
        assert!(parse(&["clotocore", "uninstall", "--plan", "--json"]).is_ok());
        assert!(
            parse(&["clotocore", "uninstall", "--json"]).is_err(),
            "a format for an output nobody asked for is not a request"
        );
    }
}
