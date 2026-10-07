import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { GUIDE_SLUGS } from "../src/lib/routes.js";
import { parseChangelog, releaseHighlights, releaseSlug } from "../src/lib/changelog.js";

const siteRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sourceDir = resolve(siteRoot, "../docs");
const changelogPath = resolve(siteRoot, "../CHANGELOG.md");
const targetDir = resolve(siteRoot, "src/content/docs");
const componentDir = resolve(sourceDir, "components");
const guideSlugs = new Set(GUIDE_SLUGS);
const componentRoutes = new Map();

function headingAnchor(heading) {
  return heading
    .replaceAll("`", "")
    .toLowerCase()
    .replace(/[^a-z0-9 _-]/g, "")
    .replace(/ /g, "-");
}

function componentHref(hash = "") {
  if (!hash) return "/components/";
  const family = componentRoutes.get(hash.slice(1));
  return family ? `/components/${family}/${hash}` : `/components/${hash}`;
}

function forSite(markdown) {
  return markdown
    // Shiki accepts the language name, while rustdoc's comma-separated fence
    // attributes are meaningful only to rustdoc.
    .replace(/^```([^,\s]+),[^\n]*$/gm, "```$1")
    .replace(
      /src="(?:\.\.\/){1,2}(crates|examples)\/([^"]+)"/g,
      'src="https://raw.githubusercontent.com/everruns/tuika/main/$1/$2"',
    )
    .replace(/src="\.\.\/(demos\/[^"#]+)"/g, 'src="/docs-assets/$1"')
    .replace(/src="(?!https?:|\/)([^"#]+)"/g, 'src="/docs-assets/$1"')
    .replace(
      /!\[([^\]]*)\]\((?:\.\.\/){1,2}(crates|examples)\/([^)]+)\)/g,
      "![$1](https://raw.githubusercontent.com/everruns/tuika/main/$2/$3)",
    )
    .replace(/!\[([^\]]*)\]\(\.\.\/(demos\/[^)]+)\)/g, "![$1](/docs-assets/$2)")
    .replace(/!\[([^\]]*)\]\((?!https?:|\/)([^)]+)\)/g, "![$1](/docs-assets/$2)")
    .replace(/\]\((?:\.\/)?components\/([a-z-]+)\.md(#[^)]+)?\)/g, "](/components/$1/$2)")
    .replace(/\]\(\.\.\/components\.md(#[^)]+)?\)/g, (_match, hash = "") =>
      `](${componentHref(hash)})`,
    )
    .replace(/\]\((?:\.\/)?components\.md(#[^)]+)?\)/g, (_match, hash = "") =>
      `](${componentHref(hash)})`,
    )
    .replace(/href="(?:\.\/)?components\.md(#[^"]*)?"/g, (_match, hash = "") =>
      `href="${componentHref(hash)}"`,
    )
    .replace(/\]\((?:\.\.\/|\.\/)?([a-z-]+)\.md(#[^)]+)?\)/g, (match, slug, hash = "") =>
      guideSlugs.has(slug) ? `](/${slug}/${hash})` : match,
    )
    .replace(/href="(?:\.\.\/|\.\/)?([a-z-]+)\.md(#[^"]*)?"/g, (match, slug, hash = "") =>
      guideSlugs.has(slug) ? `href="/${slug}/${hash}"` : match,
    )
    .replace(
      /\]\((?:\.\.\/){1,2}(crates|examples)\/([^)]*)\)/g,
      "](https://github.com/everruns/tuika/tree/main/$1/$2)",
    )
    .replace(
      /href="(?:\.\.\/){1,2}(crates|examples)\/([^"]*)"/g,
      'href="https://github.com/everruns/tuika/tree/main/$1/$2"',
    )
    .replace(
      /\]\((?:\.\.\/){1,2}README\.md([^)]+)?\)/g,
      "](https://github.com/everruns/tuika/blob/main/README.md$1)",
    );
}

await rm(targetDir, { recursive: true, force: true });
await mkdir(targetDir, { recursive: true });

