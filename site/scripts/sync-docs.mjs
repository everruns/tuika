import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { GUIDE_SLUGS } from "../src/lib/routes.js";

const siteRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sourceDir = resolve(siteRoot, "../docs");
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
