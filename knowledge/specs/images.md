---
type: Product Specification
title: Image rendering
description: Defines how tuika paints real pixels over reserved cells via terminal graphics protocols, and why decoding stays in the host.
---

# Image rendering

## Why

tuika renders everything into its own cell buffer, where a cell carries one
grapheme plus a style and nothing else. Pictures — an avatar in a chat
transcript, a chart, a rendered diagram, an image referenced from markdown —
have no home in that model. Terminals that speak a graphics protocol (Kitty
graphics, iTerm2 inline images, Sixel) can paint real pixels, but only via
escape sequences that live *outside* the cell buffer.

tuika already smuggles out-of-band escapes past the cell buffer several times — OSC 8
hyperlinks, OSC 52 clipboard, OSC 9;4 progress, OSC 22 pointer shape, all under
`term`. Image support follows the same shape, with one new constraint those did
not have (see *Cursor* below), which is why the protocol half lives beside them
in `term::image` while the `Image` view lives in `components` with every other
view. That split is the module tree stating the design: pixels are a terminal
concern, reserving cells is a layout concern.

## What

A host can place a decoded image into the layout, sized in whole terminal
cells, and have it painted by the terminal's graphics protocol over the cells
tuika reserved for it. `Runner` detects support and owns the image layer
lifecycle. Custom hosts can supply the same graphics context explicitly. When
the terminal does not support the protocol, the same component degrades to a
text placeholder (its alt text) with no garbage on screen — or, when the host
opts in with `Image::mosaic`, to the picture drawn from block glyphs by a
`Blitter`. The mosaic is opt-in because a finer blitter's glyphs may be missing
from an older terminal font, and a host that does not know its terminal is
better served by readable alt text than by missing-glyph boxes.

Scope is deliberately phased:

