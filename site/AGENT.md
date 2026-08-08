# tuika.dev

Nimbus documentation site for tuika, deployed as Cloudflare Worker static
assets.

## Content contract

- Public Markdown below `../docs/` is the only source for guide content. Do not edit generated
  files under `src/content/docs/`; `pnpm sync:docs` recreates them and Git
  ignores them.
- Guide frontmatter belongs in the source file under `../docs/`.
- The custom home page lives at `src/pages/index.astro`. Keep it factual,
  minimal, and consistent with Everruns branding.
- Repository documentation assets are served through `public/docs-assets`, a
  symlink to `../docs`.

## Validation

Run these from `site/` after changes:

```sh
pnpm test
pnpm typecheck
pnpm build
pnpm exec nimbus-docs check --json
pnpm exec wrangler deploy --dry-run
```

The build verifies all public pages, Markdown twins, canonical metadata,
structured data, sitemap, social card, and agent discovery files.

## Deployment

`wrangler.jsonc` owns the `tuika.dev` custom domain and asset binding. Deploy
only from an authenticated Wrangler environment with `pnpm deploy`.
