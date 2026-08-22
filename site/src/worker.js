import { PAGE_ROUTES } from "./lib/routes.js";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const canonicalPath = canonicalPagePath(url.pathname);
    if (canonicalPath && url.pathname !== canonicalPath) {
      return Response.redirect(new URL(canonicalPath + url.search, url), 308);
    }

    const markdownPath = markdownAssetPath(canonicalPath ?? url.pathname);
    const canNegotiate = request.method === "GET" || request.method === "HEAD";

    if (markdownPath && canNegotiate && prefersMarkdown(request.headers.get("Accept") ?? "")) {
      const markdownUrl = new URL(markdownPath, url);
      const response = await env.ASSETS.fetch(new Request(markdownUrl, request));
      if (response.ok) return decorate(response, canonicalPath, true);
    }

    if (canonicalPath) {
      const assetUrl = new URL(htmlAssetPath(canonicalPath), url);
      const response = await env.ASSETS.fetch(new Request(assetUrl, request));
      return decorate(response, canonicalPath);
    }

    return env.ASSETS.fetch(request);
  },
};

export function canonicalPagePath(pathname) {
  const withoutIndex = pathname.endsWith("/index.html")
    ? pathname.slice(0, -"index.html".length)
    : pathname;
  const candidate = withoutIndex === "/" || withoutIndex.endsWith("/")
    ? withoutIndex
    : `${withoutIndex}/`;
  return PAGE_ROUTES.has(candidate) ? candidate : null;
}

export function htmlAssetPath(canonicalPath) {
  return canonicalPath === "/" ? "/index.html" : `${canonicalPath}index.html`;
}

export function markdownAssetPath(pathname) {
  const canonicalPath = canonicalPagePath(pathname);
  if (!canonicalPath) return null;
  return canonicalPath === "/" ? "/index.md" : `${canonicalPath}index.md`;
}

export function prefersMarkdown(acceptHeader) {
  const markdown = qualityFor(acceptHeader, "text/markdown", false);
  return markdown > 0 && markdown >= qualityFor(acceptHeader, "text/html", true);
}

function qualityFor(acceptHeader, mediaType, includeWildcards) {
  let best = 0;
  for (const part of acceptHeader.split(",")) {
    const [rawType, ...params] = part.trim().split(";");
    if (!matches(rawType.toLowerCase(), mediaType, includeWildcards)) continue;
    const qParam = params.find((param) => param.trim().toLowerCase().startsWith("q="));
    const q = qParam ? Number.parseFloat(qParam.split("=")[1]) : 1;
    if (Number.isFinite(q)) best = Math.max(best, Math.min(Math.max(q, 0), 1));
  }
  return best;
}

function matches(candidate, mediaType, includeWildcards) {
  if (candidate === mediaType) return true;
  if (!includeWildcards) return false;
  const [candidateType, candidateSubtype] = candidate.split("/");
  const [mediaTypeType, mediaTypeSubtype] = mediaType.split("/");
  return (candidateType === "*" || candidateType === mediaTypeType) &&
    (candidateSubtype === "*" || candidateSubtype === mediaTypeSubtype);
}

function decorate(response, pathname, negotiated = false) {
  const headers = new Headers(response.headers);
  if (markdownAssetPath(pathname)) appendVary(headers, "Accept");
  if (negotiated) headers.set("Content-Type", "text/markdown; charset=utf-8");
  const links = discoveryLinks(pathname);
  if (links) headers.set("Link", links);
  headers.set("X-Content-Type-Options", "nosniff");
  headers.set("Referrer-Policy", "strict-origin-when-cross-origin");
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

function discoveryLinks(pathname) {
  const markdownPath = markdownAssetPath(pathname);
  if (!markdownPath) return null;
  const canonical = pathname === "/" ? "https://tuika.dev/" : `https://tuika.dev${pathname.replace(/\/$/, "")}/`;
  return [
    `<${canonical}>; rel="canonical"`,
    `<${markdownPath}>; rel="alternate"; type="text/markdown"`,
    `</llms.txt>; rel="service-desc"; type="text/plain"`,
  ].join(", ");
}

function appendVary(headers, value) {
  const vary = headers.get("Vary");
  const values = vary?.split(",").map((item) => item.trim().toLowerCase()) ?? [];
  if (!values.includes(value.toLowerCase())) headers.set("Vary", vary ? `${vary}, ${value}` : value);
}
