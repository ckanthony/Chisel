#!/usr/bin/env bash
# Build and package Chisel as a .mcpb Desktop Extension.
# Usage: ./bundle/pack.sh [--release]
# Output: bundle/chisel-<version>.mcpb

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROFILE="${1:---release}"

if [[ "$PROFILE" == "--release" ]]; then
  CARGO_PROFILE="release"
  CARGO_FLAG="--release"
else
  CARGO_PROFILE="debug"
  CARGO_FLAG=""
fi

VERSION=$(grep '^version' "$REPO_ROOT/chisel/Cargo.toml" | head -1 | sed 's/.*= *"//' | sed 's/".*//')
OUT_NAME="chisel-${VERSION}.mcpb"
STAGE_DIR="$SCRIPT_DIR/_stage"
OUT_FILE="$SCRIPT_DIR/$OUT_NAME"

echo "→ Building chisel ($CARGO_PROFILE)..."
cd "$REPO_ROOT"
cargo build $CARGO_FLAG -p chisel

echo "→ Staging bundle..."
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/server"

# Copy manifest and icon
cp "$SCRIPT_DIR/manifest.json" "$STAGE_DIR/manifest.json"
if [[ -f "$REPO_ROOT/chisel.jpeg" ]]; then
  cp "$REPO_ROOT/chisel.jpeg" "$STAGE_DIR/icon.png"
fi

# Copy native binary (current platform)
BINARY="$REPO_ROOT/target/$CARGO_PROFILE/chisel"
if [[ ! -f "$BINARY" ]]; then
  echo "error: binary not found at $BINARY" >&2
  exit 1
fi

OS="$(uname -s)"
case "$OS" in
  Darwin)  cp "$BINARY" "$STAGE_DIR/server/chisel"; chmod +x "$STAGE_DIR/server/chisel" ;;
  Linux)   cp "$BINARY" "$STAGE_DIR/server/chisel"; chmod +x "$STAGE_DIR/server/chisel" ;;
  MINGW*|CYGWIN*|MSYS*) cp "$BINARY" "$STAGE_DIR/server/chisel.exe" ;;
  *) echo "error: unsupported OS: $OS" >&2; exit 1 ;;
esac

echo "→ Packing $OUT_NAME..."
rm -f "$OUT_FILE"
cd "$STAGE_DIR"
zip -r "$OUT_FILE" . -x "*.DS_Store"
rm -rf "$STAGE_DIR"

echo ""
echo "✓ $OUT_FILE"
echo ""
echo "Install: drag $OUT_NAME into Claude Desktop → Settings window"
echo "  or:    double-click the file"
