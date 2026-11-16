#!/usr/bin/env bash
# Package an already built native macOS app and MCP server. Does not upload anything.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${1:-}"
case "$ARCH" in
  arm64) MACH_ARCH="arm64" ;;
  x64) MACH_ARCH="x86_64" ;;
  *) echo "Usage: scripts/package-release.sh arm64|x64" >&2; exit 2 ;;
esac

APP="$ROOT/target/release/bundle/macos/MontyBrain.app"
MCP="$ROOT/target/release/montybrain-mcp"
for binary in "$APP/Contents/MacOS/montybrain-desktop" "$MCP"; do
  [ -x "$binary" ] || { echo "Build the app and MCP server first: missing $binary" >&2; exit 1; }
  [ "$(lipo -archs "$binary")" = "$MACH_ARCH" ] || {
    echo "Architecture mismatch: $binary is not a native $MACH_ARCH build." >&2; exit 1;
  }
done

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
NAME="MontyBrain-macos-$ARCH"
DEST="$STAGE/$NAME"
mkdir -p "$DEST/bin" "$ROOT/release"
ditto "$APP" "$DEST/MontyBrain.app"
cp "$MCP" "$DEST/bin/montybrain-mcp"
cp "$ROOT/install.sh" "$ROOT/LICENSE" "$DEST/"
cp "$ROOT/docs/release-install.md" "$DEST/INSTALL.md"
cp -R "$ROOT/skills" "$DEST/skills"
ditto -c -k --sequesterRsrc --keepParent "$DEST" "$ROOT/release/$NAME.zip"
(cd "$ROOT/release" && shasum -a 256 "$NAME.zip" > "$NAME.zip.sha256")
echo "Created release/$NAME.zip and its SHA-256 checksum."
