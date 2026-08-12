# Changelog

Notable changes to `tuika`. This file starts at 0.5.0, the first release cut
from this repository.

Versions 0.1.0 through 0.4.0 were published to crates.io from the
[`everruns/yolop`](https://github.com/everruns/yolop) workspace, before tuika was
extracted into its own repository — so this repository holds neither a `vX.Y.Z`
tag nor a GitHub Release for them, and no commit here is the source of any of
those `.crate` files. Their sources remain on
[crates.io](https://crates.io/crates/tuika/versions); the tag and release history
described in the release process begins with the entry below.

## Unreleased

### Added

- `SelectViewportState` couples index selection to a persistent top row for
  `SelectList` and `Table`. Its resolved `VirtualWindow` is shared with mouse
  hit testing, so selection scrolls only across viewport edges and clicks do not
  recenter the list. The existing selection-centered `.viewport(rows)` remains
  available; persistent hosts migrate to `resolve` plus `.visible_window`.
- `TreeList`, `TreeRow`, and `TreeState` provide stable-id expansion, selection,
  keyboard/mouse navigation, ancestor fallback across refreshes, persistent
  scrolling, branch rendering, and a scrollbar over host-provided tree rows.
- `FocusRegistry::focus(id)` lets a `HitMap` focus a registered pane while
  rejecting unknown ids and requests blocked by overlay input ownership.
- `AsyncRunner::{run_with_messages, run_with_events_and_messages}` delivers a
  typed application stream through `AsyncSignal<M>`, including deterministic
  completion/error/redraw/exit behavior without shared mutable state or polling.

- `Runner` and `AsyncRunner` restore drag-to-select behavior by default when
  their terminal session captures the mouse. Selection is painted over the
  final cell frame, releasing copies through OSC 52 on real-terminal runs, and
  wheel events continue to reach application scrolling. Applications can claim
  gestures with `UpdateResult::Consumed` / `Dirty` or opt out with
  `with_text_selection(false)`.
- Hover styling: `mouse::HoverTracker` pairs the pointer-motion stream with the
  existing `HitMap` hit-testing, reporting when the hovered region changes so a
  host can restyle it (and knows a redraw is warranted).
- Timed style transitions: `anim::Transition` is a retargetable eased ramp for
  state-driven motion (hover on/off, focus, expansion) — retargeting mid-flight
  continues from the current value instead of jumping. `anim::lerp` and
  `style::lerp_color` interpolate scalars and 24-bit colors; non-RGB colors
  snap to the nearer endpoint, since an indexed color has no blendable value.
- Gradient color spans: `style::Gradient` is a multi-stop color ramp with
  even (`across`) or explicit (`with_stops`) stops; `Gradient::line` sweeps a
  string's foreground across the ramp per display column (wide glyphs get one
  coherent color) for use with `Text`. `Transition` and `Gradient` (with
  `lerp_color`) are also in the prelude.

### Fixed

- `tuika-mermaid` renders a Mermaid decision node with a `<br/>` label as a
  diagram, with every label line inside the shape. The upstream defect
  ([mmdflux#387](https://github.com/kevinswiber/mmdflux/issues/387)) — which
  unwound out of `View::render` and took the host application down, and failed
  silently in release builds by dropping the label or painting a short one over
  the shape's own borders — is fixed in mmdflux 2.6.1, which `tuika-mermaid`
  now requires. The adapter-side guard that degraded these fences to the
  code-block fallback is gone with it; the unconditional panic containment
  around the layout engine stays.

### Changed

- `Paragraph` now treats its input as human-facing prose: bare `http(s)` URLs
  use the stylesheet's link role and carry OSC 8 targets through wrapping by
  default. `Paragraph::link_policy(LinkPolicy::NONE)` restores literal,
  single-style rendering; `Text`, `Wrap`, `CodeBlock`, and `Console` remain
  inert.
- **Breaking**: `UpdateResult::Clean` now means an input was unhandled and may
  receive runner default behavior; return the new `UpdateResult::Consumed` for
  a handled input that does not need a repaint. Exhaustive matches over
  `UpdateResult` must add the new variant.

- `tuika-mermaid` re-lays out a diagram wider than the available columns at
  progressively tighter node separation until it fits, instead of letting it be
  clipped at the pane's right edge. Fitting is best-effort: a graph with many
  parallel branches can be irreducibly wider than the terminal, and the
  narrowest layout is used there. Diagrams that already fit keep mmdflux's own
  spacing, so existing renders are unchanged.

## [0.8.0] - 2026-08-07

Released alongside `tuika-codeformatters` 0.4.1, `tuika-mermaid` 0.2.1, and
`tuika-html` 0.1.1. Their tuika dependency requirements now track 0.8.

### Highlights

**Turn-key application shell.** `AppShell`, a borrowed `Application` runtime,
completion palettes, dialog presets, and activity lists give hosts a coherent
foundation for responsive tool-style applications without retained view state.

![app shell demo](https://raw.githubusercontent.com/everruns/tuika/v0.8.0/docs/demos/app_shell.png)

**Key-stable collection views.** `SelectionScreen` and `KeyedTable` render
borrowed data while preserving application-key selection across reordering,
filtering, and streaming updates.

![selection screen demo](https://raw.githubusercontent.com/everruns/tuika/v0.8.0/docs/demos/selection_screen.png)

### Added

- `view_fn(measure, render)` defines one-off borrowed views inline with explicit
  intrinsic measurement and allocation-free `Fn` rendering, and composes anywhere
  a normal `View` is accepted.
- `SelectionScreen` composes responsive action, agent, permission, and resume
  pickers from `AppShell`, borrowed selectable rows, semantic heading/rule
  styles, and optional custom chrome while keeping short-height selections
  visible.
- `Application` and `Runner::{run_app, run_app_with_backend}` provide a
  data-driven synchronous runtime whose frame tree can borrow application state
  through `ScopedElement<'_>`; `TestHarness::{render_app, step_app}` exercises
  the same contract without a terminal, and the existing owned-element closure
  API remains.
- `KeyedTable`, `KeyedColumn`, `KeyedSelectState`, and
  `KeyedMultiSelectState` adapt and render only visible borrowed rows while
  preserving application-key selection across reorder, filtering, and
  streaming updates, with scrolling margins, keyboard/mouse navigation,
  responsive columns, aligned styled cells, and leading cursor/check indicators.
- `KeyedRowSource` and `NavigableKeyedRowSource` let `KeyedTable` borrow an
  indirect visible order from authoritative storage, compare computed composite
  identity without per-frame key clones, and materialize a key only when input
  changes selection. Indexed `KeyedColumn` constructors project parallel row
  metadata such as fuzzy-match spans without cached wrapper rows.
- `AppShell` composes a responsive tool-style application frame from borrowed
  or owned header, main, status, and footer views, with optional theme-aware
  rules and short-terminal chrome collapse.
- `CompletionItem`, `CompletionState`, and `CompletionPalette` provide reusable
  fuzzy-ranked command and token completion with host-owned query/selection
  state and explicit replacement text.
- `ConfirmDialog`, `ChoiceDialog`, `MultiChoiceDialog`, and `InputDialog`, with
  paired host-owned state types, provide higher-level modal flows that convert
  into the general `Dialog` builder.
- `ActivityItem`, `ActivityStatus`, and `ActivityList` render multi-step task
  lifecycle state, including optional determinate progress for individual
  steps, without owning scheduling or workflow state.

### Changed

- `SelectList` now further clamps its configured viewport to the height it is
  actually rendered into, keeping a selected row visible after parent chrome
  or a terminal resize reduces the allocation.
- Runner resize signals now force a frame even when application updates return
  `UpdateResult::Clean`; headless harness resize signals also apply their new
  viewport dimensions automatically.

## [0.7.0] - 2026-07-31

Released alongside `tuika-html` 0.1.0, the new companion crate behind the
block-HTML seam, plus `tuika-codeformatters` 0.4.0 and `tuika-mermaid` 0.2.0.
The existing companions adopt tuika 0.7's breaking view-measurement API and
update their tuika dependency requirement in the same release.

### Highlights

**Responsive application primitives.** Docked panels, target-relative overlays,
virtualized collections, semantic styles, and single-line input give hosts a
coherent framework for complex terminal applications without retained UI state.

![primitives demo](https://raw.githubusercontent.com/everruns/tuika/v0.7.0/docs/demos/primitives.gif)

**Rich HTML in Markdown.** Presentational inline HTML now shares Markdown's
styles, while the new parser-free `HtmlBlockRenderer` seam lets `tuika-html`
render styled block HTML without adding a parser to tuika core.

![markdown HTML demo](https://raw.githubusercontent.com/everruns/tuika/v0.7.0/docs/demos/markdown_html.png)

### Added

- Wrapped flex lines with independent row/column gaps, grow and shrink weights,
  min/max constraints, `align_self`, cross-line `AlignContent`, and exact
  boundary-based cell rounding. `FlexItemStyle` separates child properties from
  `LayoutStyle`; `solve_layout` also reports resolved line geometry.
- `Flow` for intrinsic-width wrapping and a deliberately small equal-column
  row-major `Grid` component.
- Extensible `MeasureRequest` / `AvailableSpace` measurement, with known axes
  and definite/min-content/max-content modes. The default adapter preserves
  existing `View::measure` implementations.
- Runtime-neutral `RunnerCore`, configurable `TerminalSessionConfig`, and
  `testing::TestHarness` for state/signal/view application tests without a
  terminal or async runtime.
- `render_once` / `write_once` for ANSI-styled ordinary output, and `view!`
  `when(...)` / `for(... in ...)` composition.

- `Clock` and `SystemClock` provide one monotonic time seam.
  `SelectionState::handle_with_clock` makes double-click gestures deterministic,
  and `Runner::with_clock` lets replayable synchronous hosts own tick time;
  existing `handle` and `Runner::new` behavior remains system-clock backed.
- `VirtualWindow` provides overflow-safe clamped and selection-centered ranges;
  `Scrollbar` renders the same window vertically or horizontally with semantic
  styling and local glyph/style overrides. Scroll, item scroll, viewport,
  select, and table now share those primitives. `SelectList::windowed` and
  `Table::windowed` accept only the visible records while preserving absolute
  selection and full-collection scrollbar geometry.
- `DockState`, `DockSpec`, and `DockLayout` provide a host-owned responsive
  lifecycle for one auxiliary panel: wide panels dock, narrow passive panels
  hide, and focused narrow panels resolve as overlay drawers without introducing
  a retained panel manager.
- Target-relative overlays: `TargetPlacement` selects above/below/left/right,
  cross-axis alignment, gap, and optional edge-aware flipping;
  `OverlaySpec::resolve_target` resolves it directly and
  `SceneOverlay::target` follows a `RectProbe` from the scene root in the same
  frame. Screen-anchored placement remains unchanged.
- `StyleRole` and `StyleResolver` form an open semantic styling seam for hosts
  and companion crates. `RenderCtx::style` resolves built-in or namespaced
  application roles, resolver bundles partially overlay stylesheet defaults,
  and resolver revisions invalidate measurement caches. `paint_with_context`
  and `testing::render_with_context` install and test the complete policy.
- `ScopedElement<'view>`, the boxed frame-borrowed counterpart to owned
  `Element`, for heterogeneous component subtrees that read host state without
  cloning it.
- **Inline HTML in markdown.** The presentational inline tags render instead of
  being dropped: `<b>`/`<strong>`, `<i>`/`<em>`/`<var>`/`<cite>`/`<dfn>`,
  `<code>`/`<kbd>`/`<samp>`/`<tt>`, `<s>`/`<del>`/`<strike>`, `<u>`/`<ins>`,
  `<mark>`, `<a href>`, `<img src alt>`, `<br>`, and `<sub>`/`<sup>`. Each
  resolves the same `StyleSheet` role as the markdown construct it mirrors, so a
  host that restyles `strong` restyles `<b>` with it; `<a>` and `<img>` take the
  existing hyperlink and `ImageResolver` paths. No new dependency and no HTML
  parser: this is a fixed tag whitelist, so anything outside it — block-level
  HTML, `<script>`, unlisted attributes — is dropped as before, and never echoed
  as literal markup.
- **One structured markdown block seam.** `MarkdownBlockRenderer` receives a
  non-exhaustive `MarkdownBlock` descriptor (`Fenced` or `Html`) and a shared
  `MarkdownBlockContext` containing width, theme, and the active stylesheet.
  `Markdown::block_renderer` and `MarkdownState::with_block_renderer` append to
  an ordered renderer chain, so Mermaid, HTML, and host-defined block parsers
  compose without adding another trait or field for every syntax.
- `tuika-html` bounds nesting on the source before parsing, so a fragment deep
  enough to overflow html5ever's recursive tree building is refused rather than
  crashing the host.
- **`markdown::Renderers`** builds the same ordered block-renderer chain for
  `markdown::to_lines_with` / `to_linked_lines_with`.
- `ProgressBar::label` draws a centered, clipped caption over determinate and
  indeterminate bars.
- `Scroll::wrap(true)` reflows owned styled lines at render width before
  applying the scroll window.
- `Table::selection_style` and `SelectList::selection_style` allow per-instance
  selection foreground, background, and modifiers.
- `SelectNavigation` policies for optional j/k, Ctrl+N/P, Tab/Shift+Tab, and
  numeric selection aliases; explicit mouse hit-testing on `SelectState`; and
  `MultiSelectState` for toggleable multiple-selection workflows.
- `KeyHints::from_keymap`, priority-aware whole-hint fitting, and `KeymapHelp`
  so one labeled keymap declaration drives dispatch, responsive footer hints,
  and a complete help view.
- `SingleLineInputState` for search/command fields, with newline normalization,
  Enter/Ctrl+J submission, and allocation-free borrowed text access.
- `tuika::ui` re-exports `Rect`, `Color`, `Style`, `Modifier`, `Line`, and `Span`
  for custom views without a direct `ratatui-core` dependency.
- `testing::render_with_sheet` renders consumer views under an explicit
  stylesheet in the same hermetic buffer harness as `testing::render`.

### Changed

- **Breaking:** `FencedBlockRenderer` and `HtmlBlockRenderer` are replaced by
  `MarkdownBlockRenderer`. Match on `MarkdownBlock`, read width/theme/sheet from
  `MarkdownBlockContext`, register every implementation through
  `block_renderer`, and build free-function chains with
  `Renderers::new().renderer(&first).renderer(&second)`. HTML fences now receive
  the host's active stylesheet instead of synthesizing theme defaults.
- **Breaking:** `LayoutStyle::gap` is split into `row_gap` and `column_gap`
  (the `.gap(...)` builder still sets both), and `Item::dimension` is replaced
  by `Item::style`. Use `Item::new` for the compatible compact path or
  `Item::styled(FlexItemStyle, ...)` for independent flex properties.

- **Breaking:** keymap character specs are now explicitly logical text. Write
  the character produced by the active layout (`A`, `?`, `ctrl+R`) rather than
  `shift+a` or `shift+/`; `Shift` remains valid for non-character chords such as
  `shift+enter`. Ambiguous `shift+character` specs now return `KeyParseError`
  (or panic through static `Layer::bind`) instead of silently discarding Shift.
- **Breaking:** `AsyncRunner` update closures now return `UpdateResult` instead
  of `ControlFlow<()>`, matching synchronous `Runner`; clean ticks and events no
  longer rebuild or repaint the view.

- **Breaking:** interactive component handlers now return one root/prelude
  `InputOutcome` (`Ignored`, `Consumed`, `Changed`, `Submitted`, or `Cancelled`).
  Read the submitted value back from its host-owned state. Replace
  `SelectOutcome`, `MultiSelectOutcome`, `FormOutcome`, `TabSelectOutcome`,
  `TextInputEvent`, and direct `EventFlow` matches with `InputOutcome`; call
  `.flow()` when only propagation matters. `SelectState`, `MultiSelectState`,
  and `ScrollState` now make `Default` identical to `new()`; use
  `SelectState::unselected()` for an initially cursorless list.
- **Breaking:** `StyleSheet` adds typed toast, diff, and key-hint fields. Toasts,
  diffs, `KeyHints`, and `KeymapHelp` now derive their defaults from those roles
  instead of renderer literals; explicit `Diff::style` colors still win for one
  instance. Exhaustive `StyleSheet` literals must add the new fields or use
  `..StyleSheet::from_theme(&theme)`. Construct `RenderCtx` through
  `RenderCtx::new` rather than a struct literal now that it carries the optional
  resolver.
- **Breaking:** `View::measure` now takes `&RenderCtx`, and every composition
  container forwards the frame's active theme, stylesheet, and focus state.
  Migrate `fn measure(&self, available: Size)` implementations to
  `fn measure(&self, available: Size, ctx: &RenderCtx)`; callers likewise pass
  the context used for rendering. `Flex::solve` now takes that context, and
  `ItemScroll::{measure_height, measure_views}` take it as their final argument.
- `Markdown` and the companion `tuika-html::Html` view now measure with the same
  theme and stylesheet they render with. `Boxed` implements stylesheet panel
  padding as real layout; an explicit `.padding(...)` wins over the stylesheet.
- `element`, `view!`, and composition containers now preserve borrowed child
  views at any depth. Existing owned trees continue to use `Element`; borrowed
  trees use the same builders and are bounded to their frame lifetime.
- `Flex` measures padded children against their actual inner box and reports
  fixed/percent child dimensions when the container itself is auto-sized, so
  nested measurement matches the rects assigned during rendering.
- `TextInputState` preserves grapheme clusters during cursor motion and
  deletion. Text input wrapping, rendering, and terminal cursor placement now
  share terminal-cell width, fixing CJK and multi-scalar emoji alignment while
  keeping public cursor and span coordinates as char indices.
- `FocusRegistry` immediately falls back to the first current registration when
  a focused id disappears from a dynamic frame, and commits that fallback at
  the next frame boundary instead of retaining or resurrecting a stale target.
- **Breaking:** synchronous `Runner::run` and `run_with_backend` now mirror the
  state/view/update shape of `AsyncRunner`: pass `&mut State`, render through
  `view(&State, frame)`, and handle `Signal` in `update(&mut State, Signal)`.
  Updates return `UpdateResult::{Clean, Dirty, Exit}` instead of
  `ControlFlow<()>`; ticks no longer repaint unless the update is dirty or a
  `RedrawHandle` fires.

- **Breaking**: `SelectState::selected()` now returns `Option<usize>`, and
  `SelectState::select` takes `Option<usize>`, so lists and tables can render
  without a selected row. Migrate `state.select(index)` to
  `state.select(Some(index))`; use `state.select(None)` or
  `SelectState::default()` for no selection. `SelectState::new()` still selects
  the first row.
- **Markdown output changes where the source contains inline HTML.** Text inside
  the whitelisted tags is now styled, `<br>` starts a new line, `<img>` becomes
  an image or an alt-text placeholder, and `<sub>`/`<sup>` digits become Unicode
  (`H<sub>2</sub>O` → `H₂O`). Markdown without HTML renders identically.
- Consecutive blank lines no longer appear in markdown output when a block
  renders nothing (a block-HTML run with no renderer attached), so a dropped
  block leaves no gap where it was.
- New gallery demo for inline HTML (`docs/demos/markdown_html.png`), referenced
  from the component gallery, the markdown guide, and `Markdown`'s rustdoc.
- `tuika-html` gains an example and a recording for the `Html` *component*
  (`cargo run -p tuika-html --example html_view`); the existing example covers
  the markdown seam. `<sub>`/`<sup>` now transliterate there too, so one
  document cannot render `H₂O` through markdown and `H2O` through the crate,
  and `<dd>` hangs directly under its `<dt>` instead of a blank line below.
- `Table` now windows rows to its assigned render height by default.
  `Table::viewport(rows)` remains an optional upper bound.
- ratatui `Line` styles are composed underneath their `Span` styles in text,
  table cells, scrolling text, and box titles.
- `Boxed` titles start directly after the corner and truncate at the opposite
  corner, matching ratatui `Block` title placement.

## [0.6.0] - 2026-07-25

Released alongside `tuika-codeformatters` 0.3.1 and `tuika-mermaid` 0.1.1,
which update their tuika dependency requirement for 0.6 compatibility.

### Highlights

**Split-footer terminal mode.** Hosts can keep a live footer pinned to the bottom
of the terminal while completed content moves into native scrollback, then return
every reserved row cleanly on exit.

![split-footer demo](https://raw.githubusercontent.com/everruns/tuika/v0.6.0/docs/demos/split-footer.svg)

**Borrowed scene roots.** `ScopedScene` renders and dispatches events through a
borrowed `View`, so hosts can keep application state outside the scene without
requiring `'static` ownership.


### Added

- **Borrowed scene roots.** `ScopedScene<'_, V>` borrows a concrete `View` for
  one frame while owning the same ordered `SceneOverlay` / `Dialog` stack as
  `Scene`. Hosts can paint large live models directly without cloning them into
  a `'static` `Element`; rendering, backdrop, placement, and focus-owner
  semantics are shared with owned scenes.
- **Screen modes.** `ScreenMode` picks which part of the terminal a frame owns:
  `Alternate` (the previous, still-default behavior) or `split_footer(rows)`,
  which reserves rows at the bottom of the *main* screen and leaves everything
  above as the terminal's own scrollback — the shell prompt, the wheel, mouse
  selection, and the output the app publishes, which survives its exit.
  `RunnerConfig::screen_mode` drives both runners; a host with its own loop
  composes `TerminalSession::enter_with`, `screen::pin_footer`, and
  `screen::close_footer`.
- **Publishing above a footer.** `Runner::scrollback()` /
  `AsyncRunner::scrollback()` return a cloneable, `Send + Sync` `Scrollback`
  queue of views the loop commits above the footer; `screen::publish_block`
  commits one view straight from a host's own loop, with no `Send` bound.
- New `split_footer` example, and `cargo run --example codex -- --split-footer`
  runs the whole coding-agent UI in the mode.
- New `scrolling-regions` feature. It is a compatibility mirror of ratatui's
  (Cargo unifies features one way, and `HyperlinkBackend` must still implement
  `Backend`), *not* an optimization: rows scrolled out of a DECSTBM region are
  discarded by the terminal instead of entering its scrollback.

### Changed

- **Breaking**: `RunnerConfig` gains a `screen_mode` field. Struct literals need
  a default update:

  Before:

  ```rust
  RunnerConfig { tick_rate }
  ```

  After:

  ```rust
  RunnerConfig { tick_rate, ..RunnerConfig::default() }
  ```
- `ScreenMode` and `Scrollback` join the crate root and the prelude.

## [0.5.0] - 2026-07-25

The first release cut from this repository. Companion crates released alongside
it: `tuika-codeformatters` 0.3.0 and `tuika-mermaid` 0.1.0 (its first release).

### Highlights

**The crate root is now a decision instead of an accumulation.** `tuika::` had
grown to 30 flat public modules plus 167 names re-exported to the root, so
almost every type had two equally valid paths and neither was canonical. Four
levels now each have one job, and where a new item goes is a rule rather than a
preference:

| Path | Holds |
| --- | --- |
| `tuika::` | the framework spine — `View`, `Element`, `RenderCtx`, layout, events, `Theme`, `Surface`, the host seam |
| `tuika::components` | every widget |
| `tuika::term` | everything out-of-band: `clipboard`, `hyperlink`, `progress`, `pointer`, `image`, `capabilities`, `palette` |
| `tuika::prelude` | the spine and the components in one glob import |

- **New**: `tuika::prelude` — `use tuika::prelude::*;` replaces most import
  blocks outright, which is the intended migration for application code.
- No behavior change: this release moves and renames public items only. Every
  test, snapshot, and benchmark passes unchanged apart from its imports, and
  `tests/public_api.rs` now pins the layout from outside the crate, the way a
  host sees it.

**A theme can be inherited from the terminal.** An application can adopt the
palette the user already configured instead of imposing its own — opt-in and
host-initiated, so nothing changes for an app that does not ask.

- **New**: `themes::TERMINAL` (also `Theme::terminal()`, and a `terminal` entry
  in `themes::PRESETS`) — a `const Theme` whose every slot is `Color::Reset` or a
  `Color::Indexed` ANSI slot, so the terminal resolves the palette. No query, no
  timeout, no failure mode.
- **New**: `tuika::term::palette` — `TerminalPalette` with `parse`/`query`, plus
  `QUERY_FOREGROUND`, `QUERY_BACKGROUND`, and `query_sequence()`. Asks the
  terminal for its colors with the xterm queries (OSC 10 / 11 / 4), fenced by the
  Device Attributes request so an unsupported query costs a round-trip rather
  than a timeout.
- **New**: `Theme::from_terminal(&TerminalPalette)` derives a full theme from the
  reply — reported foreground and background verbatim, in-between tones blended
  and contrast-guarded, hues from the ANSI palette.
- **New**: `Capabilities::query_with_palette(timeout)` answers "what can this
  terminal do" and "what colors is it using" in one round-trip.
- **New example**: `cargo run --example inherit` (and `-- --probe` to print what
  your terminal answers without taking over the screen).

The palette work is additive only — nothing moved or was renamed by it.

**A dialog, a form, and a scrollable viewport are no longer every host's
homework.** Three patterns every application rebuilt by hand are components now,
and a `Scene` owns the base tree plus its overlays so focus and compositing stop
being hand-wired at the call site.

![primitives demo](https://raw.githubusercontent.com/everruns/tuika/v0.5.0/docs/demos/primitives.gif)

- **New**: `components::Dialog` — a titled, bordered, optionally backdrop-dimmed
  panel with key hints and an action row, placed by an `OverlaySpec`.
- **New**: `components::{Form, FormField, FormState, FormOutcome}` — labelled
  fields with help and error rows, `Tab`/`Shift+Tab` focus, and a responsive
  `stack_below(width)` breakpoint.
- **New**: `components::Viewport` — a clipping, panning window over an
  oversized child, with optional scrollbars on both axes.
- **New**: `Scene`, `SceneOverlay`, `Backdrop`, and `paint_scene` — one value
  carrying the root and its overlay stack, with `sync_focus` for the registry.
- **New**: `DrawView` (alias `CanvasView`) — a `View` from a closure, for
  one-off custom painting without declaring a type.
- **New**: `SemanticRole` and `Theme::{semantic_color, semantic_style,
  success_style, warning_style, danger_style, info_style}` — success / warning /
  danger / info resolved from the theme instead of hardcoded per host.

**Markdown fenced blocks are extensible.** A fence with an unknown language used
to render as code and nothing else. A host can now claim any info string and
paint the block itself — which is how Mermaid diagrams became terminal-native,
without tuika taking on a diagram engine.

![mermaid demo](https://raw.githubusercontent.com/everruns/tuika/v0.5.0/crates/tuika-mermaid/examples/mermaid_markdown/mermaid.gif)

- **New**: `components::markdown::FencedBlockRenderer` — the seam, plus
  `Markdown::block_renderer`, `MarkdownState::with_block_renderer`, and
  `markdown::{to_lines_with_renderer, to_linked_lines_with_renderer}`.
- **New crate**: [`tuika-mermaid`](https://crates.io/crates/tuika-mermaid) —
  `MermaidRenderer`, an mmdflux-backed implementation for ```` ```mermaid ````
  fences. Diagram layout stays out of tuika core.

**Long transcripts scroll by item, not by line.** `ItemScroll` scrolls a list of
laid-out elements — the shape a chat log or an agent transcript actually has —
and the text input grew the seams a composer needs.

- **New**: `components::ItemScroll` — item-granular scrolling with `windowed`
  construction, `gap`, `scrollbar`, and `measure_height`.
- **New**: `components::textinput::{Trigger, TriggerAnchor, Token, TextSpan}`,
  plus `TextInputState::{tokens, active_token, replace_token}` and
  `TextInput::{highlights, placeholder}` — `@`/`/` mention and slash-command
  tokens, and styled spans over the edited text.
- **New example**: `cargo run --example codex` — a replica of the Codex CLI's UI
  built entirely from tuika components.

### Breaking Changes

Most application code migrates by replacing its `use tuika::{…};` block with
`use tuika::prelude::*;`. The tables below cover what the prelude does not
carry.

**Modules moved**

- **Out-of-band escapes are one family**: they shared a shape but had three
  unrelated names, so a reader had to learn each separately.
  - Before: `tuika::clipboard`, `tuika::hyperlink`, `tuika::native`, `tuika::capabilities`
  - After: `tuika::term::{clipboard, hyperlink, progress, pointer, capabilities}`
- **Images split along the cell boundary**: the protocol half talks to the
  terminal, the view half is a component like any other.
  - Before: `tuika::image::{Image, ImageData, ImageLayer, ImageSupport}`
  - After: `tuika::components::Image` and `tuika::term::image::{ImageData, ImageLayer, ImageSupport}`
- **Markdown is a component**, and lives where the other components live.
  - Before: `tuika::markdown`
  - After: `tuika::components::markdown`
- **One runner module**: a `cfg` is an implementation detail, not a second entry
  in the module list.
  - Before: `tuika::async_runner::{AsyncRunner, Signal}`
  - After: `tuika::runner::{Runner, RunnerConfig, AsyncRunner, Signal}`
- **Ratatui interop is named for the seam, not for its only type.**
  - Before: `tuika::ratatui_view::RatatuiView`
  - After: `tuika::interop::RatatuiView`

**Items renamed**

- **The OSC encoders share one shape** — a pure `encode`, a thin writer.
  - Before: `osc52`, `write_clipboard`, `osc8`, `osc8_with`, `encode_pointer_shape`, `write_pointer_shape`
  - After: `term::clipboard::{encode, write}`, `term::hyperlink::{encode, encode_with}`, `term::pointer::{encode, write}`
- **`tuika::highlight` was a module and a function at once.** The module is the
  `Highlighter` seam; the function paints a selection.
  - Before: `tuika::highlight(buffer, area, range, style)`
  - After: `tuika::mouse::paint_selection(buffer, area, range, style)`
- **Hand-prefixed names get their module back.** The prefixes existed only to
  disambiguate a flat root.
  - Before: `markdown_to_lines`, `markdown_to_linked_lines`, `diff_rows`, `qr_encode`, `wrap_lines`
  - After: `components::markdown::{to_lines, to_linked_lines}`, `components::diff::rows`, `components::qr::encode`, `components::text::wrap_lines`
  - Before: `ASCII_FONT_HEIGHT`, `CONSOLE_DEFAULT_CAPACITY`, `TOAST_DEFAULT_TTL`
  - After: `components::ascii_font::FONT_HEIGHT`, `components::console::DEFAULT_CAPACITY`, `components::toast::DEFAULT_TTL`
- **`Overlay` sits beside `OverlaySpec`**, since resolving a spec to a rect and
  pairing that rect with a view are two halves of one pipeline.
  - Before: `tuika::host::Overlay`
  - After: `tuika::overlay::Overlay` (still re-exported as `tuika::Overlay`)

**Root re-exports removed**

Components and the per-module surface (`anim`, `focus`, `framebuffer`,
`highlight`, `keymap`, `live`, `mouse`, `probe`, `themes`, `width`, and the
styling extras) are no longer flattened to `tuika::`. Reach them through
`tuika::prelude::*` or their module path.

### Fixed

- **docs.rs builds the crate again.** 0.4.0 documented nothing on docs.rs:
  `src/lib.rs` gated on `feature(doc_auto_cfg)`, which was merged into
  `doc_cfg` and removed as a name in Rust 1.92, so rustdoc failed outright on
  docs.rs's nightly. Nothing else saw it — the attribute compiles only under
  `--cfg docsrs`, which no local or CI build set. CI now rehearses docs.rs's
  own invocation (nightly, `--cfg docsrs`) alongside the consumer-facing one.
- **`Shift+Enter` reaches the text input as its own chord.** `TerminalSession`
  now enables and restores enhanced keyboard reporting, so the chord arrives at
  `TextInputState` distinctly instead of being decoded as plain `Enter` — the
  difference between "insert a newline" and "submit". The negotiation handles
  iTerm2 and tmux's xterm and CSI-u formats; Windows keeps using modifier-aware
  native console events.
- **Markdown: a block inside a tight list item no longer swallows the item's
  text.** A tight list item carries no `Paragraph` of its own, so a nested list,
  block quote, or code fence opened while the parent item's text was still
  buffered — rendering `- outer` / `  - inner` as a single `• outerinner` line,
  and placing a fence *ahead* of the item it follows.
- **Markdown: streaming no longer splits a block on a half-arrived indent.**
  `MarkdownState` settles its prefix at the last blank line, and mid-stream a
  nested item's indent arrives as a whitespace-only *unterminated* line — which
  looked blank. The prefix was committed there, permanently cutting a list in
  two so the halves re-parsed as unrelated top-level lists. Only a
  newline-terminated blank line settles the prefix now, so a streamed render
  matches the one-shot render character-for-character.
- **Markdown: `- [ ]` / `- [x]` task lists render as checkboxes.** The renderer
  had the checkbox handler and a `task_marker` stylesheet slot, but the parser
  option that emits the event was never enabled, so both were unreachable and
  the markers rendered as literal text.

  Together these cost ~3% instructions on the markdown render benches — the old
  counts were cheap because a nested item's line was being dropped rather than
  laid out — which the committed `benches/iai-baseline.json` absorbs unchanged.

### What's Changed

* fix(packaging): trim the companion crates and correct the release history (#12)
* feat(markdown): render GFM task-list checkboxes as themed markers
* fix(markdown): flush the pending item line before a nested block opens
* fix(markdown): never settle the streaming prefix on an unterminated blank line
* docs(markdown): add `docs/markdown.md` and a `markdown_table` demo scene
* chore(ci): cover the workspace on the macOS and Windows legs (#15)
* feat(markdown): render extensible fenced blocks through `FencedBlockRenderer`, and add the `tuika-mermaid` companion crate
* fix(term): enable modified-key reporting so `Shift+Enter` arrives as its own chord (#13)
* docs(example): follow the stream in the markdown example until the reader scrolls back (#11)
* refactor(markdown): split the module along its parse/flatten passes (#9)
* fix(docs): preserve demo colors during recording (#10)
* feat(themes): inherit the terminal's palette (#8)
* feat(components): add `Dialog`, `Form`, `Viewport`, `Scene`, and `DrawView` (#7)
* refactor: give the crate root, components, and term one job each (#6)
* refactor(term): group the out-of-band escapes under one module
* refactor(components): move markdown and the image view in with the components
* refactor: fold `async_runner` into `runner` and rename `ratatui_view` to `interop`
* refactor(tests): move the crate's test scaffolding under `src/tests`
* test: pin the public module layout from outside the crate (`tests/public_api.rs`)
* docs: add `knowledge/specs/api-surface.md` and a crate-layout section to the README
* chore(knowledge): split out process concepts and enforce upkeep (#5)
* feat(components): add `ItemScroll` and the composer token seams, plus the `codex` example
* docs: record the showcases at gallery pixel density and point yolop at its product page
* fix(docs): stop the demo recordings clipping their own scenes
* docs: add a showcases page with yolop and LLMSim demos (#3)
* chore(release): show demos in the changelog highlights and drop commit links
* chore: require signed commits, Doppler-managed secrets, and PRs for external contributions
* fix(ci): green the pipeline after the yolop extraction
* docs: add the knowledge bundle and agent workflows
* ci: add the build, documentation, release, and cross-terminal pipelines
* test: add a PTY smoke test and guard the published crate contents

[0.6.0]: https://github.com/everruns/tuika/releases/tag/v0.6.0