- **Phase 1 (this spec's initial cut): standalone `Image` component, Kitty
  graphics protocol only.** A host builds an `Image` from raw RGBA it decoded
  itself, reserves cells for it in the layout, and emits the graphics escapes
  after each frame. Capability detection gates the protocol; unsupported
  terminals get the alt-text fallback.
- **Phase 2: more protocols** behind the same component, selected by capability
  detection. **Done.** `ImageLayer::emit` dispatches per the recorded protocol.
  iTerm2 transmits an image file, so RGBA is PNG-encoded (in-tuika,
  dependency-free). Sixel has no cell-based sizing, so the RGBA is
  nearest-neighbor resampled to an assumed cell geometry, quantized to a fixed
  6×6×6 color cube, and emitted band-by-band with run-length encoding; detection
  is best-effort (`foot`/`mlterm`/`contour` in `TERM`) since Sixel has no
  reliable env signal, so hosts usually set `ImageSupport::Sixel` explicitly.
- **Phase 3: markdown `![alt](url)`**, in two steps:
  - **3a (done): visible placeholder.** The markdown builder parses `Tag::Image`
    and renders a marked, link-styled placeholder (the alt text, or the URL when
    there is no alt) instead of silently dropping the URL. No new API, no
    streaming-cache change — it flows through the existing inline machinery.
  - **3b: pixels in markdown (done).** `Markdown::images` and
    `MarkdownState::with_image_resolver` resolve each image URL to `ImageData`
    via a host-supplied resolver (the `Highlighter` pattern again — markdown has
    only a URL, never pixels), the parser promotes a resolved `![alt](url)` to a
    block `MdItem::Image`, `flatten` reserves its rows and reports where it
    landed, and the view overlays the standalone `Image` component there — reusing
    its protocol emission and alt fallback. Streaming works too: block-image
    placements are threaded through the settled-prefix cache (fixed rows for
    settled blocks, re-derived each frame for the in-flight tail) and published
    via `MarkdownState::images()` as `MarkdownImage`s, which a host paints at
    `rect(area)` after drawing `lines()`.

## Design

### Decoding stays in the host

tuika depends only on crossterm, unicode-\*, and pulldown-cmark,
and adds nothing heavy. Image *decoding* (PNG/JPEG → RGBA) is a
heavy dependency, so it stays in the host exactly like syntax highlighting does:
the host hands tuika an `ImageData` of raw RGBA plus pixel dimensions, the way
it hands markdown a `Highlighter`. tuika owns *presentation* (protocol
encoding, cell reservation, fallback), never decoding.

### The cell-reservation + out-of-band-emit model

An image cannot be written through `Surface` — `Surface` only writes cells. So
the model splits placement from emission, using the boundary `RectProbe` already
established for reading a view's painted rect back out:

1. The `Image` view reserves a `cols × rows` cell footprint via `measure`, so
   the flex solver lays out around it like any other component.
2. On `render`, the view records its **absolute** painted `Rect` plus a handle
   to its pixel data into a shared `ImageLayer` (an `Rc<RefCell<…>>` handle,
   cheap to clone, cleared each frame — the same ownership shape as
   `RectProbe`). It paints the reserved cells blank (or, when unsupported, the
   alt-text placeholder) so the frame diff has stable content there.
3. **After** `terminal.draw()` returns, `Runner` calls `ImageLayer::emit`, which
   writes each image's graphics escape positioned at its cell origin, then
   clears the layer. A custom host performs those same two calls itself.
   The protocol encoder is resolved in step 2 and carried on the placement, not
   matched on in `emit`: `emit` runs in every host, so choosing there would link
   all three encoders into binaries that place no image. See
   [Binary Size Discipline](../processes/maintenance.md#binary-size-discipline).

Emission happens after the frame because the graphics escape paints pixels the
cell buffer knows nothing about; doing it inside the paint pass would fight
the frame diff.

### Cursor: the one new constraint

OSC 8 / 52 / 9;4 are **cursor-neutral** — they can be spliced into a drawn cell
run without moving the cursor, which is why `HyperlinkBackend` can inline them.
Graphics escapes are **not**: Kitty places the image at the cursor. So
`ImageLayer::emit` wraps its writes in a cursor save/restore (`ESC 7` … `ESC
8`) and moves the cursor to each image's cell origin with a CUP (`ESC [ row ;
col H`) before emitting. Kitty placements also set `C=1`; restoring a saved
cursor cannot undo scrolling caused by the protocol's default post-placement
cursor movement when an image reaches the viewport bottom. Net effect on
tuika's cursor model and viewport is nil, so the diff stays consistent.

### Terminal-response suppression

Kitty acknowledges every graphics command on the tty by default. In a
full-screen TUI those replies would be read by the event loop as bogus input.
Every command sets `q=2` to suppress both success and error responses.

### Capability detection is required

The existing out-of-band features rely on "an unknown OSC is swallowed
harmlessly." That does not hold for graphics protocols — an unsupported
terminal may render the payload as visible garbage. So image support is the
first tuika feature to need real capability detection. Phase 1 uses a
conservative environment probe (`TERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID`, and
the Ghostty marker), plus an explicit host override, and defaults to *no*
graphics (text fallback) when unsure. A future phase may add a runtime query
(the Kitty graphics query escape) for certainty.

### Transmit once, place by id (Kitty)

Kitty stores transmitted pixels under an image id and draws them wherever a
placement of that id says, until told otherwise; writing cells over a placement
does not erase it. So the layer keeps what the terminal holds between frames and
writes only the difference: a new image is sent once (`a=t`, under an id from a
per-process sequence kept to 24 bits so it also fits a placeholder cell's
color), each recorded rect becomes a placement (`a=p`, with a placement id per
copy so one image can sit in several places), a moved placement is re-placed
under the same ids, and an image no frame uses any more is deleted with its
data. A frame where nothing changed or moved writes no bytes at all — before,
every redraw (a spinner tick included) base64-encoded and uploaded every pixel.

An image is recognized by its shared buffer, or failing that by equal
dimensions and equal bytes, because hosts such as charts rebuild their
`ImageData` every frame; comparing bytes is exact and far cheaper than
re-sending them. New pixels go out before anything is placed, and old
placements and images are retired only after their replacements are up, so an
ordinary redraw never shows a blank interval.

The cache is only as good as the screen it describes. Anything that may have
erased or moved images behind the layer — a resize, a clear, a scrollback
publish in split-footer mode — has to call `ImageLayer::invalidate`, after which
the next emit deletes everything it owned and starts again under fresh ids.
`Runner` does this from the terminal's screen epoch, a counter its `Terminal`
bumps on every clear, scroll, insert and invalidate; a custom host calls
`invalidate` itself.

iTerm2 and Sixel images live in the cells they cover, and a cell redraw erases
that part of them, so they are still written in full every frame.

## Non-goals

- Image *decoding* in tuika (host's job).
- Animation / video.
- Sub-cell pixel-exact sizing driven by cell-pixel geometry queries — sizing is
  quantized to whole cells (`View::measure` is `u16`).

## Related

- [out-of-band.md](./out-of-band.md) — the escape-emission family this belongs to.
- [markdown.md](./markdown.md) — how block images thread through the streaming cache.
- [goal.md](./goal.md) — the host-owns-decoding boundary.
