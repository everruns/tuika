import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { GUIDE_SLUGS } from "../src/lib/routes.js";
import { parseChangelog, releaseHighlights, releaseSlug } from "../src/lib/changelog.js";

const siteRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(siteRoot, "..");
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

// Byte-level escape-sequence diagrams (```text blocks that start with `ESC`)
// are one long line in the source, which a code block can only scroll. On the
// site they render as a wrapping sequence: control bytes as chips, the command
// number emphasised, `<placeholders>` set apart, and a break opportunity after
// every separator. The text content stays the literal sequence, so copying it
// still yields what the guide documents. GitHub keeps the plain fence.
const CONTROL_TOKENS = new Set(["ESC", "BEL", "ST", "CSI", "OSC", "DCS", "APC"]);

function escapeHtml(text) {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

function escapeParams(token) {
  if (/^https?:\/\//.test(token)) return escapeHtml(token);
  return token
    .split(/(<[^<>\s]+>|[;:,=])/)
    .filter(Boolean)
    .map((part) => {
      if (/^<[^<>\s]+>$/.test(part)) {
        return `<span class="nb-esc-var">${escapeHtml(part)}</span>`;
      }
      if (/^[;:,]$/.test(part)) return `<span class="nb-esc-sep">${part}</span><wbr>`;
      if (part === "=") return `<span class="nb-esc-sep">=</span>`;
      return escapeHtml(part);
    })
    .join("");
}

function escapeLine(line) {
  const tokens = line.trim().split(/ +/);
  const out = [];
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    if (CONTROL_TOKENS.has(token)) {
      // `ESC ]`, `ESC _`, `ESC P`, `ESC \` name one control function together.
      const next = tokens[i + 1];
      const joined = token === "ESC" && next && next.length === 1 && !/\w/.test(next);
      const label = joined ? `${token} ${next}` : token;
      if (joined) i += 1;
      out.push(`<span class="nb-esc-ctl">${escapeHtml(label)}</span>`);
      // The command number right after an introducer (`ESC ] 7501`).
      if (joined && /^\d+$/.test(tokens[i + 1] ?? "")) {
        i += 1;
        out.push(`<span class="nb-esc-cmd">${tokens[i]}</span>`);
      }
    } else {
      out.push(`<span class="nb-esc-arg">${escapeParams(token)}</span>`);
    }
  }
  return out.join(" ");
}

function escapeDiagrams(markdown) {
  return markdown.replace(/^```text\n(ESC [^`]*?)\n```$/gm, (_match, body) => {
    const lines = body.split("\n").map((line) => `<span class="nb-esc-line">${escapeLine(line)}</span>`);
    return `<pre class="nb-esc" aria-label="Escape sequence"><code>${lines.join("\n")}</code></pre>`;
  });
}

function forSite(markdown) {
  return sizeImages(rewriteLinks(markdown));
}

function rewriteLinks(markdown) {
  return escapeDiagrams(markdown)
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

// Image dimensions. The guides embed their recordings as `<img width="880">`
// or `![alt](src)` with no height, so the browser only learns how tall a
// recording is once it has downloaded it, and everything below jumps down as
// each one arrives — on every page load, which on this site is every sidebar
// click. Stamping the intrinsic aspect ratio as `width`/`height` lets it
// reserve the box up front (Tailwind's preflight keeps `height: auto`, so
// the box still scales with the column). Only files that are in this
// repository are measured; anything else is left as written.
const RAW_PREFIX = "https://raw.githubusercontent.com/everruns/tuika/";
const imageSizes = new Map();

// The bytes of an image this repository holds: `/docs-assets/` is `docs/`,
// a `main` raw URL is the working tree, and a tag-pinned raw URL (release
// notes) is read from that tag when the checkout has it.
function localImage(src) {
  const clean = decodeURIComponent(src.split(/[?#]/)[0]);
  if (clean.startsWith("/docs-assets/")) {
    return readInside(resolve(repoRoot, "docs"), clean.slice("/docs-assets/".length));
  }
  if (!clean.startsWith(RAW_PREFIX)) return null;
  const [ref, ...rest] = clean.slice(RAW_PREFIX.length).split("/");
  const path = rest.join("/");
  if (ref === "main") return readInside(repoRoot, path);
  if (!/^v[\w.-]+$/.test(ref) || path.split("/").includes("..")) return null;
  return execFileSync("git", ["show", `${ref}:${path}`], {
    cwd: repoRoot,
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "ignore"],
  });
}

function readInside(dir, relative) {
  const path = resolve(dir, relative);
  return path.startsWith(`${dir}/`) ? readFileSync(path) : null;
}

function svgLength(value) {
  const match = /^\s*([\d.]+)(px)?\s*$/.exec(value ?? "");
  return match ? Number(match[1]) : null;
}

function imageSize(bytes) {
  // PNG: width and height are the first two fields of the IHDR chunk.
  if (bytes.length >= 24 && bytes.toString("latin1", 1, 4) === "PNG") {
    return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
  }
  // GIF: the logical screen descriptor follows the 6-byte signature.
  if (bytes.length >= 10 && bytes.toString("latin1", 0, 3) === "GIF") {
    return { width: bytes.readUInt16LE(6), height: bytes.readUInt16LE(8) };
  }
  const head = bytes.toString("utf8", 0, Math.min(bytes.length, 4096));
  const svg = /<svg\b[^>]*>/.exec(head)?.[0];
  if (svg) {
    const attr = (name) => new RegExp(`\\s${name}="([^"]*)"`).exec(svg)?.[1];
    const width = svgLength(attr("width"));
    const height = svgLength(attr("height"));
    if (width && height) return { width, height };
    const box = attr("viewBox")?.trim().split(/[\s,]+/).map(Number);
    if (box?.length === 4 && box[2] > 0 && box[3] > 0) return { width: box[2], height: box[3] };
  }
  return null;
}

