import { defineCollection } from "astro:content";
// `z` re-exported from `astro:content` is deprecated; import it from
// `astro/zod` (the pattern nimbus-docs' own schema helpers document).
import { z } from "astro/zod";
import { docsCollection, partialsCollection } from "@cloudflare/nimbus-docs/content";

export const collections = {
  docs: defineCollection(
    docsCollection({
      schemaFields: {
        // Nimbus docs are agent-friendly by default. Set `audience: human`
        // to flag a page that's written primarily for human readers.
        audience: z.literal("human").optional(),
        // Astro's glob loader takes the entry id from `slug` when present.
        // Generated release pages set it so `/releases/0.13.0/` keeps its dots.
        slug: z.string().optional(),
      },
    }),
  ),
  partials: defineCollection(partialsCollection()),
};
