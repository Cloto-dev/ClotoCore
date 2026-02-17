use colored::Colorize;
use comfy_table::{Table, ContentArrangement, presets::NOTHING};

/// Print a decorated section header.
pub fn print_header(title: &str) {
    let line = "─".repeat(36);
    println!();
    println!("  {}", title.bold());
    println!("  {}", line.dimmed());
}

/// Status dot: ● (online/green), ◐ (degraded/yellow), ○ (offline/dim).
pub fn status_dot(status: &str) -> String {
    match status {
        "online" => "●".green().to_string(),
        "degraded" => "◐".yellow().to_string(),
        _ => "○".dimmed().to_string(),
    }
}

/// Print agents as a rich table.
pub fn print_agents_table(agents: &[exiv_shared::AgentMetadata]) {
    if agents.is_empty() {
        println!("  {}", "No agents registered.".dimmed());
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for agent in agents {
        let agent_type = if agent.default_engine_id.as_deref().map(|e| e.starts_with("mind.")).unwrap_or(false)
            || agent.metadata.get("agent_type").map(|t| t == "ai").unwrap_or(false)
        {
            "AI Agent".cyan().to_string()
        } else {
            "Container".magenta().to_string()
        };

        let engine = agent.default_engine_id.as_deref().unwrap_or("-").dimmed().to_string();

        table.add_row(vec![
            format!("  {}", status_dot(&agent.status)),
            agent.id.clone().bold().to_string(),
            agent_type,
            agent.status.clone(),
            engine,
            agent.description.clone().dimmed().to_string(),
        ]);
    }

    println!("{table}");
}

/// Print plugins as a categorized table.
pub fn print_plugins_table(plugins: &[exiv_shared::PluginManifest]) {
    if plugins.is_empty() {
        println!("  {}", "No plugins loaded.".dimmed());
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for plugin in plugins {
        let active = if plugin.is_active {
            "●".green().to_string()
        } else {
            "○".dimmed().to_string()
        };

        let category = format!("{:?}", plugin.category);

        table.add_row(vec![
            format!("  {active}"),
            plugin.id.clone().bold().to_string(),
            category.dimmed().to_string(),
            plugin.version.clone(),
            plugin.name.clone().dimmed().to_string(),
        ]);
    }

    println!("{table}");
}

/// Create a styled spinner with a message.
pub fn spinner(msg: &str) -> indicatif::ProgressBar {
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("  {spinner} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}
