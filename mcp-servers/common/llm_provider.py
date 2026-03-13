"""
Cloto MCP Common: LLM Provider Base
Shared logic for OpenAI-compatible LLM provider MCP servers.
Extracted from deepseek/server.py and cerebras/server.py.

Provides:
- LLM API call via the kernel proxy (MGP S13.4)
- Message building (system prompt, chat messages)
- Response parsing (content extraction, tool-call parsing)
- Common MCP tool definitions and handlers
"""

import json
import os
import platform
import shutil
from dataclasses import dataclass

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent


# ============================================================
# Provider Configuration
# ============================================================


def _detect_host_os() -> str:
    """Build a concise OS summary string for the system prompt.

    Examples:
      "Windows 11 (10.0.26200), shell: PowerShell"
      "Linux 6.5.0-44 (Ubuntu 24.04), shell: bash"
      "Darwin 23.5.0 (macOS 14.5), shell: zsh"
    """
    system = platform.system()       # Windows / Linux / Darwin
    release = platform.release()     # 10.0.26200 / 6.5.0-44 / 23.5.0
    version = platform.version()     # full version string

    if system == "Windows":
        # platform.release() returns "11" on modern Python/Win11, or "10" on older.
        # platform.version() returns the full build string e.g. "10.0.26200".
        win_ver = release  # "10" or "11"
        if release == "10":
            # Disambiguate Win10 vs Win11 via build number
            try:
                build = int(version.split(".")[-1]) if version else 0
                if build >= 22000:
                    win_ver = "11"
            except (ValueError, IndexError):
                pass
        os_part = f"Windows {win_ver} ({version})"
    elif system == "Darwin":
        mac_ver = platform.mac_ver()[0]  # e.g. "14.5"
        os_part = f"macOS {mac_ver} (Darwin {release})" if mac_ver else f"Darwin {release}"
    elif system == "Linux":
        # Try freedesktop os-release for distro name
        distro = ""
        for p in ("/etc/os-release", "/usr/lib/os-release"):
            if os.path.isfile(p):
                try:
                    with open(p) as f:
                        for line in f:
                            if line.startswith("PRETTY_NAME="):
                                distro = line.split("=", 1)[1].strip().strip('"')
                                break
                except OSError:
                    pass
                break
        os_part = f"Linux {release} ({distro})" if distro else f"Linux {release}"
    else:
        os_part = f"{system} {release}"

    # Detect default shell
    shell_path = os.environ.get("SHELL") or os.environ.get("COMSPEC") or ""
    shell_name = os.path.basename(shell_path).removesuffix(".exe") if shell_path else "unknown"
    # On Windows, also check for PowerShell availability
    if system == "Windows" and shell_name in ("cmd", "unknown"):
        if shutil.which("pwsh") or shutil.which("powershell"):
            shell_name = "PowerShell"

    return f"{os_part}, shell: {shell_name}"


_HOST_OS_SUMMARY: str = _detect_host_os()


@dataclass
class ProviderConfig:
    """Configuration for an LLM provider server."""

    provider_id: str
    model_id: str
    api_url: str = "http://127.0.0.1:8082/v1/chat/completions"
    request_timeout: int = 120
    supports_tools: bool = True
    display_name: str = ""

    def __post_init__(self):
        if not self.display_name:
            self.display_name = self.provider_id.capitalize()


# ============================================================
# LLM Utilities (ported from crates/shared/src/llm.rs)
# ============================================================


def model_supports_tools(config: ProviderConfig) -> bool:
    """Check if the configured model supports tool schemas.

    deepseek-reasoner (R1) explicitly does not support tool schemas.
    Providers with supports_tools=False (e.g. Cerebras) always return False.
    """
    if not config.supports_tools:
        return False
    return "reasoner" not in config.model_id


