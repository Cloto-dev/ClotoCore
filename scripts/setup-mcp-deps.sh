#!/usr/bin/env bash
# setup-mcp-deps.sh — Install Python dependencies for all MCP servers.
#
# Creates a shared virtual environment at mcp-servers/.venv and installs
# each server's dependencies via its pyproject.toml.
#
# Usage:
#   bash scripts/setup-mcp-deps.sh
#
# After installation, activate the venv before running the kernel:
#   source mcp-servers/.venv/bin/activate   # Linux/macOS
#   source mcp-servers/.venv/Scripts/activate  # Windows (Git Bash)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENV_DIR="$PROJECT_ROOT/mcp-servers/.venv"

echo "=== Exiv MCP Server Dependency Setup ==="
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
    echo "Creating virtual environment at mcp-servers/.venv ..."
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
for server_dir in "$PROJECT_ROOT"/mcp-servers/*/; do
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
echo "  source mcp-servers/.venv/bin/activate       # Linux/macOS"
echo "  source mcp-servers/.venv/Scripts/activate    # Windows (Git Bash)"
