# Install MontyBrain on macOS

This package contains the desktop app, its matching MCP server, and optional agent skills.
Use the Apple Silicon package on M-series Macs or the Intel package on Intel Macs.

## Desktop app

Move `MontyBrain.app` to your Applications folder and open it. The app creates its Markdown vault at `~/MontyBrain` on first launch (or reuses an existing legacy vault).

Initial releases are not Apple Developer signed or notarized. If macOS blocks the app, follow Apple's instructions:
https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac

You do not need Rust, Node.js, an agent, or a MontyBrain account to use the desktop app.

## Optional: MCP server and agent skills

Install and authenticate Claude Code or Codex separately. In Terminal, change into this unzipped package directory and run:

```sh
mkdir -p "$HOME/.local/bin"
cp bin/montybrain-mcp "$HOME/.local/bin/montybrain-mcp.new"
mv -f "$HOME/.local/bin/montybrain-mcp.new" "$HOME/.local/bin/montybrain-mcp"
./install.sh --no-build
```

The first three commands install the prebuilt server. `--no-build` tells the installer to use that binary, register it with installed CLIs, and install the bundled skills without compiling anything. Start a new agent session afterward.

To register with just one agent, use `./install.sh --no-build --only claude` or `./install.sh --no-build --only codex`. To use another vault, append `--vault /absolute/path/to/vault` and select the same folder in the desktop app's settings.

Agent calls can send task content and repository context to the selected provider. Provider access and usage costs are separate from MontyBrain.

## Updates and removal

Download a newer package for the same architecture and replace the app. Repeat the MCP installation commands to update its binary and skills. Your vault is stored separately and is not replaced.

`./install.sh --uninstall` removes the MCP registrations and installer-managed skills; it leaves your vault and the MCP binary in place. You can remove the app from Applications separately.

## Check your download

Download the `.zip.sha256` file alongside the ZIP. In the directory containing both files, run (using the matching filename for Intel):

```sh
shasum -a 256 -c MontyBrain-macos-arm64.zip.sha256
```

A matching checksum detects a corrupted download; it is not a developer signature or notarization.