def build_system_prompt(
    agent: dict, tools: list[dict] | None = None
) -> str:
    """Build a 5-layer system prompt for a Cloto agent.

    Layers:
      1. Identity   — agent name + platform intro
      2. Platform   — Cloto local/self-hosted description
      3. Persona    — structured role/expertise/style from metadata.persona
      4. Capabilities — available tools (dynamic), memory, avatar
      5. Behavior   — tool-usage guidance + free-text description
    """
    name = agent.get("name", "Agent")
    description = agent.get("description", "")
    metadata = agent.get("metadata", {})

    lines: list[str] = []

    # --- [1] Identity ---
    lines.append(f"You are {name}, an AI agent running on the Cloto platform.")

    # --- [2] Platform ---
    lines.append(
        "Cloto is a local, self-hosted AI container system — "
        "all data stays on your operator's hardware and is never sent to external services."
    )
    lines.append(
        f"Host OS: {_HOST_OS_SUMMARY}. "
        f"When using execute_command, always use commands native to this OS "
        f"({'e.g. dir, type, findstr, Get-ChildItem' if platform.system() == 'Windows' else 'e.g. ls, cat, grep, find'})."
    )

    # --- [3] Persona (from metadata.persona JSON) ---
    persona_raw = metadata.get("persona", "")
    if persona_raw:
        try:
            p = json.loads(persona_raw) if isinstance(persona_raw, str) else persona_raw
            if p.get("role"):
                lines.append(f"Your role: {p['role']}")
            if p.get("expertise"):
                exp = p["expertise"]
                if isinstance(exp, list):
                    lines.append(f"Your areas of expertise: {', '.join(exp)}")
                else:
                    lines.append(f"Your areas of expertise: {exp}")
            if p.get("communication_style"):
                lines.append(f"Communication style: {p['communication_style']}")
        except (json.JSONDecodeError, TypeError):
            pass

    # --- [4] Capabilities ---
    if metadata.get("preferred_memory"):
        lines.append(
            "You have persistent memory — you can store and recall past conversations."
        )

    avatar_desc = metadata.get("avatar_description", "")
    if avatar_desc:
        lines.append(f"Your visual appearance/avatar: {avatar_desc}")

    # Dynamic tool listing — lets the model know exactly what it can do
    if tools:
        tool_lines = []
        for t in tools:
            fn = t.get("function", {})
            tname = fn.get("name", "")
            tdesc = fn.get("description", "")
            if tname:
                short_desc = tdesc.split(".")[0].strip() if tdesc else ""
                tool_lines.append(f"  - {tname}: {short_desc}")
        if tool_lines:
            lines.append("")
            lines.append(f"You have access to {len(tool_lines)} tools:")
            lines.extend(tool_lines)

    # --- [5] Behavior ---
    lines.append("")
    lines.append(
        "When the user's request can be fulfilled by using a tool, "
        "prefer calling the appropriate tool over guessing or explaining "
        "how to do it manually. Execute first, explain after."
    )
    lines.append(
        "If no tool can help, respond honestly based on your knowledge."
    )
    lines.append(
        "Never state the current time, date, or day of the week without first "
        "verifying it by calling get_current_time. Recalled memories may contain "
        "outdated time references — do not echo or extrapolate from them."
    )
    lines.append(
        "Prefer fast tools. Only use high-latency tools (generate_image, "
        "deep_research, transcribe, analyze_image) when the user explicitly requests them."
    )
    lines.append(
        "Do not call update_profile or archive_episode — "
        "the system handles these automatically in the background."
    )

    if description:
        lines.append("")
        lines.append(description)

    return "\n".join(lines)


def build_chat_messages(
    agent: dict,
    message: dict,
    context: list[dict],
    tools: list[dict] | None = None,
) -> list[dict]:
    """Build the standard OpenAI-compatible messages array.

    Returns [system_message, ...context_messages, user_message].
    When tools are provided, the system prompt includes a dynamic tool listing.
    """
    messages = [{"role": "system", "content": build_system_prompt(agent, tools)}]

    if context:
        messages.append({
            "role": "system",
            "content": (
                "[The following are recalled memories from past conversations. "
                "They are NOT recent messages. Time references in them may be outdated.]"
            ),
        })

    for msg in context:
        source = msg.get("source", {})
        # Handle both serde internally-tagged {"type": "User", ...}
        # and legacy externally-tagged {"User": {...}} formats
        src_type = source.get("type", "") if isinstance(source, dict) else ""
        content = msg.get("content", "")
        if src_type in ("User",) or "User" in source or "user" in source:
            role = "user"
            # Include user name in context messages for multi-user awareness
            ctx_name = source.get("name", "") if isinstance(source, dict) else ""
            if ctx_name and ctx_name not in ("User", ""):
                content = f"[{ctx_name}]: {content}"
        elif src_type in ("Agent",) or "Agent" in source or "agent" in source:
            role = "assistant"
        else:
            role = "system"
        messages.append({"role": role, "content": content})

    if context:
        messages.append({
            "role": "system",
            "content": "[End of recalled memories. Current conversation follows.]",
        })

    # Extract user name from source for multi-user awareness
    source = message.get("source", {})
    user_name = ""
    if isinstance(source, dict) and source.get("type") == "User":
        user_name = source.get("name", "")
    user_content = message.get("content", "")
    if user_name and user_name not in ("User", ""):
        messages.append({"role": "user", "content": f"[{user_name}]: {user_content}"})
    else:
        messages.append({"role": "user", "content": user_content})
    return messages


