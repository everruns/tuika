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
- **Owned scenes are frame descriptions, not retained UI.** `Scene` owns the
  root and overlay elements for one frame and resolves each overlay's
  `OverlaySpec` while rendering. This removes borrowed compositor plumbing
  without adding identity, lifecycle, or hidden persistent state.
- **Scoped scenes borrow only the frame root.** `ScopedScene` lets a concrete
  root read large host-owned state directly while overlays remain owned
  elements. It uses the same renderer and focus-owner resolution as `Scene`;
  keeping the borrowing boundary at the root avoids making `Element` and every
  component lifetime-generic.
- **The host owns the terminal**: the screen mode (alternate screen or a split
  footer over live scrollback — see [screen-modes.md](./screen-modes.md)), raw
  mode, mouse capture, input translation, and frame compositing.

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

## Content the host lays out: lines vs items

Two viewport primitives exist on purpose. `Scroll` windows `Line`s. Its default
mode treats one input line as one content row and can paint O(viewport) without
measuring anything; its opt-in wrapping mode reflows owned lines at render width
and derives the row window then, because width does not exist earlier.
`ItemScroll` windows `Element`s:
each entry is measured at the render width and scrolled by row, so an entry
taller than the space left clips at the edge rather than snapping to an item
boundary.

The second exists because flattening is lossy. A history whose entries are
bordered panels, tables, diffs, or nested layouts cannot be expressed as lines
without the host drawing box glyphs into strings — reimplementing layout in a
place the solver cannot see. Chat and agent UIs are the archetype.

The cost of measuring items is real, so the ownership split mirrors unwrapped
`Scroll`: the owned constructor measures every item every frame; the windowed
one takes the visible slice plus a host-supplied content height, for a host
keeping its own height cache. Because item heights depend on width, the
scrollbar's column is reserved whenever the bar is enabled — not only while
content overflows — so the appearance of a bar cannot silently re-wrap and
re-measure everything above it.

## Input and focus

The host translates crossterm events into tuika's own `Key`/`Mouse` events
before anything else sees them. `TerminalSession` first pushes enhanced keyboard
reporting, because legacy terminal input cannot distinguish non-character
chords such as `Shift+Enter` from `Enter`; it pops exactly its own stack entry on
exit. iTerm2 and tmux's xterm format omit event-type reporting, while tmux CSI-u
also enables `modifyOtherKeys` mode 2. Windows needs no negotiation because its
native console events already include modifier state. Everything above that
boundary — the keymap engine, focus routing, component handlers — consumes only
tuika types, which is what lets all of it be unit-tested without a PTY.

Focus is a registry of scopes rather than a flag per component: a `FocusScope`
claims input ownership for its subtree, so a modal or overlay can take input
without every component learning about modality.

Inline composer tokens follow the same host-agnostic rule. A `Trigger` declares
only *where* an opening character counts (`Anywhere`/`WordStart`/`LineStart`/
`BufferStart`) and where the token ends; `TextInputState` finds and delimits the
matches and can splice a replacement back in. What `@` or `/` **means** — the
completion source, the popup, the styling, whether confirming runs a command or
inserts a path — is the application's, and `TextInput::highlights` paints ranges
the host computed rather than semantics tuika inferred. A toolkit that shipped
"mentions" and "slash commands" as features would be encoding one host's product
decisions; declaring the lexical rule and returning the spans is the seam.

`Viewport` follows the same host-state rule as `Scroll`: offsets live in
`ScrollState`, while the view owns only an ephemeral child and declared content
extent. Its scratch buffer covers only the visible source rectangle, even when
the child's logical extent is much larger.

## Rendering pipeline

1. The host builds the view tree from application state (optionally through the
   `view!` DSL, which expands to the same builder calls — no runtime cost). A
   root may borrow that state for the frame through `ScopedScene`; owned
   component subtrees remain `Element`s.
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

- [api-surface.md](./api-surface.md)
- [goal.md](./goal.md)
- [markdown.md](./markdown.md)
- [images.md](./images.md)
- [keymap.md](./keymap.md)