function sizeOf(src) {
  if (!imageSizes.has(src)) {
    let size = null;
    try {
      const bytes = localImage(src);
      size = bytes && imageSize(bytes);
    } catch {
      // A missing asset (or a tag this checkout lacks) keeps the image as
      // written: reserving space is an improvement, not a requirement.
    }
    imageSizes.set(src, size);
  }
  return imageSizes.get(src);
}

function escapeAttr(text) {
  return escapeHtml(text).replaceAll('"', "&quot;");
}

function sizeHtmlImage(tag) {
  if (/\sheight=/.test(tag)) return tag;
  const src = /\ssrc="([^"]+)"/.exec(tag)?.[1];
  const size = src && sizeOf(src);
  if (!size) return tag;
  const declared = Number(/\swidth="(\d+)"/.exec(tag)?.[1]);
  const width = declared || size.width;
  const height = Math.round((width * size.height) / size.width);
  const attrs = declared ? ` height="${height}"` : ` width="${width}" height="${height}"`;
  return tag.replace(/\s*\/?>$/, (end) => `${attrs}${end}`);
}

function sizeImages(markdown) {
  let inFence = false;
  return markdown
    .split("\n")
    .map((line) => {
      if (/^\s*(```|~~~)/.test(line)) inFence = !inFence;
      if (inFence) return line;
      // Inline code spans are left alone; only the text between them is sized.
      return line
        .split(/(`+[^`]*`+)/)
        .map((part, index) => {
          if (index % 2 === 1) return part;
          return part
            .replace(/<img\b[^>]*>/g, sizeHtmlImage)
            .replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (match, alt, src) => {
              const size = sizeOf(src);
              if (!size) return match;
              return `<img src="${escapeAttr(src)}" alt="${escapeAttr(alt)}" width="${size.width}" height="${size.height}">`;
            });
        })
        .join("");
    })
    .join("\n");
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
