use anyhow::Result;
use colored::Colorize;

use crate::client::ExivClient;
use crate::output;

pub async fn run(client: &ExivClient, json_mode: bool) -> Result<()> {
    let sp = if !json_mode { Some(output::spinner("Fetching system status...")) } else { None };

    let agents = client.get_agents().await?;
    let plugins = client.get_plugins().await?;
    let metrics = client.get_metrics().await?;

    if let Some(sp) = sp { sp.finish_and_clear(); }

    if json_mode {
        let data = serde_json::json!({
            "endpoint": client.base_url(),
            "agents": {
                "total": agents.len(),
                "online": agents.iter().filter(|a| a.status == "online").count(),
                "offline": agents.iter().filter(|a| a.status != "online" && a.status != "degraded").count(),
                "degraded": agents.iter().filter(|a| a.status == "degraded").count(),
            },
            "plugins": {
                "total": plugins.len(),
                "active": plugins.iter().filter(|p| p.is_active).count(),
            },
            "metrics": metrics,
        });
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    let online = agents.iter().filter(|a| a.status == "online").count();
    let degraded = agents.iter().filter(|a| a.status == "degraded").count();
    // bug-033: Use explicit filter (same as JSON mode) instead of subtraction
    let offline = agents.iter().filter(|a| a.status != "online" && a.status != "degraded").count();

    let active_plugins = plugins.iter().filter(|p| p.is_active).count();

    let total_requests = metrics.get("total_requests").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_memories = metrics.get("total_memories").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_episodes = metrics.get("total_episodes").and_then(|v| v.as_u64()).unwrap_or(0);

    output::print_header("Exiv System Status");

    println!("  {}    v{} ({})",
        "Kernel:".dimmed(),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
    );
    println!("  {}  {}", "Endpoint:".dimmed(), client.base_url());
    println!("  {}    {} registered ({} online, {} offline{})",
        "Agents:".dimmed(),
        agents.len(),
        format!("{online}").green(),
        format!("{offline}").dimmed(),
        if degraded > 0 { format!(", {} degraded", format!("{degraded}").yellow()) } else { String::new() },
    );
    println!("  {}   {} loaded ({} active)",
        "Plugins:".dimmed(),
        plugins.len(),
        format!("{active_plugins}").green(),
    );
    println!("  {}  {} total",
        "Requests:".dimmed(),
        total_requests,
    );
    println!("  {}  {} stored / {} episodes",
        "Memories:".dimmed(),
        total_memories,
        total_episodes,
    );
    println!();

    Ok(())
}
