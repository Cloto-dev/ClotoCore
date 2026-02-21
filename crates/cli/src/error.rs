use colored::Colorize;

/// Format an error for CLI display with contextual help messages.
pub fn display_error(err: &anyhow::Error) {
    let msg = format!("{err}");

    if msg.contains("Connection refused")
        || msg.contains("error sending request")
        || msg.contains("tcp connect error")
    {
        eprintln!("  {} Cannot connect to Exiv kernel", "ERROR".red().bold());
        eprintln!(
            "        Is the kernel running? Check with: {}",
            "systemctl status exiv".dimmed()
        );
        eprintln!("        Current endpoint: {}", "exiv config show".dimmed());
    } else if msg.contains("403") || msg.contains("PermissionDenied") {
        eprintln!("  {} Authentication failed", "ERROR".red().bold());
        eprintln!(
            "        Set your API key: {}",
            "exiv config set api_key <key>".dimmed()
        );
    } else {
        eprintln!("  {} {}", "ERROR".red().bold(), msg);
        // Print cause chain
        for cause in err.chain().skip(1) {
            eprintln!("        {} {cause}", "caused by:".dimmed());
        }
    }
}