def _check_api_error(label: str, response_data: dict) -> None:
    """Raise ValueError if the response contains an API error (OpenAI or Cerebras format)."""
    if "error" in response_data:
        error = response_data["error"]
        msg = error.get("message", str(error)) if isinstance(error, dict) else str(error)
        raise ValueError(f"{label} API Error: {msg}")
    if response_data.get("type", "").endswith("error"):
        msg = response_data.get("message", "Unknown error")
        raise ValueError(f"{label} API Error: {msg}")


def parse_chat_content(config: ProviderConfig, response_data: dict) -> str:
    """Extract text content from a chat completions response.

    Ported from llm::parse_chat_content().
    """
    _check_api_error(config.display_name, response_data)

    try:
        return response_data["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as e:
        raise ValueError(
            f"Invalid {label} API response: missing choices[0].message.content: {e}"
        ) from e


def parse_chat_think_result(config: ProviderConfig, response_data: dict) -> dict:
    """Parse a chat completions response into a ThinkResult.

    Returns either:
      {"type": "final", "content": "..."}
    or:
      {"type": "tool_calls", "assistant_content": "...", "calls": [...]}

    Ported from llm::parse_chat_think_result().
    """
    _check_api_error(config.display_name, response_data)

    try:
        choice = response_data["choices"][0]
    except (KeyError, IndexError, TypeError) as e:
        raise ValueError(f"Invalid API response: missing choices[0]: {e}") from e

    message_obj = choice.get("message", {})
    finish_reason = choice.get("finish_reason", "stop")

    if finish_reason == "tool_calls" or "tool_calls" in message_obj:
        tool_calls_arr = message_obj.get("tool_calls", [])
        calls = []
        for tc in tool_calls_arr:
            tc_id = tc.get("id", "")
            function = tc.get("function", {})
            name = function.get("name", "")
            arguments_str = function.get("arguments", "{}")
            try:
                arguments = json.loads(arguments_str)
            except json.JSONDecodeError:
                arguments = {}

            if tc_id and name:
                calls.append(
                    {"id": tc_id, "name": name, "arguments": arguments}
                )

        if calls:
            # Prefer content, fall back to reasoning_content (DeepSeek R1 etc.)
            assistant_content = message_obj.get("content") or message_obj.get("reasoning_content")
            return {
                "type": "tool_calls",
                "assistant_content": assistant_content,
                "calls": calls,
            }

    content = message_obj.get("content", "")
    if content is None:
        content = ""
    return {"type": "final", "content": content}


# ============================================================
# LLM API Call
# ============================================================


class LlmApiError(Exception):
    """Structured error from the LLM proxy with an error code."""

    def __init__(self, message: str, code: str = "unknown", status_code: int = 0):
        super().__init__(message)
        self.message = message
        self.code = code
        self.status_code = status_code


def _sanitize_tool_names(tools: list[dict]) -> tuple[list[dict], dict[str, str]]:
    """Replace dots in tool names with underscores for LLM API compatibility.

    Many LLM providers (DeepSeek, OpenAI) require tool names to match
    ^[a-zA-Z0-9_-]+$. MGP tools use dots (e.g. mgp.health.ping).

    Returns (sanitized_tools, reverse_map) where reverse_map maps
    sanitized names back to original names.
    """
    sanitized = []
    reverse_map: dict[str, str] = {}
    for tool in tools:
        fn = tool.get("function", {})
        original_name = fn.get("name", "")
        safe_name = original_name.replace(".", "_")
        if safe_name != original_name:
            reverse_map[safe_name] = original_name
            tool = json.loads(json.dumps(tool))  # deep copy
            tool["function"]["name"] = safe_name
        sanitized.append(tool)
    return sanitized, reverse_map


def _restore_tool_names(response_data: dict, reverse_map: dict[str, str]) -> dict:
    """Restore original tool names (with dots) in LLM response."""
    if not reverse_map:
        return response_data
    try:
        for choice in response_data.get("choices", []):
            for tc in choice.get("message", {}).get("tool_calls", []):
                fn = tc.get("function", {})
                name = fn.get("name", "")
                if name in reverse_map:
                    fn["name"] = reverse_map[name]
    except (KeyError, TypeError):
        pass
    return response_data


async def call_llm_api(
    config: ProviderConfig,
    messages: list[dict],
    tools: list[dict] | None = None,
) -> dict:
    """Send a request via the kernel LLM proxy (MGP S13.4)."""
    body: dict = {
        "model": config.model_id,
        "messages": messages,
        "stream": False,
    }

    reverse_map: dict[str, str] = {}
    if tools and model_supports_tools(config):
        sanitized, reverse_map = _sanitize_tool_names(tools)
        body["tools"] = sanitized

    try:
        async with httpx.AsyncClient(timeout=config.request_timeout) as client:
            response = await client.post(
                config.api_url,
                json=body,
                headers={
                    "X-LLM-Provider": config.provider_id,
                    "Content-Type": "application/json",
                },
            )
    except httpx.ConnectError:
        raise LlmApiError(
            f"Cannot connect to LLM proxy. Ensure the kernel is running.",
            "connection_failed",
        )
    except httpx.TimeoutException:
        raise LlmApiError(
            f"LLM request timed out after {config.request_timeout}s.",
            "timeout",
        )

    if response.status_code >= 400:
        # Extract structured error from proxy response
        try:
            err_body = response.json()
            err_obj = err_body.get("error", {})
            msg = err_obj.get("message", f"HTTP {response.status_code}")
            code = err_obj.get("code", "unknown")
        except Exception:
            msg = f"HTTP {response.status_code}"
            code = "unknown"
        raise LlmApiError(msg, code, response.status_code)

    return _restore_tool_names(response.json(), reverse_map)


# ============================================================
# Common MCP Tool Definitions
# ============================================================

THINK_INPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "agent": {
            "type": "object",
            "description": "Agent metadata (name, description, metadata)",
        },
        "message": {
            "type": "object",
            "description": "User message with 'content' field",
        },
        "context": {
            "type": "array",
            "description": "Conversation context messages",
            "items": {"type": "object"},
        },
    },
    "required": ["agent", "message", "context"],
}

