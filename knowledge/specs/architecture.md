---
type: Architecture Specification
title: Rendering Architecture
description: Defines tuika's view/state/layout/host model and the seams that keep it host-agnostic.
---

# Rendering Architecture

## Why

A terminal UI toolkit has two plausible shapes: a retained widget tree with a
reconciler, or an immediate-mode redraw. ratatui already diffs a cell buffer
against the terminal every frame, so a second reconciliation layer above it
would duplicate work and introduce a second source of truth for "what is on
screen". tuika therefore builds *on top of* the diff ratatui already performs
rather than beside it.

## The model

- **Views are ephemeral.** A `View` is rebuilt from application state every
  frame. There is no identity, no keys, no lifecycle. This is affordable because
  ratatui diffs the resulting buffer, so an unchanged frame writes nothing to
  the terminal.
- **State that must survive lives in the host.** Scroll offset, selection index,
  focus registry, text-input cursor, and toast expiry are `*State` structs the
  host owns and passes back in each frame — the `StatefulWidget` idiom. A view
  never hides mutable state a host cannot inspect or persist.
- **Layout is a flexbox subset** over a direction-agnostic axis, so rows and
  columns share one solver: `Dimension` (`Auto`/`Fixed`/`Percent`/`Flex`) plus
  `Align`, `Justify`, and `Direction`.
- **Overlays anchor over the base tree** rather than nesting inside it, so a
  dialog's position is independent of where it is declared and input routing can
  give the topmost overlay first refusal.
- **The host owns the terminal**: alternate screen, raw mode, mouse capture,
  input translation, and frame compositing.

`measure` and `render` are the two halves of every view: `measure` reports a
size in whole cells so the solver can allocate, `render` paints into the clipped
region it was given. Sizes are whole cells throughout — there is no sub-cell
layout — which is why image sizing is quantized to cells rather than driven by
pixel geometry.

## Why the `ratatui-core` seam, not the umbrella

tuika renders none of ratatui's own widgets, so it depends on `ratatui-core`
(plus `ratatui-crossterm` for the backend) directly. This drops
`ratatui-widgets`, `ratatui-macros`, and their transitive weight from every
downstream build that does not use them.

It costs nothing in interoperability: the interop seam is a raw
`&mut Buffer` from that same `ratatui-core`. A host bringing the `ratatui`
umbrella resolves to one shared `ratatui-core`, so `Surface::render_ratatui` and
`RatatuiView` accept any real ratatui widget without conversion. The
`underline-color` feature is enabled to match the umbrella's default so cell
rendering is byte-identical either way.

## The host boundary is a feature, not an assumption

Everything above `paint` is pure computation: the solver, overlays, focus, the
keymap, and every component read state and write cells. Only the layer *below*
it needs a terminal — alternate screen, raw mode, crossterm's event types, and
the write loop.

That line is drawn in the manifest: `crossterm` is a default-on feature carrying
`TerminalSession`, `AltScreen`, `translate_event`, `Runner`, `AsyncRunner`, and
`HyperlinkBackend`/`write_line`. With it off, the crate has no platform I/O and
builds for `wasm32-unknown-unknown`, which crossterm does not support.

This is not speculative portability. It is the same claim the architecture
already makes — that tuika owns presentation and the host owns acquisition —
made checkable: a `use crossterm::…` that leaks out of the host layer now fails
a CI job instead of passing unnoticed. A non-terminal host supplies four things
(own a `Buffer`, call `paint`, present the cells, translate input); everything
else it inherits. See [`docs/wasm.md`](../../docs/wasm.md) and the browser
prototype in `examples/web/`.

Two consequences are worth stating, because they are the only places the model
assumed a terminal:

- **Colors are unresolved.** tuika emits `Color::Reset` and the ANSI palette
  slots and lets the terminal decide what they mean. A host without a palette
  must resolve them itself; tuika should not grow a palette table to spare it.
- **`std` has no clock on `wasm32-unknown-unknown`.** tuika reads the clock in
  exactly one place — double-click detection — so `SelectionState::handle_at`
  takes a caller-supplied timestamp and `handle` (which reads `Instant`) is
  compiled out on that target. Everything else is driven by a host-supplied
  frame counter, which is why animation, spinners, and toast expiry need no
  clock at all. New time-dependent behavior must follow the frame-counter rule
  rather than reaching for `Instant`.

## The probe pattern

Some facts about a frame are only known *after* it is painted — where a view
actually landed, which images need out-of-band emission. tuika does not solve
this by returning values up the render call chain (which would make `render`
signatures host-specific); it uses a shared, frame-scoped handle the view writes
into and the host reads back:

- `RectProbe` records a view's painted absolute rect.
- `ImageLayer` records each image's rect plus its pixel handle, for emission
  after `terminal.draw()` returns (see [images.md](./images.md)).

Both are cheap-to-clone `Rc<RefCell<…>>` handles, cleared each frame. New
"the host needs to know where something landed" requirements should reuse this
shape rather than invent a third.

## Input and focus

The host translates crossterm events into tuika's own `Key`/`Mouse` events
before anything else sees them. Everything above that boundary — the keymap
engine, focus routing, component handlers — consumes only tuika types, which is
what lets all of it be unit-tested without a PTY.

Focus is a registry of scopes rather than a flag per component: a `FocusScope`
claims input ownership for its subtree, so a modal or overlay can take input
without every component learning about modality.

## Rendering pipeline

1. The host builds the view tree from application state (optionally through the
   `view!` DSL, which expands to the same builder calls — no runtime cost).
2. The solver resolves the tree to rects.
3. Views paint into a clipped `Surface`; overlays composite over the base.
4. The host calls out-of-band emitters (images, native progress) after the
   frame, because those escapes paint outside the cell model.
5. ratatui diffs the buffer and writes only what changed.

## Non-goals

- No virtual DOM, keys, or component identity.
- No global mutable state inside the library.
- No layout cache across frames: the solver is cheap and a cache would need
  invalidation the model deliberately lacks. Caching that *is* worth it lives in
  a component's own state (e.g. `MarkdownState`'s settled-prefix cache).

## Related

- [goal.md](./goal.md)
- [markdown.md](./markdown.md)
- [images.md](./images.md)
- [keymap.md](./keymap.md)
