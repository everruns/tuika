# tuika in the browser — research prototype

A complete non-terminal host for tuika: the crate compiles to
`wasm32-unknown-unknown`, and `tuika.js` paints its cell buffer onto a
`<canvas>` and feeds DOM events back in.

This exists to answer one question — *does tuika's host seam actually hold
outside a terminal?* — and to show what the answer costs. It is not a published
crate and not a supported deliverable. See [`docs/wasm.md`](../../docs/wasm.md)
for the guidance it produced.

```bash
rustup target add wasm32-unknown-unknown
./build.sh --serve      # http://localhost:8000
```

`tab` switches tabs · `↑`/`↓` moves · `enter` opens · `t` pushes a toast ·
`?` opens the key map · the wheel scrolls the log.

## Layout

| File | What it is |
| --- | --- |
| `src/app.rs` | The application. Ordinary tuika — it does not know it is in a browser. |
| `src/lib.rs` | The wasm ABI: packed cells out, key/mouse/paste in. |
| `src/palette.rs` | `ratatui` `Color` → RGB, since a canvas has no palette. |
| `tuika.js` | The canvas "terminal emulator" and DOM event translation. |
| `index.html` | The page. |

## How it works

tuika is already split so the terminal lives on one side of `paint`. Building
for the browser is therefore not a port — it is supplying the other four things
a host does:

1. Own a `Buffer` and resize it when the viewport changes.
2. Call `paint(&mut buffer, area, &theme, root, &overlays)`.
3. Present the cells.
4. Translate input into tuika `Event`s.

Steps 1, 2 and 4 are in `src/lib.rs`; step 3 is in `tuika.js`. No tuika code
was modified to make this run — the only library change was making `crossterm`
an optional (default-on) feature, since it does not compile for wasm.

### Why no wasm-bindgen

The whole interface is a handful of `extern "C"` functions plus one flat array
the JavaScript side reads straight out of linear memory:

```
tk_start(cols, rows)     tk_render() -> cell count      tk_key(code, ch, mods)
tk_resize(cols, rows)    tk_cells()  -> *PackedCell     tk_mouse(kind, button, col, row, mods)
tk_background()          tk_atlas()  -> *u8             tk_paste_buffer(len) / tk_paste()
```

Each `PackedCell` is 16 bytes: a symbol offset and length into a per-frame
symbol blob, style flags, and pre-resolved RGB foreground and background. A
96×34 grid is ~52 KB per frame, read with one `DataView` and no marshalling.
Keeping the boundary this thin means the demo builds with plain `cargo build`
and the per-frame cost is visible rather than hidden behind generated glue.

### What the browser gives you free, and what it doesn't

Free: a monospace grid, 24-bit color, bold/italic/underline/strikethrough,
key and mouse events, and a resize signal. That is most of what tuika asks a
terminal for.

Not free, and implemented here:

- **Color resolution.** A terminal owns the ANSI palette and decides what
  `Color::Reset` means. `src/palette.rs` resolves all 256 slots plus `Reset`.
- **Box-drawing.** A terminal's glyph cells tile; canvas text does not, so
  borders render dashed. `drawGraphic` in `tuika.js` paints `U+2500`–`U+259F`
  as geometry — the single largest fidelity fix in the prototype.
- **A clock.** `Instant::now()` panics on `wasm32-unknown-unknown`, so
  double-click timing goes through `SelectionState::handle_at`.

## Known gaps

Deliberate, since the point was the seam and not a product:

- Images (Kitty/iTerm2/Sixel) are terminal protocols with no canvas analogue;
  a browser host would draw the RGBA directly instead.
- OSC 8 hyperlinks, OSC 52 clipboard, and OSC 9;4 progress are emitted as
  strings by tuika but have no consumer here; a real host would map them to
  `<a>` semantics, `navigator.clipboard`, and the page title.
- The canvas is repainted in full each frame. ratatui's buffer diff exists to
  cut *terminal writes*; a real host would diff against the previous packed
  frame to cut `fillText` calls.
- No IME, no accessibility tree. Both are real work for a production host.
