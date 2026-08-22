---
type: Architecture Specification
title: Rendering Architecture
description: Defines tuika's view/state/layout/host model and the boundaries that keep it host-agnostic.
---

# Rendering Architecture

## Why

A terminal UI toolkit has two plausible shapes: a retained widget tree with a
reconciler, or an immediate-mode redraw. Diffing a cell buffer
against the terminal every frame, so a second reconciliation layer above it
would duplicate work and introduce a second source of truth for "what is on
screen". tuika therefore makes that buffer diff its *only* reconciliation
rather than adding a second layer above it.

## The model

- **Views are ephemeral.** A `View` is rebuilt from application state every
  frame. There is no identity, no keys, no lifecycle. This is affordable because
  `Buffer::diff` compares the resulting buffer against the last frame, so an
  unchanged frame writes nothing to the terminal.
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
- **Layout is an integer-native flexbox subset** over a direction-agnostic axis.
  Container style owns direction, two-axis gaps, wrapping, justification, item
  alignment, and line alignment; `FlexItemStyle` separately owns basis, grow,
  shrink, min/max, and `align_self`. Positive and negative free space use
  weighted cumulative boundaries, so rounded cell allocations sum exactly.
  `Flow` packages intrinsic-width wrapping. `Grid` deliberately stops at
  equal-column row-major placement rather than importing CSS Grid semantics.
- **Overlays composite over the base tree** rather than nesting inside it, so
  input routing can give the topmost layer first refusal. Screen anchors keep a
  dialog independent of where it is declared; target placement follows a
  `RectProbe` from the already-painted root for popovers and menus, with
  cross-axis alignment, gaps, edge-aware flipping, and final screen clamping.
- **Selection policy is host-configurable**: the core state owns cursor and
  checked-item transitions, while aliases and mouse geometry are explicit
  inputs. Hosts can share picker behavior without inheriting hard-coded keys or
  layout assumptions.
- **Collection identity can be application-keyed without becoming view
  identity.** Borrowed keyed tables store stable domain keys in host-owned
  selection state, so reorder, filters, and streaming inserts do not reinterpret
  an index as a different record. Rows remain borrowed frame inputs and the view
  is still ephemeral; keys do not create retained components or lifecycle.
- **Selectable viewports persist independently of selection centering.**
  `SelectViewportState` resolves one explicit `VirtualWindow` that is reused by
  rendering and mouse hit testing. Selection crossing an edge moves the window
  minimally; resize and collection refresh are explicit reconciliation points.
  Centering and edge-following are the *same* window math (`VirtualWindow::around`
  and `VirtualWindow::keeping`) with and without a remembered start, so a
  stateless component can offer either policy via `SelectionAnchor` without a
  host threading persistent state through its model to get the common one.
  `TreeState` extends the same host-owned model with stable node identity,
  expansion, and remembered ancestry while domain traversal remains outside the
  toolkit.
- **Key bindings are the help source of truth**: active labeled bindings expose
  layer priority, and component adapters derive responsive footer hints and
  complete help rows from the same declarations used for dispatch.
- **Single-line input is a state invariant**: search/command inputs normalize
  line boundaries at every public mutation boundary and expose borrowed text;
  hosts do not repair multiline editor state during rendering.
- **Completion is derived host state.** Candidate ownership and acceptance
  semantics stay with the application; completion state stores the query,
  ranked indices, and cursor needed to render command palettes or token popups.
  Refreshing the same query preserves a stable selected replacement.
- **Dialog presets compose primitives.** Confirm, choice, multi-choice, and
  input flows pair host-owned state with a one-frame `Dialog` snapshot. They do
  not introduce a retained modal manager or a second input-outcome vocabulary.
- **Application shells compose regions.** `AppShell` is a thin flex allocation
  around one growing main view plus intrinsic, optional chrome. Regions remain
  ordinary frame-scoped views; short terminals collapse separators and
  secondary chrome without introducing navigation, input, or host policy.
- **Selection screens are shell presets, not a new state model.** Action,
  agent, permission, and resume pickers compose `AppShell`, the borrowed
  `SelectList` row renderer, `SelectState`, and `KeyHints`. Their viewport is
  derived from the body rows allocated this frame, while row ownership,
  navigation, submission, and cancellation remain with the host.
- **Activity and progress answer different questions.** Activity items model
  lifecycle across steps; a progress bar models amount complete for one
  measurable operation and may be composed inside an activity item.
- **Owned scenes are frame descriptions, not retained UI.** `Scene` owns the
  root and overlay elements for one frame and resolves each overlay's
  `OverlaySpec` while rendering. This removes borrowed compositor plumbing
  without adding identity, lifecycle, or hidden persistent state.
- **Scoped element trees borrow for one frame.** `Element` remains the owned,
  `'static` box used by retained host boundaries, while `ScopedElement<'frame>` may
  hold a borrowed view anywhere in the base component tree. Composition
  containers are generic over their child view type and default to `Element`,
  preserving the ordinary owned path without parallel scoped components.
  `ScopedScene` composes a borrowed base tree with owned overlays and shares the
  same renderer and focus-owner resolution as `Scene`.