const componentEntries = (await readdir(componentDir, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".md"));
for (const entry of componentEntries) {
  const source = await readFile(resolve(componentDir, entry.name), "utf8");
  const family = entry.name.slice(0, -3);
  for (const match of source.matchAll(/^### (.+)$/gm)) {
    componentRoutes.set(headingAnchor(match[1]), family);
  }
}

for (const entry of await readdir(sourceDir, { withFileTypes: true })) {
  if (!entry.isFile() || !entry.name.endsWith(".md")) continue;
  const source = await readFile(resolve(sourceDir, entry.name), "utf8");
  await writeFile(resolve(targetDir, entry.name), forSite(source));
}

await mkdir(resolve(targetDir, "components"), { recursive: true });
for (const entry of componentEntries) {
  const source = await readFile(resolve(componentDir, entry.name), "utf8");
  await writeFile(resolve(targetDir, "components", entry.name), forSite(source));
}

// Release notes: one page per CHANGELOG.md entry plus an index, so every
// release is a static, crawlable URL with its own sitemap entry, Markdown twin,
// and social card.
function yamlString(value) {
  return JSON.stringify(value);
}

function releaseBody(body, version) {
  const tree = `https://github.com/everruns/tuika/tree/v${version}`;
  let inFence = false;
  return body
    .split("\n")
    .map((line) => {
      if (/^\s*(```|~~~)/.test(line)) inFence = !inFence;
      if (inFence) return line;
      return line
        // The page title is the release, so its `###` sections become `##`.
        .replace(/^(#{3,6}) /, (_match, hashes) => `${hashes.slice(1)} `)
        .replace(
          /\(#(\d+)\)/g,
          "([#$1](https://github.com/everruns/tuika/pull/$1))",
        )
        // Repository-relative links resolve against the release's own tag.
        .replace(/\]\(docs\/([a-z-]+)\.md(#[^)]+)?\)/g, (match, slug, hash = "") =>
          guideSlugs.has(slug) ? `](/${slug}/${hash})` : match,
        )
        .replace(
          /\]\((?:\.\/)?((?:crates|examples|src|docs)\/[^)\s]*|README\.md[^)\s]*)\)/g,
          `](${tree}/$1)`,
        );
    })
    .join("\n");
}

const releases = parseChangelog(await readFile(changelogPath, "utf8"));
const releaseDir = resolve(targetDir, "releases");
await mkdir(releaseDir, { recursive: true });

const indexLines = [
  "---",
  "title: Releases",
  `description: ${yamlString(
    `Release notes for every tuika version${releases[0] ? `, latest ${releases[0].version}` : ""}: highlights, breaking changes, additions, and fixes.`,
  )}`,
  "sidebar:",
  "  group:",
  "    label: Releases",
  "  order: 12",
  "---",
  "",
  "# Releases",
  "",
  "Release notes for every tuika version published from this repository, newest",
  "first. Each version is on [crates.io](https://crates.io/crates/tuika/versions)",
  "and has a matching [GitHub release](https://github.com/everruns/tuika/releases).",
  "Versions 0.1.0 through 0.4.0 were published from the",
  "[yolop](https://github.com/everruns/yolop) workspace before tuika moved to its",
  "own repository, so they have no notes here.",
  "",
];

for (const [index, release] of releases.entries()) {
  const { version, date, body } = release;
  const highlights = releaseHighlights(body);
  const dated = date ? ` (${date})` : "";
  const summary = highlights.length
    ? ` Highlights: ${highlights.join("; ")}.`
    : " Breaking changes, additions, and fixes.";
  const frontmatter = [
    "---",
    `title: ${yamlString(`tuika ${version}`)}`,
    // Astro's glob loader would slugify `0.13.0` to `0130`; keep the dots.
    `slug: ${yamlString(releaseSlug(version))}`,
    `description: ${yamlString(`Release notes for tuika ${version}${dated}.${summary}`)}`,
    ...(date ? [`lastUpdated: ${date}`] : []),
    "sidebar:",
    `  label: ${yamlString(version)}`,
    `  order: ${index + 1}`,
    "---",
  ];
  const links = [
    date ? `Released ${date}.` : null,
    `[crates.io](https://crates.io/crates/tuika/${version})`,
    `· [docs.rs](https://docs.rs/tuika/${version}/tuika/)`,
    `· [GitHub release](https://github.com/everruns/tuika/releases/tag/v${version})`,
  ].filter(Boolean);
  const page = [
    ...frontmatter,
    "",
    `# tuika ${version}`,
    "",
    links.join(" "),
    "",
    forSite(releaseBody(body, version)),
    "",
  ].join("\n");
  await writeFile(resolve(releaseDir, `${version}.md`), page);

  indexLines.push(`## [${version}](/releases/${version}/)`, "");
  if (date) indexLines.push(`Released ${date}.`, "");
  for (const highlight of highlights) indexLines.push(`- ${highlight}`);
  if (highlights.length) indexLines.push("");
}

await writeFile(resolve(targetDir, "releases.md"), indexLines.join("\n"));
