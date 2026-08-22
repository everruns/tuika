import { GUIDE_SLUGS } from "./routes.js";

const guideSlugs = new Set(GUIDE_SLUGS);

export function normalizeMarkdownLinks(markdown: string): string {
  return markdown
    .replace(/^# .+\n+/, "")
    .replace(/\]\((?:\.\/)?([a-z-]+)\.md(#[^)]+)?\)/g, (match, slug, hash = "") =>
      guideSlugs.has(slug) ? `](/${slug}/${hash})` : match,
    )
    .replace(/src="(?!https?:|\/)([^"#]+)"/g, 'src="/docs-assets/$1"')
    .replace(/!\[([^\]]*)\]\((?!https?:|\/)([^)]+)\)/g, "![$1](/docs-assets/$2)")
    .replace(
      /(?:\.\.\/)+(crates|examples)\/([^\s)\"]+)/g,
      "https://github.com/everruns/tuika/tree/main/$1/$2",
    );
}