THINK_WITH_TOOLS_INPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "agent": {
            "type": "object",
            "description": "Agent metadata (name, description, metadata)",
        },
        "message": {
            "type": "object",
            "description": "User message with 'content' field",
        },
        "context": {
            "type": "array",
            "description": "Conversation context messages",
            "items": {"type": "object"},
        },
        "tools": {
            "type": "array",
            "description": "Available tool schemas (OpenAI format)",
            "items": {"type": "object"},
        },
        "tool_history": {
            "type": "array",
            "description": "Prior tool calls and results",
            "items": {"type": "object"},
        },
    },
    "required": [
        "agent",
        "message",
        "context",
        "tools",
        "tool_history",
    ],
}


# ============================================================
# Common MCP Tool Handlers
# ============================================================


def _error_response(error: Exception) -> list[TextContent]:
    """Build a structured error response for tool handlers."""
    if isinstance(error, LlmApiError):
        return [TextContent(type="text", text=json.dumps({
            "error": error.message, "error_code": error.code,
        }))]
    return [TextContent(type="text", text=json.dumps({
        "error": "An unexpected error occurred", "error_code": "internal",
    }))]


async def handle_think(
    config: ProviderConfig, arguments: dict
) -> list[TextContent]:
    """Handle 'think' tool: simple text generation."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])

        messages = build_chat_messages(agent, message, context)
        response_data = await call_llm_api(config, messages)
        content = parse_chat_content(config, response_data)

        return [
            TextContent(
                type="text", text=json.dumps({"type": "final", "content": content})
            )
        ]
    except Exception as e:
        return _error_response(e)


async def handle_think_with_tools(
    config: ProviderConfig, arguments: dict
) -> list[TextContent]:
    """Handle 'think_with_tools' tool: may return tool calls or final text."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])
        tools = arguments.get("tools", [])
        tool_history = arguments.get("tool_history", [])

        messages = build_chat_messages(agent, message, context, tools=tools)
        # Sanitize dot-names in tool_history for LLM API compatibility
        for entry in tool_history:
            if "tool_calls" in entry:
                entry = json.loads(json.dumps(entry))  # deep copy
                for tc in entry.get("tool_calls", []):
                    fn = tc.get("function", {})
                    name = fn.get("name", "")
                    safe = name.replace(".", "_")
                    if safe != name:
                        fn["name"] = safe
            elif entry.get("role") == "tool" and "name" in entry:
                name = entry.get("name", "")
                safe = name.replace(".", "_")
                if safe != name:
                    entry = {**entry, "name": safe}
            messages.append(entry)

        response_data = await call_llm_api(config, messages, tools)
        result = parse_chat_think_result(config, response_data)

        return [TextContent(type="text", text=json.dumps(result))]
    except Exception as e:
        return _error_response(e)


