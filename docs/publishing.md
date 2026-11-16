# Publishing MontyBrain

The domain is **montybrain.com**. The public source repository is [bartlomiejmont/montybrain](https://github.com/bartlomiejmont/montybrain). Everything here is preparation: these files do not create a repository, change DNS, upload an app, or publish a release on their own.

## 1. Configure the public source repository

The Git remote is `https://github.com/bartlomiejmont/montybrain.git`. To regenerate its links, run this from the source checkout:

```sh
node scripts/configure-project.mjs bartlomiejmont/montybrain
```

The command validates `OWNER/REPO`, saves it in `site/project.json`, fills the README clone URL, and enables the landing's GitHub links. App download buttons stay hidden until explicitly enabled after the first stable release. It is safe to run again with another repository address. Until configured, the landing shows a clear first-release message instead of links to a guessed repository.

Review the resulting diff and commit the configuration. Set the Git remote and push when ready. The repository's public contents should contain source and documentation; do not add your personal vault or build outputs. Generated ZIPs go to `release/` (ignored by Git).

## 2. Prepare the first release

The workflow `.github/workflows/release.yml` is **manual only**. It does not build on every push or pull request. It skips macOS builds in private repositories, where runner minutes could be charged.

1. Set the same stable version in `package.json`, `src-tauri/tauri.conf.json`, and the workspace `Cargo.toml`. Update `Cargo.lock` for changed workspace versions.
2. Push the intended release commit. In Actions, run **Prepare macOS release** from that commit's branch.
3. The workflow tests and builds native Apple Silicon and Intel packages on standard macOS runners. Each ZIP includes the app, matching MCP binary, four skills, installer, MIT license, and `INSTALL.md`. SHA-256 checksum files are also attached.
4. Both builds must succeed before a **draft** GitHub Release is created. Temporary workflow artifacts expire after one day; release assets remain with the release.
5. Review the draft notes and smoke-test the downloads on the matching hardware. The versioned release must not already exist; to prepare another release, bump the version.
6. Publish the draft as a stable release and ensure it is marked **latest**. Then enable app download buttons with `node scripts/configure-project.mjs bartlomiejmont/montybrain --downloads-ready` and commit the resulting configuration. The download URLs below resolve only after a stable release exists.

```text
https://github.com/bartlomiejmont/montybrain/releases/latest/download/MontyBrain-macos-arm64.zip
https://github.com/bartlomiejmont/montybrain/releases/latest/download/MontyBrain-macos-x64.zip
```

The workflow packages the app without Apple Developer signing or notarization. Both the landing and installation guide say so. If signing and notarization are added later, update the build configuration and remove that notice only after the signed packages have been verified.

To package an existing native build locally, after `pnpm tauri build --bundles app` and `cargo build --release --locked -p montybrain-mcp`:

```sh
scripts/package-release.sh arm64  # use x64 on an Intel build machine
```

The script checks the architecture of both binaries to prevent mislabeled packages. It does not upload anything.

## 3. Publish the landing

The entire landing is the static `site/` directory. It needs no server process, framework build, or dependency installation. You can upload that folder to an existing static host. Download links point directly to GitHub Releases, without proxying packages through that host.

For GitHub Pages:

1. In the repository's **Settings → Pages**, select **GitHub Actions** as the publishing source.
2. Set the custom domain to `montybrain.com` and configure its DNS using [GitHub's custom-domain instructions](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site). The intended domain is also recorded in `site/CNAME`.
3. After publishing the first stable release, manually run **Deploy landing**. The workflow configures repository links from `GITHUB_REPOSITORY` before uploading only `site/`.
4. Enable **Enforce HTTPS** when the certificate is ready. Check both architecture downloads, the GitHub links, and the clone command on the live domain.

The landing can be published earlier to announce the project and provide source links. Leave `downloadsReady` set to `false` in `site/project.json` to retain the “getting ready” message for app packages. The Pages workflow respects that setting.

## Transfer and costs

- **Downloads:** GitHub Releases distributes application ZIPs directly. GitHub documents no total release-size or release-download bandwidth limit; individual assets must be below 2 GiB. See [About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
- **Source:** `git clone --depth 1` skips old commit history. Current screenshots still download. `git pull --ff-only` fetches updates without downloading a new ZIP of the whole source every time.
- **Landing:** GitHub Pages has a soft bandwidth limit of 100 GB/month. Application downloads bypass it. See [GitHub Pages limits](https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits).
- **Builds:** Standard GitHub-hosted runners are free for public repositories. Private repositories have different quotas and billing, so the release workflow requires a public repository. See [GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
- **Storage:** Binaries stay out of Git and Git LFS. Workflow artifacts have short retention. The landing lazy-loads screenshots below the hero and uses no external fonts or JavaScript dependencies.

The domain registration/renewal and any selected third-party hosting or agent provider are separate costs. Recheck provider limits before launch; they can change.
