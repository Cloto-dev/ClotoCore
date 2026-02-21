use anyhow::Result;
use colored::Colorize;
use comfy_table::{presets::NOTHING, ContentArrangement, Table};

use crate::cli::PermissionsCommand;
use crate::client::ExivClient;
use crate::output;

const VALID_PERMISSIONS: &[&str] = &[
    "NetworkAccess",
    "FileRead",
    "FileWrite",
    "ProcessExecution",
    "VisionRead",
    "InputControl",
    "MemoryRead",
    "MemoryWrite",
    "AdminAccess",
];

pub async fn run(client: &ExivClient, cmd: PermissionsCommand, json: bool) -> Result<()> {
    match cmd {
        PermissionsCommand::Pending => pending(client, json).await,
        PermissionsCommand::List { plugin } => list(client, &plugin, json).await,
        PermissionsCommand::Approve { request_id } => approve(client, &request_id, json).await,
        PermissionsCommand::Deny { request_id } => deny(client, &request_id, json).await,
        PermissionsCommand::Grant { plugin, permission } => {
            grant(client, &plugin, &permission, json).await
        }
        PermissionsCommand::Revoke { plugin, permission } => {
            revoke(client, &plugin, &permission, json).await
        }
    }
}

async fn pending(client: &ExivClient, json: bool) -> Result<()> {
    let requests: Vec<serde_json::Value> = client.get_pending_permissions().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&requests)?);
        return Ok(());
    }

    output::print_header("Pending Permission Requests");

    if requests.is_empty() {
        println!("  {}", "No pending requests.".dimmed());
        println!();
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);

    for req in &requests {
        let id = req
            .get("request_id")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let plugin_id = req.get("plugin_id").and_then(|v| v.as_str()).unwrap_or("-");
        let perm_type = req
            .get("permission_type")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let justification = req
            .get("justification")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let target = req
            .get("target_resource")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let perm_colored = match perm_type {
            "NetworkAccess" => perm_type.yellow().to_string(),
            "FileRead" => perm_type.cyan().to_string(),
            "FileWrite" => perm_type.red().to_string(),
            "ProcessExecution" => perm_type.red().bold().to_string(),
            "AdminAccess" => perm_type.red().bold().to_string(),
            _ => perm_type.to_string(),
        };

        let detail = if target.is_empty() {
            justification.dimmed().to_string()
        } else {
            format!("{} {}", justification, format!("({})", target).dimmed())
        };

        table.add_row(vec![
            format!("  {}", "⏳".to_string()),
            id.bold().to_string(),
            plugin_id.to_string(),
            perm_colored,
            detail,
        ]);
    }

    println!("{table}");
    println!();
    println!("  {} exiv permissions approve <ID>", "Approve:".green());
    println!("  {} exiv permissions deny <ID>", "Deny:   ".red());
    println!();

    Ok(())
}

async fn approve(client: &ExivClient, request_id: &str, json: bool) -> Result<()> {
    let sp = if !json {
        Some(output::spinner(&format!("Approving {request_id}...")))
    } else {
        None
    };

    let result = client.approve_permission(request_id).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "  {} Permission request {} approved",
            "✓".green().bold(),
            request_id.bold()
        );
    }

    Ok(())
}

async fn deny(client: &ExivClient, request_id: &str, json: bool) -> Result<()> {
    let sp = if !json {
        Some(output::spinner(&format!("Denying {request_id}...")))
    } else {
        None
    };

    let result = client.deny_permission(request_id).await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "  {} Permission request {} denied",
            "✗".red().bold(),
            request_id.bold()
        );
    }

    Ok(())
}

async fn list(client: &ExivClient, plugin_id: &str, json: bool) -> Result<()> {
    let sp = if !json {
        Some(output::spinner("Fetching permissions..."))
    } else {
        None
    };
    let result = client.get_plugin_permissions(plugin_id).await?;
    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let perms = result
        .get("permissions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    output::print_header(&format!("Permissions: {plugin_id}"));

    // Enforcement legend
    let enforced = ["NetworkAccess", "InputControl"];

    if perms.is_empty() {
        println!("  {}", "No permissions granted.".dimmed());
    } else {
        for p in &perms {
            let name = p.as_str().unwrap_or("-");
            let is_enforced = enforced.contains(&name);
            let badge = if is_enforced {
                "✅ enforced".green()
            } else {
                "⚠  declared only".yellow()
            };
            println!("  {} {}", name.bold(), badge);
        }
    }

    println!();
    println!("  {}", "All 9 permissions:".dimmed());
    for p in VALID_PERMISSIONS {
        let granted = perms.iter().any(|v| v.as_str() == Some(p));
        let is_enforced = enforced.contains(p);
        let marker = if granted {
            "●".green().to_string()
        } else {
            "○".dimmed().to_string()
        };
        let badge = if is_enforced {
            "enforced".green()
        } else {
            "declared".dimmed()
        };
        println!("  {} {:<20} {}", marker, p, badge);
    }
    println!();
    Ok(())
}

async fn revoke(client: &ExivClient, plugin_id: &str, permission: &str, json: bool) -> Result<()> {
    if !VALID_PERMISSIONS.contains(&permission) {
        anyhow::bail!(
            "Invalid permission '{}'. Valid values:\n  {}",
            permission,
            VALID_PERMISSIONS.join(", ")
        );
    }

    let sp = if !json {
        Some(output::spinner(&format!(
            "Revoking {permission} from {plugin_id}..."
        )))
    } else {
        None
    };

    let result = client
        .revoke_plugin_permission(plugin_id, permission)
        .await?;
    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "  {} {} revoked from {}",
            "🔓".to_string(),
            permission.yellow().bold(),
            plugin_id.bold()
        );
    }
    Ok(())
}

async fn grant(client: &ExivClient, plugin_id: &str, permission: &str, json: bool) -> Result<()> {
    // Validate permission name
    if !VALID_PERMISSIONS.contains(&permission) {
        anyhow::bail!(
            "Invalid permission '{}'. Valid values:\n  {}",
            permission,
            VALID_PERMISSIONS.join(", ")
        );
    }

    let sp = if !json {
        Some(output::spinner(&format!(
            "Granting {permission} to {plugin_id}..."
        )))
    } else {
        None
    };

    let result = client
        .grant_plugin_permission(plugin_id, permission)
        .await?;

    if let Some(sp) = sp {
        sp.finish_and_clear();
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "  {} {} granted to {}",
            "🔐".to_string(),
            permission.yellow().bold(),
            plugin_id.bold()
        );
    }

    Ok(())
}
