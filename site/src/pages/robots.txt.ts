import { config } from "virtual:nimbus/config";

export const prerender = true;

export function GET() {
  const body = [
    "User-agent: *",
    "Allow: /",
    "Content-Signal: ai-train=yes, search=yes, ai-input=yes",
    "",
    "User-agent: GPTBot",
    "Allow: /",
    "",
    "User-agent: ClaudeBot",
    "Allow: /",
    "",
    `Sitemap: ${new URL("/sitemap.xml", config.site).href}`,
    "",
  ].join("\n");

  return new Response(body, {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  });
}
