import { access, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { PAGE_SLUGS } from "../src/lib/routes.js";

const root = resolve(import.meta.dirname, "..");
const pages = PAGE_SLUGS;

const failures = [];

async function read(path) {
  try {
    return await readFile(resolve(root, "dist", path), "utf8");
  } catch {
    failures.push(`missing dist/${path}`);
    return "";
  }
}

async function verifyPng(path) {
  try {
    const image = await readFile(resolve(root, "dist", path));
    const pngSignature = "89504e470d0a1a0a";
    if (
      image.length < 24 ||
      image.subarray(0, 8).toString("hex") !== pngSignature ||
      image.readUInt32BE(16) !== 1200 ||
      image.readUInt32BE(20) !== 630
    ) {
      failures.push(`dist/${path} must be a 1200x630 PNG`);
    }
  } catch {
    failures.push(`missing dist/${path}`);
  }
}

for (const slug of pages) {
  const directory = slug ? `${slug}/` : "";
  const html = await read(`${directory}index.html`);
  const markdown = await read(`${directory}index.md`);
  const canonical = `https://tuika.dev/${directory}`;
  const socialImage = slug
    ? `https://tuika.dev/og/${slug}.png`
    : "https://tuika.dev/social-card.png";

  for (const marker of [
    "<title>",
    'name="description"',
    `rel="canonical" href="${canonical}"`,
    `property="og:image" content="${socialImage}"`,
    'property="og:image:width" content="1200"',
    'property="og:image:height" content="630"',
    'name="twitter:card" content="summary_large_image"',
    `name="twitter:image" content="${socialImage}"`,
    'name="twitter:image:alt" content="tuika link preview with terminal-grid artwork"',
    'type="text/markdown"',
    'type="application/ld+json"',
  ]) {
    if (!html.includes(marker)) failures.push(`${directory || "/"} lacks ${marker}`);
  }
  await verifyPng(slug ? `og/${slug}.png` : "social-card.png");
  if (!markdown.startsWith("---\n")) failures.push(`${directory || "/"} lacks Markdown frontmatter`);
}

await verifyPng("og.png");

const homepageCard = await readFile(resolve(root, "dist", "social-card.png"));
const fallbackCard = await readFile(resolve(root, "dist", "og.png"));
if (!homepageCard.equals(fallbackCard)) {
  failures.push("dist/og.png must match the pre-rendered homepage social card");
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