- **One-off views stay in the same model.** `view_fn` adapts separate `Fn`
  measurement and rendering closures into an ordinary view. Captured frame
  borrows remain bounded by `ScopedElement`; the adapter adds no allocation,
  retained identity, mutation, or alternate layout language.
- **The host owns the terminal**: the screen mode (alternate screen or a split
  footer over live scrollback — see [screen-modes.md](./screen-modes.md)), raw
  mode, mouse capture, input translation, and frame compositing.
- **A runner restores the terminal affordance its mouse capture removes.** Plain
  left drags that application update code leaves clean select from the final
  rendered cell buffer, receive the theme selection style, and copy through OSC
  52 on release. Wheel input still routes to the application. A handled gesture
  returns `UpdateResult::Consumed` or `Dirty`; custom-loop hosts keep using the
  public selection primitives directly.
- **A drag belongs to the panel it starts in.** A bordered `Boxed` records its
  inner rect as a selection region while rendering; the runner resolves the
  press cell to the innermost region and confines the whole gesture there, so a
  linear selection wraps at the panel's edges and the copied text is that
  panel's text. The screen remains the region when a drag starts outside every
  panel. Regions come from the previously painted frame, which is what the
  pointer was actually aimed at; a range is always resolved against the area it
  is given, so geometry that changed underneath it can only narrow the
  selection, never reach a cell outside.
- **Runners own application state directly**: `Application` (and its awaiting
  counterpart `AsyncApplication`) receives signals through `&mut self` and
  builds a `ScopedElement<'_>` through `&self`, so a frame can borrow
  application data without shared interior mutability. The compatible closure
  boundary renders an owned `Element` from `&State` and updates through
  `&mut State`. Both boundaries are kept: the application boundary is more general in
  what a frame may *return* (`Element` is `ScopedElement<'static>`) and states
  render purity in its receiver, while the closure boundary's `FnMut` view is more
  permissive in what it may *capture*. Repaint is an explicit dirty result or an
  external redraw request; terminal resize is the exception because layout
  must be repainted even when application state is unchanged. Rendering stays
  pure, and idle ticks do not churn the terminal. The
  synchronous runner's monotonic clock defaults to the process clock and is
  replaceable for deterministic hosts.
  `RunnerCore` is the runtime-neutral dirty/render/exit state machine used by
  both sync and async shells; event polling, clocks, sleeping, and terminal I/O
  remain outside it.
  The async shell may additionally select over a typed application stream;
  messages use the same dirty/exit contract as events and ticks, and completion
  disables only that source. That stream is the async shell's alone — a
  synchronous loop has nothing to select with — so a background producer's
  runner-neutral option stays the redraw handle, which both runners expose and
  which wakes a parked async loop rather than waiting for its next tick.
- **The loop is reachable without a terminal.** `run_driven_by` is the loop with
  no session, no terminal construction, and no stdout-facing work; every other
  entry point on both runners is built on it. A host that owns its terminal and
  input calls it directly, and a test drives a whole application through it over
  an in-memory backend — the same hermetic principle the view layer already had,
  extended to the run loop. The synchronous side gets its input through an
  `EventSource` because it blocks on a timeout rather than selecting on a
  stream; that trait is the boundary a scripted or custom input implements.
- **The frame source is an argument, not a method name.** `FrameSource` has two
  implementors — `&mut app` for an `Application`, and `from_fn` over a state
  value and closures — so a run method never has to name which one it takes.
  Messages ride the same `Signal<M>`, whose default message type is
  uninhabited, so a loop without a stream has no variant to handle and existing
  two-arm matches stay exhaustive. What remains in the names is only terminal
  ownership and whether there is a message stream; the runtime is the runner
  type, and everything else is `RunnerConfig` or a builder. A capability added
  at one point of that surface belongs across it — a cross product encoded as
  names grows holes, which is what the pre-0.9 `run*`/`run_app*` matrix did.

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
`measure_request` adds optional known axes plus definite, min-content, and
max-content availability. Its default adapts to the original `measure`, so old
views remain source-compatible; the non-exhaustive request and availability
types can gain future measurement inputs without another trait-wide break.

## Why tuika owns the cell grid

tuika has no runtime dependency on ratatui. It owns its own `Rect`/`Position`,
`Color`/`Modifier`/`Style`, `Line`/`Span`, `Buffer`/`Cell`, `Backend`, and
`Terminal` — everything from the layout area down to the bytes on the wire.

This was not always so, and the reasoning for the change is worth keeping:

- **The public API was version-locked to someone else's crate.** Every `View`
  signature named a `ratatui-core` type, so a `ratatui-core` major bump was a
  breaking release for tuika, all four companion crates, and every host at once.
  Owning the vocabulary makes that a private implementation detail.
