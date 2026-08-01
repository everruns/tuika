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
- **Interactive state has one lifecycle vocabulary.** Component handlers return
  `InputOutcome`: ignored events may continue routing; recognized no-ops stop;
  persistent mutations, submission, and cancellation remain distinct. Values
  are read from the host-owned state rather than duplicated in outcome enums.
  For state types with a zero-argument constructor, `Default` and `new()` are
  identical; a meaningful alternate start is explicit (for example,
  `SelectState::unselected()`).
- **Docking is state plus resolved geometry, not a retained panel manager.** A
  host-owned dock state tracks only visibility and focus; the current frame
  resolves it to a wide dock, a narrow focused drawer, or a hidden passive
  panel. Views, focus ids, keymaps, scrolling, and domain state remain with the
  host.
- **Layout is a flexbox subset** over a direction-agnostic axis, so rows and
  columns share one solver: `Dimension` (`Auto`/`Fixed`/`Percent`/`Flex`) plus
  `Align`, `Justify`, and `Direction`.
- **Overlays composite over the base tree** rather than nesting inside it, so
  input routing can give the topmost layer first refusal. Screen anchors keep a
  dialog independent of where it is declared; target placement follows a
  `RectProbe` from the already-painted root for popovers and menus, with
  cross-axis alignment, gaps, edge-aware flipping, and final screen clamping.
- **Selection policy is host-configurable**: the core state owns cursor and
  checked-item transitions, while aliases and mouse geometry are explicit
  inputs. Hosts can share picker behavior without inheriting hard-coded keys or
  layout assumptions.
- **Key bindings are the help source of truth**: active labeled bindings expose
  layer priority, and component adapters derive responsive footer hints and
  complete help rows from the same declarations used for dispatch.
- **Single-line input is a state invariant**: search/command inputs normalize
  line boundaries at every public mutation boundary and expose borrowed text;
  hosts do not repair multiline editor state during rendering.
- **Owned scenes are frame descriptions, not retained UI.** `Scene` owns the
  root and overlay elements for one frame and resolves each overlay's
  `OverlaySpec` while rendering. This removes borrowed compositor plumbing
  without adding identity, lifecycle, or hidden persistent state.
- **Scoped element trees borrow for one frame.** `Element` remains the owned,
  `'static` box used by retained host seams, while `ScopedElement<'frame>` may
  hold a borrowed view anywhere in the base component tree. Composition
  containers are generic over their child view type and default to `Element`,
  preserving the ordinary owned path without parallel scoped components.
  `ScopedScene` composes a borrowed base tree with owned overlays and shares the
  same renderer and focus-owner resolution as `Scene`.
- **The host owns the terminal**: the screen mode (alternate screen or a split
  footer over live scrollback — see [screen-modes.md](./screen-modes.md)), raw
  mode, mouse capture, input translation, and frame compositing.
- **Runners own application state directly**: rendering receives `&State`,
  updates receive `&mut State` plus a tick or input signal, and repaint is an
  explicit dirty result (or, synchronously, an external redraw request).
  Rendering stays pure, and idle ticks do not churn the terminal. The
  synchronous runner's monotonic clock defaults to the process clock and is
  replaceable for deterministic hosts.

`measure` and `render` are the two halves of every view, and both receive the
same `RenderCtx`: `measure` reports a size in whole cells so the solver can
allocate, while `render` paints into the clipped region it was given. Theme,
stylesheet, and focus therefore cannot silently fall back to defaults during
layout. Sizes are whole cells throughout — there is no sub-cell layout — which
is why image sizing is quantized to cells rather than driven by pixel geometry.
A container forwards the context while measuring children against the content
box they will actually receive after its own resolved padding. An explicit
component padding overrides stylesheet padding. When a `Flex` is itself measured, its
declared fixed and percent dimensions contribute their resolved main-axis sizes;
the child's intrinsic size is the basis only for auto and flexible children.

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

- `RectProbe` records a view's painted absolute rect. A `SceneOverlay` may read
  a root probe later in the same paint to resolve target-relative placement.
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

All collection types share `VirtualWindow` for clamped absolute ranges and the
`Scrollbar` view for vertical or horizontal position. This unifies range and
thumb math without collapsing the distinct line/item measurement models.
`SelectList::windowed` and `Table::windowed` let a host supply only that range;
selection remains absolute and auto-width table columns intentionally measure
only supplied rows, so stable virtualized widths use fixed or flex columns.

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
without every component learning about modality. At each frame boundary the
host calls `begin_frame`, registers the complete current ring, and then queries
or routes focus. A focused id absent from that ring falls back immediately to
its first registration; the next frame boundary commits that fallback, so a
temporarily removed id cannot steal focus if it later returns.

Text-input positions exposed to hosts remain char indices, matching token and
highlight spans. Editing nevertheless moves and deletes by grapheme boundary,
and soft-wrap plus cursor placement use grapheme display width in terminal
cells. Keeping one cell-width model across measurement, painting, and cursor
math prevents CJK and multi-scalar emoji from drifting apart.

Time-sensitive component state never owns an unreplaceable wall clock. Mouse
double-click detection and the synchronous runner consume the root `Clock`
seam, defaulting to `SystemClock`; animation frames, toast expiry, and keymap
timeouts remain values or ticks explicitly advanced by the host. The async
runner uses Tokio time, whose paused-time facility is its deterministic clock.

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
   `view!` DSL, which expands to the same builder calls — no runtime cost).
   `element` preserves frame borrows as `ScopedElement`, so borrowed views may
   appear at any depth in the base tree; `ScopedScene` adds owned overlays.
2. The solver resolves the tree to rects using the frame's active `RenderCtx`.
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
