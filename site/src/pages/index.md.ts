export const prerender = true;

export function GET() {
  const markdown = `---
title: "tuika"
description: "A Rust framework for building complete terminal applications on ratatui."
---

# tuika

Rust terminal application framework: layout, anchored overlays, focus, keymaps,
components, and terminal lifecycle over ratatui.

## Install

\`\`\`sh
cargo add tuika
\`\`\`

## Model

- Views are rebuilt from application state each frame.
- Your application keeps selection, scroll, focus, and input state in its own structs.
- tuika solves layout, composites overlays, and paints a deterministic buffer.
- ratatui handles the final terminal diff.
- Existing ratatui widgets compose through \`RatatuiView\`.

Use tuika when an application needs structure around ratatui. For a small screen
made from a few widgets, ratatui alone may be enough.

## Documentation

- [Getting started](https://tuika.dev/getting-started/index.md)
- [Component gallery](https://tuika.dev/components/index.md)
- [Layout](https://tuika.dev/layout/index.md)
- [Markdown](https://tuika.dev/markdown/index.md)
- [Keymap](https://tuika.dev/keymap/index.md)
- [Styling](https://tuika.dev/styling/index.md)
- [Themes](https://tuika.dev/themes/index.md)
- [Terminal features](https://tuika.dev/features/index.md)
- [Charts](https://tuika.dev/charts/index.md)
- [Showcases](https://tuika.dev/showcases/index.md)
- [Rust API](https://docs.rs/tuika)
- [Source](https://github.com/everruns/tuika)

Full documentation index: https://tuika.dev/llms.txt
`;

  return new Response(markdown, {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
}
