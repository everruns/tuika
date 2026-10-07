// Parses the repository's CHANGELOG.md into release entries for the site's
// release-notes pages. The changelog is the single source: GitHub Releases are
// cut from the same text, so the site never carries a second copy.

const RELEASE_HEADING = /^## \[(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)\](?: - (\d{4}-\d{2}-\d{2}))?\s*$/;
const LINK_DEFINITION = /^\[[^\]]+\]: \S+\s*$/;

/**
 * Splits a Keep-a-Changelog document into released versions, newest first as
 * they appear in the file. `[Unreleased]` and link-reference definitions are
 * dropped; every other line of a release section is returned verbatim.
 *
 * @param {string} markdown
 * @returns {{ version: string, date: string | null, body: string }[]}
 */
export function parseChangelog(markdown) {
  const releases = [];
  let current = null;
  let inFence = false;

  for (const line of markdown.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) inFence = !inFence;
    if (!inFence && line.startsWith("## ")) {
      if (current) releases.push(current);
      const match = line.match(RELEASE_HEADING);
      current = match ? { version: match[1], date: match[2] ?? null, lines: [] } : null;
      continue;
    }
    if (!current) continue;
    if (!inFence && LINK_DEFINITION.test(line)) continue;
    current.lines.push(line);
  }
  if (current) releases.push(current);

  return releases.map(({ version, date, lines }) => ({
    version,
    date,
    body: lines.join("\n").trim(),
  }));
}

/**
 * The bold lead of each `### Highlights` paragraph (`**Name** — …`), used for
 * page descriptions and the release index.
 *
 * @param {string} body
 * @returns {string[]}
 */
export function releaseHighlights(body) {
  const section = body.match(/^### Highlights\s*\n([\s\S]*?)(?=^### |(?![\s\S]))/m);
  if (!section) return [];
  return [...section[1].matchAll(/^\*\*(.+?)\*\*/gm)].map((match) =>
    match[1].replace(/`/g, ""),
  );
}

/** URL slug for a release page: `/releases/<version>/`. */
export function releaseSlug(version) {
  return `releases/${version}`;
}
