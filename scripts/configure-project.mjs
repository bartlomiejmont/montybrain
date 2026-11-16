#!/usr/bin/env node
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const configPath = new URL("site/project.json", root);
const config = JSON.parse(await readFile(configPath, "utf8"));
const repository = process.argv[2] ?? config.repository;
const downloadsReady = process.argv.includes("--downloads-ready") || (config.repository === repository && config.downloadsReady === true);
if (!repository || !/^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\/[A-Za-z0-9_.-]+$/.test(repository)
    || /\/(?:\.|\.\.)$/.test(repository)) {
  console.error("Usage: node scripts/configure-project.mjs OWNER/REPO");
  process.exit(1);
}
const repoUrl = `https://github.com/${repository}`;
const htmlPath = new URL("site/index.html", root);
let html = await readFile(htmlPath, "utf8");
html = html.replace(/<a data-github-path="([^"]*)" href="[^"]*"(?: hidden)?/g,
  (_, path) => `<a data-github-path="${path}" href="${repoUrl}${path}"${path.startsWith("/releases/latest/download/") && !downloadsReady ? " hidden" : ""}`);
html = html.replace(/data-repository-pending(?: hidden)?/g, "data-repository-pending hidden");
html = html.replace(/data-repository-ready(?: hidden)?/g, "data-repository-ready");
html = html.replace(/data-downloads-pending(?: hidden)?/g, `data-downloads-pending${downloadsReady ? " hidden" : ""}`);
html = html.replace(/data-downloads-ready(?: hidden)?/g, `data-downloads-ready${downloadsReady ? "" : " hidden"}`);
html = html.replace(/git clone --depth 1 \S+ montybrain/g,
  `git clone --depth 1 ${repoUrl}.git montybrain`);
const readmePath = new URL("README.md", root);
let readme = await readFile(readmePath, "utf8");
readme = readme.replace(/git clone --depth 1 \S+ montybrain/g,
  `git clone --depth 1 ${repoUrl}.git montybrain`);
readme = readme.replace(/The clone URL below is a placeholder[^\n]*/,
  `Source repository: [${repository}](${repoUrl}).`);
readme = readme.replace(/Source repository: \[[^\]]+\]\([^\n]+\)\./,
  `Source repository: [${repository}](${repoUrl}).`);
readme = readme.replace(/The public repository address and first downloadable release are still being prepared\. Until they are published, build from a local copy of the source as described below\./,
  `[Browse GitHub Releases](${repoUrl}/releases) for published builds. If no release is available yet, build from source as described below.`);
readme = readme.replace(/\[Browse GitHub Releases\]\([^)]*\)/,
  `[Browse GitHub Releases](${repoUrl}/releases)`);
await writeFile(htmlPath, html);
await writeFile(readmePath, readme);
await writeFile(configPath, `${JSON.stringify({ ...config, repository, downloadsReady }, null, 2)}\n`);
console.log(`Configured ${repository} in ${fileURLToPath(root)}. No remote, DNS, or GitHub settings were changed.`);
