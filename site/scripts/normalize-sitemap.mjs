import { execFile as execFileCallback } from "node:child_process";
import { readdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { promisify } from "node:util";
import { parseChangelog, releaseSlug } from "../src/lib/changelog.js";

const dist = resolve(import.meta.dirname, "../dist");
const repo = resolve(import.meta.dirname, "../..");
const execFile = promisify(execFileCallback);
const releaseDates = new Map(
  parseChangelog(await readFile(resolve(repo, "CHANGELOG.md"), "utf8"))
    .filter(({ date }) => date)
    .map(({ version, date }) => [`/${releaseSlug(version)}/`, date]),
);

await rename(resolve(dist, "sitemap-0.xml"), resolve(dist, "sitemap.xml"));
await rm(resolve(dist, "sitemap-index.xml"));

function sourceFor(location) {
  const pathname = new URL(location).pathname;
  if (pathname === "/") return "site/src/pages/index.astro";
  if (pathname === "/releases/" || pathname.startsWith("/releases/")) return "CHANGELOG.md";
  return `docs/${pathname.replace(/^\//, "").replace(/\/$/, "")}.md`;
}

async function lastModified(source) {
  const { stdout: status } = await execFile("git", ["status", "--porcelain", "--", source], {
    cwd: repo,
  });
  if (!status.trim()) {
    const { stdout: committed } = await execFile(
      "git",
      ["log", "-1", "--format=%cI", "--", source],
      { cwd: repo },
    );
    if (committed.trim()) return committed.trim().slice(0, 10);
  }

  return (await stat(resolve(repo, source))).mtime.toISOString().slice(0, 10);
}

const sitemapPath = resolve(dist, "sitemap.xml");
let sitemap = await readFile(sitemapPath, "utf8");
for (const match of [...sitemap.matchAll(/<url><loc>([^<]+)<\/loc><\/url>/g)]) {
  // A release page changes when its release does, not on every changelog edit.
  const modified =
    releaseDates.get(new URL(match[1]).pathname) ?? (await lastModified(sourceFor(match[1])));
  sitemap = sitemap.replace(
    match[0],
    `<url><loc>${match[1]}</loc><lastmod>${modified}</lastmod></url>`,
  );
}
await writeFile(sitemapPath, sitemap);

async function rewriteSitemapLinks(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      await rewriteSitemapLinks(path);
    } else if (entry.name.endsWith(".html")) {
      const source = await readFile(path, "utf8");
      await writeFile(path, source.replaceAll("/sitemap-index.xml", "/sitemap.xml"));
    }
  }
}

await rewriteSitemapLinks(dist);
