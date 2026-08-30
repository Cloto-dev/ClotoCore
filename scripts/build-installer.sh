#!/usr/bin/env bash
# Build the marketplace install engine (tools/cloto-installer) for a Rust
# target triple, stamped with the workspace version the kernel requires of
# it, and put it where the kernel looks: beside the app binary.
#
#   scripts/build-installer.sh                      # host triple, Tauri sidecar slot
#   scripts/build-installer.sh --target <triple>    # cross-compile (CGO is off; any host builds any target)
#   scripts/build-installer.sh --out <path>         # elsewhere, e.g. target/debug/cloto-installer
#                                                    # for `cargo run --bin clotocore`
#
# The default output is the Tauri sidecar slot
# (dashboard/src-tauri/binaries/cloto-installer-<triple>[.exe]), which
# `tauri.conf.json` `bundle.externalBin` picks up at build time; a missing
# sidecar fails the app build, so run this before `cargo tauri dev|build`.
#
# Requires a Go toolchain (go.mod names the version). No cgo, no module
# dependencies.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target=""
out=""

while [ $# -gt 0 ]; do
  case "$1" in
    --target) target="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$target" ]; then
  target="$(rustc -vV | sed -n 's/^host: //p')"
  [ -n "$target" ] || { echo "could not determine the host triple from rustc -vV" >&2; exit 1; }
fi

case "$target" in
  x86_64-pc-windows-msvc)     goos=windows goarch=amd64 ;;
  aarch64-pc-windows-msvc)    goos=windows goarch=arm64 ;;
  x86_64-unknown-linux-gnu)   goos=linux   goarch=amd64 ;;
  aarch64-unknown-linux-gnu)  goos=linux   goarch=arm64 ;;
  x86_64-apple-darwin)        goos=darwin  goarch=amd64 ;;
  aarch64-apple-darwin)       goos=darwin  goarch=arm64 ;;
  *) echo "no GOOS/GOARCH mapping for target triple '$target'" >&2; exit 1 ;;
esac

ext=""
[ "$goos" = windows ] && ext=".exe"
if [ -z "$out" ]; then
  out="$here/dashboard/src-tauri/binaries/cloto-installer-${target}${ext}"
fi
# The build runs from the module directory below; a relative --out must
# stay relative to where the caller stood.
case "$out" in
  /*) ;;
  *) out="$(pwd)/$out" ;;
esac

# The kernel accepts the engine only when it reports the kernel's own
# version (managers::installer), so stamp exactly what Cargo.toml says.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -1)"
[ -n "$version" ] || { echo "could not read the workspace version from Cargo.toml" >&2; exit 1; }
commit="$(git -C "$here" rev-parse --short HEAD 2>/dev/null || echo unknown)"

mkdir -p "$(dirname "$out")"
(
  cd "$here/tools/cloto-installer"
  GOOS="$goos" GOARCH="$goarch" CGO_ENABLED=0 \
    go build -trimpath -ldflags "-s -w -X main.version=${version} -X main.commit=${commit}" -o "$out" .
)

# When the build is for this machine, make the binary prove it answers as
# the kernel will ask before anything depends on it.
host="$(rustc -vV | sed -n 's/^host: //p')"
if [ "$target" = "$host" ]; then
  reported="$("$out" version | awk '{print $2}')"
  if [ "$reported" != "$version" ]; then
    echo "built engine reports version '$reported', expected '$version'" >&2
    exit 1
  fi
fi
echo "cloto-installer ${version} (${commit}) for ${target} -> ${out}"
