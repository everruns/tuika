import { getCollection } from "astro:content";
import { OGImageRoute } from "astro-og-canvas";
import { ogCardConfig, ogCardDescription } from "./_og-card-config";

const entries = await getCollection("docs", (entry) => !entry.data.draft);

const pages = Object.fromEntries(
  entries.map((entry) => [
    entry.id,
    {
      title: entry.data.title,
    },
  ]),
);

export const { getStaticPaths, GET } = await OGImageRoute({
  pages,
  // The default slug strips everything after the last dot as an extension,
  // which would turn `releases/0.13.0` into `releases/0.13.png`. Entry ids
  // carry no extension, so append one.
  getSlug: (path) => `${path}.png`,
  getImageOptions: (_path, page) => ({
    title: page.title,
    description: ogCardDescription,
    ...ogCardConfig,
  }),
});
