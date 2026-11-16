# Contributing to MontyBrain

Thanks for helping improve MontyBrain. The project is early, with a macOS desktop app, a shared Rust core, and a standalone MCP server. The app defaults to English and also supports Polish (Settings → App language). UI messages live in `src/i18n`; add translations when introducing new labels.

## Report a bug or propose a change

Open an issue in [bartlomiejmont/montybrain](https://github.com/bartlomiejmont/montybrain/issues). For a bug, include:

- MontyBrain version, macOS version, and whether your Mac uses Apple Silicon or Intel.
- What you did, what you expected, and what happened.
- A minimal, anonymized Markdown example if the problem concerns vault parsing or writing.
- Relevant error output, with personal data, repository paths, and credentials removed.

Please discuss larger features before implementing them so we can agree on the behavior and scope. Small fixes and documentation improvements can go straight to a pull request.

## Work locally

Follow the [README setup](README.md#build-from-source). Use a separate demo vault while developing; keep real personal data out of the source repository. To launch the app with a throwaway vault:

```sh
MONTYBRAIN_DIR="$(mktemp -d)" pnpm tauri dev
```

The browser frontend (`pnpm dev`) uses mock data. Changes to real vault behavior should be checked in the desktop app or through the MCP server.

## Before opening a pull request

Keep the change focused, explain the user-facing behavior, and add a regression test for parser, writer, or state-management fixes. Preserve the documented Markdown format and existing user content. Files in `crates/core/tests/fixtures/` can intentionally have unusual line endings; keep them byte-exact.

Run checks relevant to your change:

```sh
pnpm test
pnpm build
cargo test --workspace --locked
```

For landing or release configuration changes, also run:

```sh
node --check site/app.js
node --check scripts/configure-project.mjs
bash -n scripts/package-release.sh
```

Include the checks you ran and any limitations in your pull request. For interface changes, include screenshots where helpful. Do not commit build outputs, release archives, credentials, or personal vaults.

Contributions are made under the project's [MIT license](LICENSE).
