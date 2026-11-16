/**
 * Per-page `/<slug>/index.md` — the clean-markdown alternate for every
 * indexable entry of the primary `docs` collection.
 *
 * Non-primary collections (`api`, `blog`, …) mount under their own
 * URL namespace by convention; their `.md` alternates live at the
 * sibling route `pages/<collection>/[...slug]/index.md.ts`. This route
 * filters to the primary collection so multi-collection sites don't
 * generate conflicting `[...slug]` paths at root.
 */

import { getIndexedEntries, type IndexedEntry } from "@cloudflare/nimbus-docs";
import {
  getMarkdownPayload,
  getMarkdownStaticPaths,
  type MarkdownEndpointReference,
} from "@cloudflare/nimbus-docs/agent-endpoints";
import { config } from "virtual:nimbus/config";
import { normalizeMarkdownLinks } from "../../lib/docs-links";

export const prerender = true;

const PRIMARY_COLLECTION = "docs";

interface SlugProps {
  item: IndexedEntry;
  reference: MarkdownEndpointReference;
}

export async function getStaticPaths() {
  const [indexed, paths] = await Promise.all([
    getIndexedEntries(),
    getMarkdownStaticPaths({ collection: PRIMARY_COLLECTION, surface: "markdown" }),
  ]);
  const byId = new Map(
    indexed
      .filter((item) => item.collection === PRIMARY_COLLECTION)
      .map((item) => [item.entry.id, item]),
  );
  return paths
    .filter(({ props }) => byId.has(props.reference.id))
    .map(({ params, props }) => ({
      // The endpoint already normalizes the root index (`id === "index"`)
      // to `slug: undefined`, so `/index.md` keeps working without the
      // route synthesizing paths itself.
      params,
      props: { item: byId.get(props.reference.id), reference: props.reference } as SlugProps,
    }));
}

export async function GET({ props }: { props: SlugProps }) {
  const { item, reference } = props;
  const payload = await getMarkdownPayload({
    collection: reference.collection,
    surface: "markdown",
    reference,
  });
  if (!payload) {
    return new Response("Not found", { status: 404 });
  }
  const { entry, title, description, markdownUrl, sourceUrl, version } = item;
  const data = (entry.data ?? {}) as Record<string, unknown>;
  const rawImage = data.socialImage;
  const socialImage =
    typeof rawImage === "string" && rawImage.length > 0
      ? rawImage
      : config.socialImage;

  const markdown = normalizeMarkdownLinks(payload.content);

  const body = [
    "---",
    `title: ${JSON.stringify(title)}`,
    ...(description ? [`description: ${JSON.stringify(description)}`] : []),
    ...(socialImage
      ? [`image: ${JSON.stringify(new URL(socialImage, config.site).href)}`]
      : []),
    ...(version ? [`version: ${JSON.stringify(version)}`] : []),
    "---",
    "",
    "> Documentation Index",
    `> Fetch the complete documentation index at: ${new URL("/llms.txt", config.site).href}`,
    "> Use this file to discover all available pages before exploring further.",
    "",
    `# ${title}`,
    "",
    markdown,
    "",
    // Point at the authored source (`.mdx` twin) when it exists — the
    // `.md` alternate referencing itself was a placeholder. `item.url`
    // is an unreachable fallback: every entry this route serves has a
    // `.md` URL by construction, and file-backed entries have a source.
    `Source: ${new URL(sourceUrl ?? markdownUrl ?? item.url, config.site).href}`,
    "",
  ].join("\n");

  return new Response(body, {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
}
