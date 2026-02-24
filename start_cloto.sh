#!/bin/bash
# Cloto System - Startup Script (Guardian Managed)
set -e
cd "$(dirname "$0")"

# Load environment variables from .env file if it exists
if [ -f .env ]; then
    set -a
    source .env
    set +a
fi

# Configuration (with defaults; override via .env or environment)
export PORT="${PORT:-8081}"
export CLOTO_DASHBOARD_PATH="${CLOTO_DASHBOARD_PATH:-$(pwd)/dashboard/dist}"
export DATABASE_URL="${DATABASE_URL:-sqlite:$(pwd)/cloto_memories.db}"
export RUST_LOG="${RUST_LOG:-info}"

echo "🛑 Ensuring clean state for dev environment..."
# 安定版(8080)以外の開発用プロセスを掃除
pkill -9 -f cloto_guardian || true
pkill -9 -f target/debug/cloto_core || true
rm -f .maintenance

echo "🔨 Building ClotoCore..."
cargo build

# ガーディアンが監視する実行ファイル
TARGET_BIN="./target/debug/cloto_system"

echo "🚀 Launching ClotoCore via Guardian on port ${PORT}..."
# Guardianはカレントディレクトリのバイナリを監視・実行する
# Guardian monitors the binary and auto-restarts on crash or update.

nohup ./cloto_guardian > kernel.log 2>&1 &
echo "✅ Cloto System is now under Guardian protection (Port: ${PORT})."
echo "📡 You can use 'POST /api/system/shutdown' to trigger a safe maintenance restart."
