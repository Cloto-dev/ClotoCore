#!/usr/bin/env bash
# setup-mcp-deps.sh — Install Python dependencies for all MCP servers.
#
# Reads [paths].servers from mcp.toml to locate the servers directory
# (defaults to mcp-servers/ in the project root for backward compatibility).
# Creates a shared virtual environment and installs each server's dependencies
# via its pyproject.toml.
#
# Usage:
#   bash scripts/setup-mcp-deps.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Read [paths].servers from mcp.toml if available
SERVERS_DIR=""
if [[ -f "$PROJECT_ROOT/mcp.toml" ]]; then
    # Extract paths.servers value (simple TOML parsing)
    SERVERS_DIR=$(grep -A5 '^\[paths\]' "$PROJECT_ROOT/mcp.toml" 2>/dev/null \
        | grep '^servers\s*=' \
        | sed 's/^servers\s*=\s*"\(.*\)"/\1/' \
        | head -1)
fi

# Fallback to legacy location
if [[ -z "$SERVERS_DIR" ]] || [[ ! -d "$SERVERS_DIR" ]]; then
    SERVERS_DIR="$PROJECT_ROOT/mcp-servers"
fi

VENV_DIR="$SERVERS_DIR/.venv"

echo "=== Cloto MCP Server Dependency Setup ==="
echo "Servers directory: $SERVERS_DIR"
echo ""

# Detect Python command
PYTHON=""
for cmd in python3 python; do
    if command -v "$cmd" &>/dev/null; then
        PYTHON="$cmd"
        break
    fi
done

if [[ -z "$PYTHON" ]]; then
    echo "ERROR: Python 3.10+ is required but not found in PATH."
    exit 1
fi

PY_VERSION=$($PYTHON --version 2>&1)
echo "Using: $PY_VERSION ($PYTHON)"

# Create shared venv if it doesn't exist
if [[ ! -d "$VENV_DIR" ]]; then
    echo "Creating virtual environment at $VENV_DIR ..."
    $PYTHON -m venv "$VENV_DIR"
fi

# Activate venv
if [[ -f "$VENV_DIR/bin/activate" ]]; then
    source "$VENV_DIR/bin/activate"
elif [[ -f "$VENV_DIR/Scripts/activate" ]]; then
    source "$VENV_DIR/Scripts/activate"
else
    echo "ERROR: Could not find venv activate script."
    exit 1
fi

echo "Virtual environment activated."
echo ""

# Upgrade pip
python -m pip install --upgrade pip --quiet

# Install each MCP server's dependencies
INSTALLED=0
for server_dir in "$SERVERS_DIR"/*/; do
    server_name=$(basename "$server_dir")
    if [[ -f "$server_dir/pyproject.toml" ]]; then
        echo "  Installing: $server_name"
        pip install "$server_dir" --quiet
        INSTALLED=$((INSTALLED + 1))
    fi
done

echo ""
echo "=== Setup complete ==="
echo "Installed $INSTALLED MCP server(s)."
echo "Virtual environment: $VENV_DIR"
echo ""
echo "Before running the kernel, activate the venv:"
echo "  source $VENV_DIR/bin/activate       # Linux/macOS"
echo "  source $VENV_DIR/Scripts/activate    # Windows (Git Bash)"
