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
  "releases",
  ...COMPONENT_SLUGS.map((slug) => `components/${slug}`),
]);

export const PAGE_ROUTES = new Set(
  PAGE_SLUGS.map((slug) => (slug ? `/${slug}/` : "/")),
);

// One page per CHANGELOG.md release (`/releases/0.13.0/`). The worker cannot
// read the changelog, so it recognises the shape; the build verifies the set.
export const RELEASE_ROUTE = /^\/releases\/\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?\/$/;

export function isPageRoute(path) {
  return PAGE_ROUTES.has(path) || RELEASE_ROUTE.test(path);
}
