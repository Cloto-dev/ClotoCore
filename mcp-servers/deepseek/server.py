"""
Cloto MCP Server: DeepSeek
OpenAI-compatible reasoning engine via MCP protocol.
"""

import asyncio
import os
import sys

# Resolve parent directory for common module import.
_script_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(_script_dir, "..")))

from common.llm_provider import create_llm_mcp_server, load_llm_provider_config, run_server

config = load_llm_provider_config(
    prefix="DEEPSEEK",
    display_name="DeepSeek",
    default_model="deepseek-chat",
    supports_tools=True,
)

server = create_llm_mcp_server(config)

if __name__ == "__main__":
    asyncio.run(run_server(server))
