#!/usr/bin/env bash
# Build the desktop app from this checkout and replace /Applications/MontyBrain.app
# with it. Keeps the previous app as a backup. Does not touch the vault or the MCP server.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_ID="pl.montybrain.desktop"
BUILT="$ROOT/target/release/bundle/macos/MontyBrain.app"
DEST="/Applications/MontyBrain.app"
BACKUPS="$ROOT/target/local-deploy-backups"
KEEP=3
OPEN=1
BUILD=1

for arg in "$@"; do
  case "$arg" in
    --no-open) OPEN=0 ;;
    --no-build) BUILD=0 ;;
    *) echo "Usage: scripts/local-deploy.sh [--no-build] [--no-open]" >&2; exit 2 ;;
  esac
done

export PATH="$HOME/.cargo/bin:$PATH"
cd "$ROOT"

if [ "$BUILD" = 1 ]; then
  pnpm test
  pnpm tauri build --bundles app
fi
[ -x "$BUILT/Contents/MacOS/montybrain-desktop" ] || { echo "No built app at $BUILT" >&2; exit 1; }

running() { pgrep -f "$DEST/Contents/MacOS/" >/dev/null; }
if running; then
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 15); do running || break; sleep 1; done
  running && { echo "MontyBrain is still running; close it and run again." >&2; exit 1; }
fi

if [ -d "$DEST" ]; then
  mkdir -p "$BACKUPS"
  mv "$DEST" "$BACKUPS/MontyBrain-$(date +%Y%m%d-%H%M%S).app"
  # Keep only the newest $KEEP backups.
  ls -1dt "$BACKUPS"/MontyBrain-*.app | tail -n +$((KEEP + 1)) | while read -r old; do rm -rf "$old"; done
fi
ditto "$BUILT" "$DEST"

a="$(shasum -a 256 "$BUILT/Contents/MacOS/montybrain-desktop" | cut -d' ' -f1)"
b="$(shasum -a 256 "$DEST/Contents/MacOS/montybrain-desktop" | cut -d' ' -f1)"
[ "$a" = "$b" ] || { echo "Installed binary differs from the build." >&2; exit 1; }

[ "$OPEN" = 1 ] && open "$DEST"
echo "Installed $DEST (sha256 $a)."
echo "Previous versions: $BACKUPS"
