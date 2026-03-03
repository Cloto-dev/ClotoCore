// Kernel-side tool and code validation for MCP servers.
// Extracted from mcp.rs for separation of concerns.

use anyhow::Result;
use serde_json::Value;
use tracing::warn;
use unicode_normalization::UnicodeNormalization;

// ============================================================
// Kernel-side Tool Validation (Security Feature A)
// ============================================================

/// Blocked shell patterns for the "sandbox" validator.
/// Ported from plugins/terminal/src/sandbox.rs for kernel-level defense-in-depth.
const SANDBOX_BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -fr /",
    "mkfs",
    "dd if=/dev",
    ":(){ :|:& };:",
    "> /dev/sda",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "chmod -r 777 /",
    "chown -r",
    "sudo ",
    "su ",
    "su\t",
    "doas ",
    "/bin/rm -rf",
    "/usr/bin/rm -rf",
];

/// Blocked shell metacharacters for the "sandbox" validator.
const SANDBOX_BLOCKED_METACHAR: &[&str] = &["$(", "`", "|", ";", "&&", "||"];

/// Validate tool arguments at the kernel level before forwarding to an MCP server.
/// This provides defense-in-depth: even if the MCP server's own validation is
/// bypassed (e.g., compromised server), the kernel still catches dangerous inputs.
pub(super) fn validate_tool_arguments(
    validator_name: &str,
    tool_name: &str,
    args: &Value,
) -> Result<()> {
    match validator_name {
        "sandbox" => validate_sandbox_args(tool_name, args),
        other => {
            warn!(
                "Unknown tool validator '{}' for tool '{}', skipping",
                other, tool_name
            );
            Ok(())
        }
    }
}

/// "sandbox" validator: checks command arguments against blocked patterns.
/// Applied to tools like `execute_command` that run shell commands.
fn validate_sandbox_args(_tool_name: &str, args: &Value) -> Result<()> {
    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
        return Ok(()); // No command argument, nothing to validate
    };

    if command.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "Kernel validation: empty command is not allowed"
        ));
    }

    // NFKC normalization: canonicalize Unicode to prevent bypasses via
    // confusable characters (e.g., U+2011 non-breaking hyphen → U+002D hyphen-minus).
    let normalized: String = command.nfkc().collect();
    let lower = normalized.to_lowercase();

    // Block embedded newlines/carriage returns (injection vectors)
    if lower.contains('\n')
        || lower.contains('\r')
        || lower.contains('\u{2028}')
        || lower.contains('\u{2029}')
    {
        return Err(anyhow::anyhow!(
            "Kernel validation: command contains embedded newline or line separator"
        ));
    }

    // Block shell metacharacters
    for meta in SANDBOX_BLOCKED_METACHAR {
        if lower.contains(meta) {
            return Err(anyhow::anyhow!(
                "Kernel validation: command contains blocked shell metacharacter: '{}'",
                meta
            ));
        }
    }

    // Check for blocked patterns
    for pattern in SANDBOX_BLOCKED_PATTERNS {
        if lower.contains(pattern) {
            return Err(anyhow::anyhow!(
                "Kernel validation: command contains blocked pattern: '{}'",
                pattern
            ));
        }
    }

    // Block rm with both -r and -f flags
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    if let Some(first) = tokens.first() {
        if *first == "rm" || first.ends_with("/rm") {
            let has_recursive = tokens.iter().any(|t| {
                t.starts_with('-') && !t.starts_with("--") && (t.contains('r') || t.contains('R'))
            });
            let has_force = tokens
                .iter()
                .any(|t| t.starts_with('-') && !t.starts_with("--") && t.contains('f'));
            if has_recursive && has_force {
                return Err(anyhow::anyhow!(
                    "Kernel validation: command contains dangerous rm flags (-r and -f)"
                ));
            }
        }
    }

    Ok(())
}

// ============================================================
// Code Validator — safety checks for agent-generated MCP code
// ============================================================

/// Blocked imports that could enable system access or code execution.
pub(super) const BLOCKED_IMPORTS: &[&str] = &[
    "subprocess",
    "shutil",
    "socket",
    "ctypes",
    "multiprocessing",
    "signal",
    "pty",
    "fcntl",
    "resource",
    "importlib",
    "code",
    "codeop",
    "compileall",
    "py_compile",
];

/// Blocked function/attribute patterns.
pub(super) const BLOCKED_PATTERNS: &[&str] = &[
    "eval(",
    "exec(",
    "__import__(",
    "compile(",
    "open(",
    "globals(",
    "locals(",
    "os.system",
    "os.popen",
    "os.spawn",
    "os.exec",
    "os.remove",
    "os.unlink",
    "os.rmdir",
    "os.makedirs",
    "subprocess.",
    "__builtins__",
    "getattr(",
    "setattr(",
    "delattr(",
];

/// Maximum allowed code size in bytes.
pub(super) const MAX_CODE_SIZE: usize = 10_000;

/// Validate agent-generated Python code for safety.
/// Returns Ok(()) if code is safe, Err with list of violations otherwise.
pub(super) fn validate_mcp_code(code: &str) -> std::result::Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // L1: Size limit
    if code.len() > MAX_CODE_SIZE {
        errors.push(format!(
            "Code too large: {} bytes (max {})",
            code.len(),
            MAX_CODE_SIZE
        ));
    }

    // Normalize for pattern matching (lowercase for import checks)
    let code_lower = code.to_lowercase();

    // L2: Blocked imports
    for &blocked in BLOCKED_IMPORTS {
        // Match "import subprocess", "from subprocess", "import subprocess,"
        let import_pattern = format!("import {blocked}");
        let from_pattern = format!("from {blocked}");
        if code_lower.contains(&import_pattern) || code_lower.contains(&from_pattern) {
            errors.push(format!("Blocked import: '{blocked}'"));
        }
    }

    // L3: Blocked function/attribute patterns
    for &pattern in BLOCKED_PATTERNS {
        if code.contains(pattern) {
            errors.push(format!("Blocked pattern: '{pattern}'"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