- **Half the dependency weight was for code tuika never called.** `ratatui-core`
  pulls `kasuari` (a Cassowary solver), `lru`, and a second `hashbrown` for
  `layout::Layout` — which tuika replaced with its own flex solver on day one —
  plus `strum`, `compact_str`, `itertools`, and `unicode-truncate`. Removing it
  took the default graph from 55 crates to 32 and the cold dependency build from
  roughly 32s to 6s.
- **The cell model had already outgrown it.** OSC 8 hyperlinks do not fit in a
  ratatui `Cell`, which is why `HyperlinkBackend` existed as a whole `Backend`
  implementation whose job was to re-scan finished cells for URLs.

What did *not* motivate it: performance. That the render benchmarks came out
~7% faster is a side effect of `Cell` storing short grapheme clusters inline
rather than in a `CompactString`, not the goal.

### Interoperability is preserved, not abandoned

`Surface::render_ratatui` and `RatatuiView` still accept any real ratatui
widget, behind the optional `ratatui` feature. The boundary is no longer a
shared `Buffer` — the two crates no longer share a cell type — but a cell-by-cell
conversion over the *rendered area only*, in `interop.rs`.

That conversion is cheap because `render_ratatui` was always a scratch-buffer
round trip: cells were already copied in and back out to enforce the clip. The
only change is that the scratch is converted on the way through. A host that
renders no ratatui widgets compiles none of it and takes no dependency.

The conversions are exhaustive `match`es rather than transmutes, and the
modifier bit layout — the one place a silent divergence could hide — is pinned
by a test rather than assumed.

### The cost that was accepted

Every `pub` item that named a ratatui type changed, which is a breaking release
for every host. It was taken as one change rather than two: decoupling the API
without removing the dependency would have broken hosts once, and removing the
dependency later would have broken them again.

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
`KeyedTable` borrows either an ordered row slice or a source that projects
visible indices into authoritative storage. A projected source compares its
row fields directly with the host-owned key, so composite identity needs no
cached key or per-frame allocation; it materializes an owned key only when an
input action changes selection. Indexed cell adapters join parallel metadata
without creating wrapper rows, and all cell work remains inside the resolved
viewport. Temporary absence preserves a key because the component cannot infer
filtering versus deletion; `retain_present` or `retain_present_source` is the
explicit authoritative-delete boundary. A host may supply the selected key's
ephemeral current position to avoid a full lookup scan; the key remains the
persisted identity.

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

Character key codes are logical Unicode text produced by the active keyboard
layout, never physical key positions. Keymap character specs use that exact
text (`A`, `?`, `ctrl+R`) and reject `shift+character`, whose result depends on
layout; Shift remains independently matchable for non-character keys. This
keeps input editing and command bindings on the same layout-neutral identity.

Focus is a registry of scopes rather than a flag per component: a `FocusScope`
claims input ownership for its subtree, so a modal or overlay can take input
without every component learning about modality. At each frame boundary the
host calls `begin_frame`, registers the complete current ring, and then queries
or routes focus. A focused id absent from that ring falls back immediately to
its first registration; the next frame boundary commits that fallback, so a
temporarily removed id cannot steal focus if it later returns.

Focus ownership only *decides* the input target; delivering the event to that
target is [input routing](input-routing.md), which is toolkit-owned for the same
reason. A host that re-derives delivery per event kind eventually derives it
differently for one of them, which is how a paste reaches the surface behind an
open modal.

Text-input positions exposed to hosts remain char indices, matching token and
highlight spans. Editing nevertheless moves and deletes by grapheme boundary,
and soft-wrap plus cursor placement use grapheme display width in terminal
cells. Keeping one cell-width model across measurement, painting, and cursor
math prevents CJK and multi-scalar emoji from drifting apart.

Time-sensitive component state never owns an unreplaceable wall clock. Mouse
double-click detection and the synchronous runner consume the root `Clock`
boundary, defaulting to `SystemClock`; animation frames, toast expiry, and keymap
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
decisions; declaring the lexical rule and returning the spans is the boundary.

`Viewport` follows the same host-state rule as `Scroll`: offsets live in
`ScrollState`, while the view owns only an ephemeral child and declared content
extent. Its scratch buffer covers only the visible source rectangle, even when
the child's logical extent is much larger.

## Rendering pipeline

1. The host builds the view tree from application state (optionally through the
   `view!` DSL, whose conditional and repeated forms still expand to the same
   builder calls — no reconciler or runtime identity).
   `element` preserves frame borrows as `ScopedElement`, so borrowed views may
   appear at any depth in the base tree; `ScopedScene` adds owned overlays.
2. The solver resolves the tree to rects using the frame's active `RenderCtx`.
3. Views paint into a clipped `Surface`; overlays composite over the base.
4. The host calls out-of-band emitters (images, native progress) after the
   frame, because those escapes paint outside the cell model.
5. `Terminal` diffs the buffer against the previous frame and writes only what
   changed.

## Non-goals

- No virtual DOM or retained component identity. Application row keys may be
  used as domain identity by collection state; they do not identify views.
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
