"""
Exiv MCP Server: Terminal
Sandboxed shell command execution via MCP protocol.
Ported from plugins/terminal/src/lib.rs + sandbox.rs
"""

import asyncio
import json
import os
import unicodedata

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

# ============================================================
# Configuration (from environment variables)
# ============================================================

WORKING_DIR = os.environ.get("EXIV_SANDBOX_DIR", "/tmp/exiv-sandbox")
MAX_OUTPUT_BYTES = int(os.environ.get("EXIV_MAX_OUTPUT_BYTES", "65536"))
ALLOWED_COMMANDS_STR = os.environ.get("EXIV_ALLOWED_COMMANDS", "")

ALLOWED_COMMANDS: list[str] | None = None
if ALLOWED_COMMANDS_STR:
    ALLOWED_COMMANDS = [c.strip() for c in ALLOWED_COMMANDS_STR.split(",") if c.strip()]

# ============================================================
# Sandbox: Command Validation (ported from sandbox.rs)
# ============================================================

BLOCKED_PATTERNS = [
    "rm -rf /", "rm -fr /", "mkfs", "dd if=/dev",
    ":(){ :|:& };:", "> /dev/sda", "shutdown", "reboot",
    "init 0", "init 6", "chmod -r 777 /", "chown -r",
    "sudo ", "su ", "su\t", "doas ",
    "/bin/rm -rf", "/usr/bin/rm -rf",
    "python -c", "python2 -c", "python3 -c",
    "perl -e", "ruby -e", "node -e", "php -r", "lua -e",
    "nc -e", "ncat -e", "socat exec:",
    "shred ", "wipefs",
]

BLOCKED_METACHAR_PATTERNS = [
    "$(", "`", "|", ";", "&&", "||",
]


def validate_command(command: str) -> None:
    """Validate a command against security rules. Raises ValueError on failure."""
    if not command.strip():
        raise ValueError("Empty command is not allowed")

    # NFKC normalization to prevent Unicode homoglyph bypass
    command = unicodedata.normalize("NFKC", command)

    # Block embedded newlines/carriage returns and Unicode line separators
    if "\n" in command or "\r" in command or "\u2028" in command or "\u2029" in command:
        raise ValueError(
            "Command contains embedded newline or line separator (potential injection)"
        )

    lower = command.lower()

    # Block shell metacharacters
    for meta in BLOCKED_METACHAR_PATTERNS:
        if meta in lower:
            raise ValueError(f"Command contains blocked shell metacharacter: '{meta}'")

    # Check for blocked patterns
    for pattern in BLOCKED_PATTERNS:
        if pattern in lower:
            raise ValueError(f"Command contains blocked pattern: '{pattern}'")

    # Block rm with both -r and -f flags
    normalized = " ".join(lower.split())
    if normalized.startswith("rm ") or "/rm " in normalized:
        tokens = normalized.split()
        has_recursive = any(
            t.startswith("-") and not t.startswith("--") and ("r" in t or "R" in t)
            for t in tokens
        )
        has_force = any(
            t.startswith("-") and not t.startswith("--") and "f" in t
            for t in tokens
        )
        if has_recursive and has_force:
            raise ValueError("Command contains dangerous rm flags (-r and -f)")

    # If an allowlist is configured, check the first word
    if ALLOWED_COMMANDS is not None:
        first_word = command.split()[0] if command.split() else ""
        if first_word not in ALLOWED_COMMANDS:
            raise ValueError(
                f"Command '{first_word}' is not in the allowlist. "
                f"Allowed: {ALLOWED_COMMANDS}"
            )


def safe_truncate(s: str, max_bytes: int) -> str:
    """Safely truncate a string at a UTF-8 byte boundary."""
    encoded = s.encode("utf-8")
    if len(encoded) <= max_bytes:
        return s
    truncated = encoded[:max_bytes]
    return truncated.decode("utf-8", errors="ignore")


# ============================================================
# MCP Server
# ============================================================

server = Server("exiv-mcp-terminal")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="execute_command",
            description=(
                "Execute a shell command and return stdout, stderr, and exit code. "
                "Use this to run scripts, check file contents, inspect system state, "
                "compile code, run tests, or perform any command-line operation."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute",
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 30, max: 120)",
                    },
                },
                "required": ["command"],
            },
        )
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name != "execute_command":
        return [TextContent(type="text", text=json.dumps({
            "exit_code": -1,
            "stdout": "",
            "stderr": f"Unknown tool: {name}",
        }))]

    command = arguments.get("command")
    if not command:
        return [TextContent(type="text", text=json.dumps({
            "exit_code": -1,
            "stdout": "",
            "stderr": "Missing 'command' argument",
        }))]

    timeout_secs = min(arguments.get("timeout_secs", 30), 120)

    # Validate command against sandbox rules
    try:
        validate_command(command)
    except ValueError as e:
        return [TextContent(type="text", text=json.dumps({
            "exit_code": -1,
            "stdout": "",
            "stderr": str(e),
        }))]

    # Ensure working directory exists
    os.makedirs(WORKING_DIR, exist_ok=True)

    try:
        proc = await asyncio.create_subprocess_shell(
            command,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=WORKING_DIR,
        )

        try:
            stdout_bytes, stderr_bytes = await asyncio.wait_for(
                proc.communicate(), timeout=timeout_secs
            )
        except asyncio.TimeoutError:
            proc.kill()
            await proc.wait()
            return [TextContent(type="text", text=json.dumps({
                "exit_code": -1,
                "stdout": "",
                "stderr": f"Command timed out after {timeout_secs} seconds",
            }))]

        stdout = stdout_bytes.decode("utf-8", errors="replace")
        stderr = stderr_bytes.decode("utf-8", errors="replace")

        # Safe UTF-8 truncation
        if len(stdout.encode("utf-8")) > MAX_OUTPUT_BYTES:
            stdout = (
                safe_truncate(stdout, MAX_OUTPUT_BYTES)
                + f"...[truncated, {len(stdout_bytes)} bytes total]"
            )
        if len(stderr.encode("utf-8")) > MAX_OUTPUT_BYTES:
            stderr = (
                safe_truncate(stderr, MAX_OUTPUT_BYTES)
                + f"...[truncated, {len(stderr_bytes)} bytes total]"
            )

        exit_code = proc.returncode if proc.returncode is not None else -1

        return [TextContent(type="text", text=json.dumps({
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        }))]

    except Exception as e:
        return [TextContent(type="text", text=json.dumps({
            "exit_code": -1,
            "stdout": "",
            "stderr": f"Failed to execute command: {e}",
        }))]


async def main():
    async with stdio_server() as (read_stream, write_stream):
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())
