---
type: Architecture Specification
title: Accessibility
description: Defines how a tuika frame is read by a screen reader — semantic nodes recorded during paint, a cell fallback for unlabelled views, and a linear plain-text document in reading order.
---

# Accessibility

## Why

A screen reader reads a terminal as a stream of text. A cell-grid UI defeats it:
every repaint is cursor-addressed, borders and spinners are read as glyphs, and
nothing says what has focus or what changed. Ink is the only TUI framework with
an answer; no Rust library had one. tuika owns the paint, so it is the layer that
can say what a frame *means* without each host writing a second UI.

## What

- **Semantics travel with the paint.** `RenderCtx` optionally carries an
  `access::AccessFrame`. While painting, a view records an `access::Node`: the
  cells it covers, a `Role`, a label, a value, and state (focused, selected,
  checked, expanded, disabled, busy, position in its set). Recording takes a
  closure, so a paint without a collector builds nothing.
- **Leaf versus container.** A leaf node replaces the text of the cells it
  covers. A container node (list, tab list, table, tree, dialog, group) is a
  header: everything painted inside its rect is read after it, together.
- **A fallback for unlabelled views.** Cells no leaf node covers are read as
  text, one run per row, with box-drawing, block, Braille, and sextant glyphs
  dropped and blank runs collapsed. Every existing app produces readable output;
  labels improve it. A run stops at a container edge, so side-by-side panels are
  not read as one line.
- **Overlays occlude.** An overlay drops the nodes it fully covers. A scene
  overlay with a dimmed backdrop is modal: everything outside it is inert.
- **Apps label their own views** with `access::Semantic`, a layout-transparent
  wrapper in the shape of `probe::Probe`; `Semantic::hidden` marks decoration.
- **Screen-reader mode.** `Runner` and `AsyncRunner` speak instead of
  painting when `with_screen_reader` is active — by default
  `ScreenReader::Auto`, which follows `TUIKA_SCREEN_READER`, so a user turns it
  on for any app without the app's help. The loop is the same one (ticks,
  events, redraw scheduling, `update`); only its terminal differs: an
  off-screen sink at the real size, a session with raw input, no alternate
  screen, no mouse capture, and a visible cursor. Published scrollback blocks
  are spoken whole, in the order a split footer would commit them.
- **Narration speaks only changes.** `access::Narrator` keeps the previous
  document and writes new alerts first, then the newly focused node when focus
  moved, then every other new or changed line. A focused text input is
  rewritten in place (carriage return, clear line) while typing and committed
  with a newline when focus leaves. Lines end in `\r\n` for raw mode.
- **Testing.** `testing::screen_reader` returns a frame's spoken text, so hosts
  assert it the way they assert cells; `demo -- <scene> --screen-reader` prints
  it for a gallery scene.

## Constraints

- Output is data written to a terminal: labels and cell text are stripped of
  control characters before they reach a `Document`.
- No new required `View` method. Semantics are opt-in recording, so third-party
  views keep compiling and are read through the cell fallback.
- OS accessibility APIs (AT-SPI, UIA, NSAccessibility) are out of scope: a
  terminal application cannot reach them; the emulator owns that bridge.
- The `access` module stays behind its own path, like `probe`: hosts name it
  when they opt in, not on every frame.
- The switch is a runner builder method, not a `RunnerConfig` field: hosts
  build that struct literally, so a new field would break them for a setting
  most never touch.
- The only escape the mode writes itself is clear-line, for the input line;
  no cursor addressing and no colour reach the terminal.

## Related

- [architecture.md](./architecture.md)
- [api-surface.md](./api-surface.md)
- [screen-modes.md](./screen-modes.md)
