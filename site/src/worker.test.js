import test from "node:test";
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import worker, {
  canonicalPagePath,
  htmlAssetPath,
  markdownAssetPath,
  prefersMarkdown,
} from "./worker.js";
import { COMPONENT_SLUGS, GUIDE_SLUGS } from "./lib/routes.js";

test("route inventory matches every public guide", async () => {
  const docs = new URL("../../docs/", import.meta.url);
  const guideSlugs = (await readdir(docs, { withFileTypes: true }))
    .filter((entry) => entry.isFile() && entry.name.endsWith(".md"))
    .map((entry) => entry.name.slice(0, -3))
    .sort();
  const componentSlugs = (await readdir(new URL("components/", docs), {
    withFileTypes: true,
  }))
    .filter((entry) => entry.isFile() && entry.name.endsWith(".md"))
    .map((entry) => entry.name.slice(0, -3))
    .sort();

  assert.deepEqual([...GUIDE_SLUGS].sort(), guideSlugs);
  assert.deepEqual([...COMPONENT_SLUGS].sort(), componentSlugs);
});

test("deployment sends every guide route through the worker", async () => {
  const config = JSON.parse(
    await readFile(new URL("../wrangler.jsonc", import.meta.url), "utf8"),
  );

  assert.deepEqual(config.assets.run_worker_first, [
    "/",
    "/index.html",
    ...[...GUIDE_SLUGS, "releases"].sort().map((slug) => `/${slug}*`),
  ]);
});

test("maps public pages to their markdown twins", () => {
  assert.equal(markdownAssetPath("/"), "/index.md");
  assert.equal(markdownAssetPath("/components/"), "/components/index.md");
  assert.equal(markdownAssetPath("/getting-started/"), "/getting-started/index.md");
  assert.equal(markdownAssetPath("/routing/"), "/routing/index.md");
  assert.equal(
    markdownAssetPath("/components/interactive/"),
    "/components/interactive/index.md",
  );
  assert.equal(markdownAssetPath("/not-a-guide/"), null);
  assert.equal(markdownAssetPath("/llms.txt"), null);
});

test("maps canonical pages to exact static assets", () => {
  assert.equal(canonicalPagePath("/"), "/");
  assert.equal(canonicalPagePath("/index.html"), "/");
  assert.equal(canonicalPagePath("/components"), "/components/");
  assert.equal(canonicalPagePath("/getting-started"), "/getting-started/");
  assert.equal(canonicalPagePath("/routing"), "/routing/");
  assert.equal(canonicalPagePath("/components/index.html"), "/components/");
  assert.equal(canonicalPagePath("/components/interactive"), "/components/interactive/");
  assert.equal(
    canonicalPagePath("/components/interactive/index.html"),
    "/components/interactive/",
  );
  assert.equal(canonicalPagePath("/not-a-guide"), null);
  assert.equal(canonicalPagePath("/releases"), "/releases/");
  assert.equal(canonicalPagePath("/releases/0.13.0"), "/releases/0.13.0/");
  assert.equal(canonicalPagePath("/releases/0.13.0/index.html"), "/releases/0.13.0/");
  assert.equal(canonicalPagePath("/releases/latest"), null);
  assert.equal(markdownAssetPath("/releases/0.13.0/"), "/releases/0.13.0/index.md");
  assert.equal(htmlAssetPath("/"), "/index.html");
  assert.equal(htmlAssetPath("/components/"), "/components/index.html");
  assert.equal(
    htmlAssetPath("/components/interactive/"),
    "/components/interactive/index.html",
  );
});

test("negotiates markdown only when explicitly preferred", () => {
  assert.equal(prefersMarkdown("text/markdown"), true);
  assert.equal(prefersMarkdown("text/markdown, text/html;q=0.8"), true);
  assert.equal(prefersMarkdown("text/html, text/markdown;q=0.5"), false);
  assert.equal(prefersMarkdown("*/*"), false);
  assert.equal(prefersMarkdown(""), false);
});

test("serves HTML by default and Markdown when requested", async () => {
  const env = {
    ASSETS: {
      fetch(request) {
        const markdown = new URL(request.url).pathname.endsWith(".md");
        return Promise.resolve(new Response(markdown ? "# Components" : "<!doctype html>", {
          headers: { "Content-Type": markdown ? "text/markdown" : "text/html; charset=utf-8" },
        }));
      },
    },
  };

  const html = await worker.fetch(new Request("https://tuika.dev/components/", {
    headers: { Accept: "text/html" },
  }), env);
  assert.match(html.headers.get("Content-Type"), /^text\/html/);
  assert.equal(await html.text(), "<!doctype html>");

  const markdown = await worker.fetch(new Request("https://tuika.dev/components/", {
    headers: { Accept: "text/markdown" },
  }), env);
  assert.equal(markdown.headers.get("Content-Type"), "text/markdown; charset=utf-8");
  assert.equal(await markdown.text(), "# Components");
});

test("redirects non-canonical page URLs", async () => {
  const response = await worker.fetch(new Request("https://tuika.dev/components"), {});
  assert.equal(response.status, 308);
  assert.equal(response.headers.get("Location"), "https://tuika.dev/components/");
});

test("parses every changelog release into a page", async () => {
  const { parseChangelog, releaseHighlights } = await import("./lib/changelog.js");
  const releases = parseChangelog(
    await readFile(new URL("../../CHANGELOG.md", import.meta.url), "utf8"),
  );
  assert.ok(releases.length > 0);
  for (const { version, date, body } of releases) {
    assert.match(version, /^\d+\.\d+\.\d+/);
    assert.match(date ?? "", /^\d{4}-\d{2}-\d{2}$/);
    assert.ok(body.length > 0, `${version} has notes`);
    assert.ok(!/^\[[^\]]+\]: /m.test(body), `${version} drops link definitions`);
  }

  const sample = parseChangelog(
    [
      "# Changelog",
      "## [Unreleased]",
      "- pending",
      "## [1.2.0] - 2026-01-02",
      "### Highlights",
      "**Fast** — quicker.",
      "```md",
      "## not a release",
      "```",
      "[1.2.0]: https://example.invalid",
    ].join("\n"),
  );
  assert.deepEqual(sample.map(({ version }) => version), ["1.2.0"]);
  assert.match(sample[0].body, /## not a release/);
  assert.deepEqual(releaseHighlights(sample[0].body), ["Fast"]);
});
