export const GUIDE_SLUGS = Object.freeze([
  "charts",
  "components",
  "features",
  "getting-started",
  "keymap",
  "layout",
  "markdown",
  "routing",
  "showcases",
  "styling",
  "themes",
]);

export const COMPONENT_SLUGS = Object.freeze([
  "banners-codes-pixels",
  "interactive",
  "layout",
  "markdown-code",
  "motion",
  "notifications-console",
  "text",
]);

export const PAGE_SLUGS = Object.freeze([
  "",
  ...GUIDE_SLUGS,
  ...COMPONENT_SLUGS.map((slug) => `components/${slug}`),
]);

export const PAGE_ROUTES = new Set(
  PAGE_SLUGS.map((slug) => (slug ? `/${slug}/` : "/")),
);
