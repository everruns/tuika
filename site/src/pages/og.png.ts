import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

export const prerender = true;

export async function GET() {
  const body = await readFile(resolve(process.cwd(), "public/social-card.png"));

  return new Response(body, {
    headers: { "Content-Type": "image/png" },
  });
}
