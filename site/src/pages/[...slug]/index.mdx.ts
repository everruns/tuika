/**
 * Per-page `/<slug>/index.mdx` — the raw authored source for every
 * indexable entry of the primary `docs` collection that has a string body.
 *
 * Twin grammar: `index.md` is the downleveled render for reading,
 * `index.mdx` is the source — imports, JSX, and directives intact. The
 * body is served verbatim; only the canonical frontmatter block (shared
 * with the `.md` twin) is framework-shaped.
 *
 * Non-primary collections (`api`, `blog`, …) follow the same sibling-route
 * convention as `index.md.ts`: their `.mdx` alternates live at
 * `pages/<collection>/[...slug]/index.mdx.ts`.
 */

import { getIndexedEntries, type IndexedEntry } from "@cloudflare/nimbus-docs";
import {
  getMarkdownPayload,
  getMarkdownStaticPaths,
  type MarkdownEndpointReference,
} from "@cloudflare/nimbus-docs/agent-endpoints";
import { config } from "virtual:nimbus/config";

export const prerender = true;

const PRIMARY_COLLECTION = "docs";

interface SlugProps {
  item: IndexedEntry;
  reference: MarkdownEndpointReference;
}

export async function getStaticPaths() {
  const [indexed, paths] = await Promise.all([
    getIndexedEntries(),
    getMarkdownStaticPaths({ collection: PRIMARY_COLLECTION, surface: "source" }),
  ]);
  const byId = new Map(
    indexed
      .filter(
        (item) =>
          item.collection === PRIMARY_COLLECTION && item.sourceUrl !== undefined,
      )
      .map((item) => [item.entry.id, item]),
  );
  return paths
    .filter(({ props }) => byId.has(props.reference.id))
    .map(({ params, props }) => ({
      // Same root-index shape as the `.md` twin; the endpoint normalizes
      // `id === "index"` to `slug: undefined` (`/index.mdx`).
      params,
      props: { item: byId.get(props.reference.id), reference: props.reference } as SlugProps,
    }));
}

export async function GET({ props }: { props: SlugProps }) {
  const { item, reference } = props;
  const payload = await getMarkdownPayload({
    collection: reference.collection,
    surface: "source",
    reference,
  });
  if (!payload) {
    return new Response("Not found", { status: 404 });
  }
  const { entry, title, description, version } = item;
  const data = (entry.data ?? {}) as Record<string, unknown>;
  const rawImage = data.socialImage;
  const socialImage =
    typeof rawImage === "string" && rawImage.length > 0
      ? rawImage
      : config.socialImage;

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
    payload.content,
  ].join("\n");

  return new Response(body, {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
}
