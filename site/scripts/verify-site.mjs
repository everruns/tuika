import { access, readFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const pages = [
  "",
  "charts",
  "components",
  "components/motion",
  "components/text",
  "components/markdown-code",
  "components/layout",
  "components/interactive",
  "components/notifications-console",
  "components/banners-codes-pixels",
  "features",
  "getting-started",
  "keymap",
  "layout",
  "markdown",
  "showcases",
  "styling",
  "themes",
];

const failures = [];

async function read(path) {
  try {
    return await readFile(resolve(root, "dist", path), "utf8");
  } catch {
    failures.push(`missing dist/${path}`);
    return "";
  }
}

for (const slug of pages) {
  const directory = slug ? `${slug}/` : "";
  const html = await read(`${directory}index.html`);
  const markdown = await read(`${directory}index.md`);
  const canonical = `https://tuika.dev/${directory}`;

  for (const marker of [
    "<title>",
    'name="description"',
    `rel="canonical" href="${canonical}"`,
    'property="og:image"',
    'type="text/markdown"',
    'type="application/ld+json"',
  ]) {
    if (!html.includes(marker)) failures.push(`${directory || "/"} lacks ${marker}`);
  }
  if (!markdown.startsWith("---\n")) failures.push(`${directory || "/"} lacks Markdown frontmatter`);
}

const robots = await read("robots.txt");
for (const marker of ["Content-Signal: ai-train=yes, search=yes, ai-input=yes", "Sitemap: https://tuika.dev/sitemap.xml"]) {
  if (!robots.includes(marker)) failures.push(`robots.txt lacks ${marker}`);
}

for (const path of ["llms.txt", "llms-full.txt", "social-card.png", "sitemap.xml"]) {
  await read(path);
}

const sitemap = await read("sitemap.xml");
const sitemapUrls = [...sitemap.matchAll(/<loc>[^<]+<\/loc>/g)].length;
const sitemapDates = [...sitemap.matchAll(/<lastmod>\d{4}-\d{2}-\d{2}<\/lastmod>/g)].length;
if (sitemapUrls === 0 || sitemapDates !== sitemapUrls) {
  failures.push(`sitemap.xml has ${sitemapUrls} URLs but ${sitemapDates} valid lastmod dates`);
}

for (const path of ["sitemap-index.xml", "sitemap-0.xml"]) {
  try {
    await access(resolve(root, "dist", path));
    failures.push(`dist/${path} should not be published`);
  } catch {
    // Expected: the build publishes one canonical sitemap.
  }
}

const distFiles = await Promise.all(pages.map((slug) => read(`${slug ? `${slug}/` : ""}index.html`)));
const combined = distFiles.join("\n");
for (const marker of ["example.com", "CHANGE_ME", "sitemap-index.xml", 'href="docs/', 'src="demos/']) {
  if (combined.includes(marker)) failures.push(`built pages contain ${marker}`);
}

const componentIndex = await read("components/index.html");
for (const match of componentIndex.matchAll(/href="\/components\/([a-z-]+)\/#([a-z0-9-]+)"/g)) {
  const [, family, anchor] = match;
  const page = await read(`components/${family}/index.html`);
  if (!page.includes(`id="${anchor}"`)) {
    failures.push(`component index points to missing /components/${family}/#${anchor}`);
  }
}

if (failures.length) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exitCode = 1;
} else {
  console.log(`Verified ${pages.length} HTML pages, Markdown twins, discovery files, and SEO metadata.`);
}
