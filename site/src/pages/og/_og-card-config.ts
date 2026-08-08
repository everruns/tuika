import type { OGImageOptions } from "astro-og-canvas";
import { fileURLToPath } from "node:url";

export const ogCardDescription = "tuika · Rust terminal application framework";

export const ogCardConfig = {
  bgGradient: [[5, 5, 6]],
  bgImage: {
    path: fileURLToPath(new URL("../../../public/social-card-background.png", import.meta.url)),
    fit: "fill",
  },
  // Cached renders can survive an updated background across local builds.
  // Eighteen small cards are cheap enough to render from source each time.
  cacheDir: false,
  // astro-og-canvas includes the border width in its text measure. The
  // background paints over this end border, leaving a safe title column.
  border: { color: [5, 5, 6], width: 390, side: "inline-end" },
  padding: 72,
  fonts: ["./public/fonts/Inter-Bold.ttf"],
  font: {
    title: {
      color: [250, 250, 250],
      size: 58,
      weight: "Bold",
      families: ["Inter"],
      lineHeight: 1.05,
    },
    description: {
      color: [212, 164, 58],
      size: 24,
      weight: "Bold",
      families: ["Inter"],
      lineHeight: 1.2,
    },
  },
  format: "PNG",
} satisfies Partial<OGImageOptions>;
