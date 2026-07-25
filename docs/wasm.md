# Running tuika outside a terminal (wasm / browser)

tuika splits cleanly into a *presentation* half — layout, overlays, focus, the
keymap, every component, and `paint` — and a *host* half that owns the terminal:
alternate screen, raw mode, input translation, and the write loop. Only the host
half needs a terminal, and only the host half needs `crossterm`.

That split is a feature flag:

```toml
[dependencies]
tuika = { version = "0.4", default-features = false }
```

Turning off the default `crossterm` feature drops `crossterm` and
`ratatui-crossterm`, leaves everything else, and lets the crate build for
`wasm32-unknown-unknown` — a target `crossterm` does not support.

## What you get, and what you give up

| | `default` | `default-features = false` |
| --- | --- | --- |
| Layout, overlays, focus, keymap, components, themes | ✅ | ✅ |
| `paint` / `paint_with_sheet` / `Overlay` | ✅ | ✅ |
| `Buffer`-level testing helpers (`testing::*`) | ✅ | ✅ |
| OSC string builders (`osc8`, `osc52`, `apply_buffer_links`, progress encoders) | ✅ | ✅ |
| `TerminalSession`, `AltScreen`, `translate_event` | ✅ | — |
| `Runner` (and `AsyncRunner`, behind `async`) | ✅ | — |
| `HyperlinkBackend`, `write_line` | ✅ | — |

The escape-sequence *builders* stay available with the feature off, because they
are pure string functions; only the parts that write to a terminal or read
crossterm's event types go away.

## Writing a non-terminal host

A host does four things. On a browser canvas they look like this:

1. **Own a `Buffer`.** `Buffer::empty(Rect::new(0, 0, cols, rows))`, resized when
   the viewport changes. There is no ratatui `Terminal` and no backend — those
   exist to diff against a byte stream, and a canvas has no byte stream.
2. **Composite.** `paint(&mut buffer, area, &theme, root.as_ref(), &overlays)` —
   identical to the terminal path.
3. **Present.** Walk the buffer's cells and draw them. A cell is a symbol, a
   foreground, a background, and a modifier set; on a canvas that is a
   `fillRect` plus a `fillText`. No ANSI is generated or parsed.
4. **Translate input.** Map the platform's events onto tuika's `Key` / `Mouse` /
   `Event::Resize`, exactly as `translate_event` maps crossterm's.

Two details are worth knowing before you start.

**Colors must be resolved.** A terminal owns the 16-color palette, the 256-color
cube, and what `Color::Reset` means. Your host owns none of that, so resolve
every `Color` to real RGB before painting, substituting your theme's foreground
and background for `Reset`.

**Box-drawing glyphs need help.** A terminal guarantees a box-drawing glyph
fills its cell, so borders tile seamlessly down a column. Text drawn into a
canvas cell does not, and borders come out dashed. Draw `U+2500`–`U+257F` and
`U+2580`–`U+259F` as geometry instead of as text.

## Clocks

`std::time::Instant::now()` panics on `wasm32-unknown-unknown` — the browser's
clock is a JS import `std` cannot assume. tuika calls it in exactly one place,
double-click detection, and that entry point is
`SelectionState::handle`, which is compiled out on that target. Call
`SelectionState::handle_at(&mouse, now)` instead, passing a `Duration` from any
epoch you keep stable (`performance.now()`). Everything else in tuika is driven
by a frame counter you supply, so animation, toast expiry, and spinners need no
clock at all.

## A worked example

[`examples/web`](../examples/web) is a complete browser host: a wasm module that
runs a small tuika application, and ~300 lines of JavaScript that paints its
cells onto a `<canvas>` and feeds DOM events back in. It builds with nothing but
`cargo build --target wasm32-unknown-unknown` — no `wasm-bindgen`, no
`wasm-pack`, no npm.

```bash
rustup target add wasm32-unknown-unknown
cd examples/web && ./build.sh --serve
```

It is a research prototype, not a supported deliverable: it demonstrates that
the seam holds, and shows what a real host would have to write.
