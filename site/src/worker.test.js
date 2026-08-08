import test from "node:test";
import assert from "node:assert/strict";
import worker, {
  canonicalPagePath,
  htmlAssetPath,
  markdownAssetPath,
  prefersMarkdown,
} from "./worker.js";

test("maps public pages to their markdown twins", () => {
  assert.equal(markdownAssetPath("/"), "/index.md");
  assert.equal(markdownAssetPath("/components/"), "/components/index.md");
  assert.equal(markdownAssetPath("/getting-started/"), "/getting-started/index.md");
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
  assert.equal(canonicalPagePath("/components/index.html"), "/components/");
  assert.equal(canonicalPagePath("/components/interactive"), "/components/interactive/");
  assert.equal(
    canonicalPagePath("/components/interactive/index.html"),
    "/components/interactive/",
  );
  assert.equal(canonicalPagePath("/not-a-guide"), null);
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