# ============================================================
# Server Lifecycle Helper
# ============================================================


async def run_server(server: Server):
    """Run an MCP server using stdio transport."""
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream, write_stream, server.create_initialization_options()
        )


# ============================================================
# Configuration Loader
# ============================================================


def load_llm_provider_config(
    prefix: str,
    display_name: str,
    default_model: str = "",
    supports_tools: bool = True,
    default_timeout: int = 120,
) -> ProviderConfig:
    """Load an LLM provider config from environment variables.

    Environment variables: {PREFIX}_PROVIDER, {PREFIX}_MODEL,
    {PREFIX}_API_URL, {PREFIX}_TIMEOUT_SECS.

    MGP §8-10 Proxy-Only Architecture:
    When running under OS-level isolation (NetworkScope::ProxyOnly), the kernel
    injects CLOTO_LLM_PROXY / HTTP_PROXY / HTTPS_PROXY env vars pointing to
    the kernel's LLM proxy. All outbound HTTP is expected to route through this
    proxy. Direct API keys are stripped from the child environment.
    """
    import logging
    import os

    logger = logging.getLogger(__name__)

    # CLOTO_LLM_PROXY is injected by the kernel when NetworkScope::ProxyOnly.
    # Use it as the default API base if present.
    proxy_base = os.environ.get("CLOTO_LLM_PROXY")
    default_api_url = (
        f"{proxy_base}/v1/chat/completions"
        if proxy_base
        else "http://127.0.0.1:8082/v1/chat/completions"
    )

    api_url = os.environ.get(f"{prefix}_API_URL", default_api_url)

    # Warn if proxy-only mode is active but api_url points outside localhost.
    if proxy_base and "127.0.0.1" not in api_url and "localhost" not in api_url:
        logger.warning(
            "CLOTO_LLM_PROXY is set (%s) but %s_API_URL (%s) does not point to "
            "localhost. In proxy-only isolation, direct external API calls may be "
            "blocked. Consider removing the custom API URL override.",
            proxy_base, prefix, api_url,
        )

    return ProviderConfig(
        provider_id=os.environ.get(f"{prefix}_PROVIDER", prefix.lower()),
        model_id=os.environ.get(f"{prefix}_MODEL", default_model),
        api_url=api_url,
        request_timeout=int(
            os.environ.get(f"{prefix}_TIMEOUT_SECS", str(default_timeout))
        ),
        supports_tools=supports_tools,
        display_name=display_name,
    )


# ============================================================
# Server Factory
# ============================================================


def create_llm_mcp_server(config: ProviderConfig) -> Server:
    """Create a fully configured LLM MCP server with think/think_with_tools tools.

    Eliminates boilerplate duplication across provider servers.
    """
    from mcp.types import Tool

    server = Server(f"cloto-mcp-{config.provider_id}")

    @server.list_tools()
    async def list_tools() -> list[Tool]:
        tools = [
            Tool(
                name="think",
                description=(
                    f"Generate a text response using {config.display_name} LLM."
                ),
                inputSchema=THINK_INPUT_SCHEMA,
            ),
        ]

        if model_supports_tools(config):
            tools.append(
                Tool(
                    name="think_with_tools",
                    description=(
                        "Generate a response that may include tool calls. "
                        "Returns either final text or a list of tool calls to execute."
                    ),
                    inputSchema=THINK_WITH_TOOLS_INPUT_SCHEMA,
                )
            )

        return tools

    @server.call_tool()
    async def call_tool(name: str, arguments: dict) -> list[TextContent]:
        if name == "think":
            return await handle_think(config, arguments)
        elif name == "think_with_tools":
            return await handle_think_with_tools(config, arguments)
        else:
            return [
                TextContent(
                    type="text",
                    text=json.dumps({"error": f"Unknown tool: {name}"}),
                )
            ]

    return server
