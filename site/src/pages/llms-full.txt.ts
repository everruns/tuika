// Full-corpus markdown for AI agents — every published page in one
// document. Scope and collation live in the framework helper; reshape or
// delete this route to change the site's corpus policy.
import type { APIContext } from "astro";
import { llmsFullRoute } from "@cloudflare/nimbus-docs/agent-endpoints";
import { normalizeMarkdownLinks } from "../lib/docs-links";

export const prerender = true;

const { GET: nimbusGET } = llmsFullRoute({
  body: (text) => normalizeMarkdownLinks(text),
});

export async function GET(context: APIContext) {
  const response = await nimbusGET(context);
  if (!response.ok) {
    return response;
  }
  const headers = new Headers(response.headers);
  headers.set("Content-Type", "text/plain; charset=utf-8");
  return new Response(response.body, { status: response.status, headers });
}
