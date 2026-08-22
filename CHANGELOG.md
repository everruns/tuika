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

## [Unreleased]

### Breaking Changes

- **tuika no longer depends on ratatui.** It now owns its own `Rect`/`Position`,
  `Color`/`Modifier`/`Style`, `Line`/`Span`, `Buffer`/`Cell`, `Backend`, and
  `Terminal`. Every one of those types moved from `ratatui-core` to tuika, so
  any signature naming one changes.

  For most hosts the migration is an import change — the canonical path is
  unchanged:

  ```rust
  // before
  use ratatui_core::layout::Rect;
  use ratatui_core::style::{Color, Modifier, Style};
  use ratatui_core::text::{Line, Span};

  // after (or just `use tuika::prelude::*;`)
  use tuika::ui::{Color, Line, Modifier, Rect, Span, Style};
  ```

  A host driving its own terminal takes `tuika::term::terminal::{Terminal,
  TerminalOptions, Viewport}` and `tuika::term::backend::CrosstermBackend`
  instead of ratatui's, and `tuika::term::testbackend::TestBackend` in tests.

  The types are deliberately shaped like the ones they replace, with two
  behavioral notes:
  - `Span::width` and `Line::width` count grapheme-aware display columns
    (`width::str_cols`) rather than per-`char` widths, so they now agree with
    what `Surface` actually paints. A line containing a ZWJ emoji measures
    narrower than before — and correctly.
  - `Cell` stores short grapheme clusters inline instead of in a
    `CompactString`. `Cell::new` is no longer `const`; use `Cell::default()`
    for a blank cell.

- **Ratatui interoperability is now behind the `ratatui` feature**, off by
  default. `Surface::render_ratatui`, `tuika::interop`, and `RatatuiView` need
  it; a host wrapping ratatui widgets adds:

  ```toml
  tuika = { version = "0.11", features = ["ratatui"] }
  ```

  Every ratatui widget still works. The boundary is no longer a shared `Buffer`
  — the two crates no longer share a cell type — but a cell-by-cell conversion
  over the rendered area, which `render_ratatui` was already doing a copy for.

- **`Surface::render_ratatui` was split.** The general escape hatch — a private
  scratch buffer, composited back through the clip — is now
  `Surface::render_scratch` and takes tuika's `Buffer`. `render_ratatui` is the
  ratatui-specific wrapper around it. A caller using it for scratch access
  rather than for ratatui widgets should switch to `render_scratch`, which needs
  no feature.

- **`Line::raw` and `Line::styled` split their content on line breaks**, one
  span per line, matching what they replaced. `Line::raw("")` therefore yields a
  line with no spans.

### Added

- `Surface::render_scratch` — clipped scratch-buffer access without ratatui.
- `Buffer::set_style`, `Buffer::content()`, `Cell::raw_symbol()` (tells a wide
  grapheme's placeholder apart from a blank cell).
- `framebuffer::QUADRANTS` — the sixteen quadrant block glyphs, previously
  reached through `ratatui_core::symbols::pixel`.
- `Style::{bold, dim, italic, underlined, reversed, crossed_out}` shorthands.
- `tuika::term::backend::CrosstermBackend` and
  `tuika::term::testbackend::TestBackend` are public, for a host driving
  `Terminal` itself.


- `mouse::SelectionState::confine` / `region` — restrict a gesture to a rect and
  read it back, for hosts driving the selection primitives from their own loop.

### Changed

- **Dependencies: 55 crates to 32.** Dropping `ratatui-core` removed 23,
  including `kasuari` (a Cassowary solver), `lru`, and a second `hashbrown` —
  all of which existed for `ratatui_core::layout::Layout`, which tuika replaced
  with its own flex solver and never called — plus `strum`, `compact_str`,
  `itertools`, `unicode-truncate`, and a second `syn` major version. The cold
  dependency build drops from roughly 32s to 6s.
- **Rendering is ~7% cheaper.** `render`, `frame_windowed`, and the scroll paths
  fall 7.3–7.4% in instruction count, because a `Cell` now stores short
  grapheme clusters inline. The markdown paths rise ~2%, the cost of measuring
  widths grapheme-aware; the benchmark baseline is updated accordingly.
- `scrolling-regions` is now a feature of tuika's own `Backend` trait, and no
  longer pulls in `ratatui-crossterm` to forward a flag.

- **Runner drag selection is per panel.** A left drag that starts inside a
  bordered `Boxed` selects only within that panel: positions clamp to its
  inner rect, a multi-row selection wraps at the panel's edges instead of the
  screen's, and the copied text never picks up the pane beside it. A drag that
  starts outside every panel still spans the screen as before.

### Fixed

- A `SelectionRange` that reaches past the `area` handed to
  `mouse::paint_selection`, `mouse::selected_text`, or `SelectionRange::contains`
  now stops at that area's rows instead of indexing the buffer out of bounds.

## [0.11.0] - 2026-08-22

Released alongside `tuika-charts` 0.1.2, `tuika-codeformatters` 0.5.0,
`tuika-html` 0.1.4, and `tuika-mermaid` 0.3.2. Their tuika dependency
requirements now track 0.11.

### Highlights

**One route for every event** — a host registers each surface once and every
`Event` kind (key, paste, mouse, focus) follows the same path to whichever
surface owns input this frame, so a paste can no longer take a different route
from a key and land behind an open overlay. `Router` resolves stages in a fixed
order and `Delivery` reports which surface received what, including the case
where nothing did.

**Streaming markdown renders identically to a one-shot render** — two cache
boundaries that only misfired at particular chunk splits are fixed, so a
document fed in as it arrives renders exactly like the same document rendered
whole.

![markdown demo](https://raw.githubusercontent.com/everruns/tuika/v0.11.0/docs/demos/markdown.gif)

- **Binary size**: a default `tuika-codeformatters` build measured ~24.0 MiB on
  a highlighting probe binary; a `rust` + `python` build is ~4.8 MiB. A tuika
  host that places no images drops ~8.4 KiB of graphics encoders.
- **Dependencies**: 66 crates to 55 (the `async` graph, 71 to 59).

### Breaking Changes


- **`tuika-codeformatters`**: every tree-sitter grammar now sits behind its own
  cargo feature. All fourteen are **on by default**, so a dependant taking the
  crate normally is unaffected. A dependant that already passes
  `default-features = false` now gets no grammars until it names them:

  ```toml
  tuika-codeformatters = { version = "0.4", default-features = false, features = ["rust", "python"] }
  ```

  Feature names are the language keys lowercased without punctuation — `rust`,
  `python`, `typescript` (covers TSX/JSX), `go`, `java`, `ruby`, `css`, `html`,
  `csharp`, `php`, `zig`, `scala`, `sql`. A language whose feature is off
  behaves exactly like one the crate never supported: `highlight` returns
  `None` and the caller renders plain code.

- **`Paragraph` and dialog copy wrap differently.** Both now use tuika's own
  wrap solver — the one `Wrap`/`wrap_lines` has always used — instead of
  `textwrap`. Three visible differences, all of them making `Paragraph` agree
  with the rest of the toolkit:
  - Line breaks are chosen **greedily** (first-fit) rather than by textwrap's
    optimal-fit minimum-raggedness pass, so a paragraph's rows may break at
    different words than in 0.10.
  - A run of whitespace between two words is one break opportunity and renders
    as a single space, instead of being copied through verbatim.
  - Breaks happen only at whitespace. textwrap also broke at Unicode line-break
    opportunities such as `/`, which could split a linkified URL after its
    scheme; a URL now stays on one row unless it is wider than the row.

  Widths are counted in tuika's grapheme-aware display columns, which fixes a
  latent bug: `Paragraph` wrapped with textwrap's per-`char` width model and
  then measured the result with `str_cols`, so text containing a ZWJ emoji
  (a family, a flag, a skin-tone sequence) wrapped earlier than its own
  measurement said it needed to.

### Changed

- **`tuika-codeformatters` binary size**: grammars are selected by a runtime
  language string, so the config builder named all fourteen and anchored every
  parse table into any binary that highlighted anything — roughly 21 MiB of
  read-only data, C# alone about 5 MiB. Selecting grammars by feature lets a
  host drop the ones it does not use: measured on a probe binary highlighting a
  single snippet, the default build is ~24.0 MiB and a `rust` + `python` build
  ~4.8 MiB. Highlighting output under default features is unchanged.
- **Graphics protocol encoders are now pay-for-use.** `ImageLayer::emit` runs
  after every frame in every `Runner` host and chose its encoder by matching on
  `ImageSupport`, which linked the Kitty, iTerm2, and Sixel encoders plus the
  PNG writer into every binary that used tuika at all. The encoder is now
  resolved where an image is recorded — reachable only from the `Image` view —
  so a host that places no images no longer carries any of them: about 8.4 KiB
  less tuika code in a minimal host. The emitted bytes are unchanged for all
  three protocols, and no public API changed.

- **Dependencies: 66 crates to 55.** Three runtime dependencies were removed by
  taking work tuika was already half-doing:
  - `textwrap` (with `smawk` and `unicode-linebreak`) — replaced by the wrap
    solver `Wrap` already used. See **Breaking Changes** for the visible
    difference.
  - `ratatui-crossterm` (with the `instability`/`darling` proc-macro tree behind
    it) — tuika now writes to the terminal through its own crossterm `Backend`.
    It already implemented that whole trait once, in `HyperlinkBackend`, to wrap
    the one being removed. The emitted byte stream is held **byte-for-byte
    identical** by a differential test that draws the same cells through both
    backends. The crate remains an *optional* dependency of the
    `scrolling-regions` feature, which forwards its matching flag so a host's
    own copy still compiles.
  - `tokio-stream` (under `async`) — replaced by `futures-core`, which the
    build already contained: `async` enables `crossterm/event-stream`, and that
    pulls it in. `tokio_stream::Stream` *is* `futures_core::Stream`, so a host
    passing a `ReceiverStream` is unaffected; `tokio-stream` stays a
    dev-dependency and the doc example still uses it. The `async` graph goes
    from 71 crates to 59.

  No public API changed, and no rendered output changed outside the wrapping
  differences above.

### Added

- **Input routing**: the new `routing` module (`Router`, `Delivery`,
  `RouteStage`, `InputTarget`, all re-exported from the crate root and the
  prelude) delivers an event to the surface that owns input this frame. A host
  registers each surface once and every `Event` kind follows the same route, so
  a paste can no longer take a different path from a key and land behind an open
  overlay. `Router` reads the `FocusRegistry` an overlay-bearing `Scene` already
  synchronized — and does not hold it, so a stage closure may take `&mut` on the
  host — resolves stages in a fixed order (global chord, declared exceptions,
  the active surface, last-chance global), and reports through `Delivery` which
  surface received what, including the case where nothing did. See the
  [routing guide](docs/routing.md).

- **`term::hyperlink::apply_buffer_links_in`**: the area-clipped counterpart to
  `apply_buffer_links`, for a component embedding OSC 8 runs into a sub-rect of
  a shared buffer. `apply_buffer_links` is unchanged and still clips to the
  whole buffer.

- **Fuzz and black-box robustness suites** (`src/tests/fuzz.rs`,
  `tests/robustness.rs`): adversarial text and event streams through the wrap
  solver, composed view trees, the stateful components, and the parsers that
  read untrusted terminal replies; plus a sweep of every component against every
  corpus at every degenerate size, asserting no panic, no paint outside the
  component's own rect, and no control byte in a cell. Both fixes below came out
  of them. Test-only — no API change.

- **Session stress suite** (`tests/stress_ui.rs`): every screen mode driven
  through `Runner` and `AsyncRunner` over an in-memory screen that resizes
  between frames, plus mode changes mid-session, adversarial scrollback
  publishing, and shell/overlay/dock composition at degenerate sizes. The
  split-footer fix below came out of it. Test-only — no API change.

### Fixed

- **A split footer is pinned and published at the terminal's current size.**
  Both runners write the footer's reserved rows, and render each queued
  scrollback block, at the geometry the `Terminal` last observed — and neither
  observed a resize before the loop's first frame or before draining the
  publish queue. Two consequences, both on the ordinary path: a window resized
  between the terminal being constructed and the first frame pinned the footer
  at the old size, and a block published in response to a resize (a host
  logging the new dimensions, a status line reflowing) was committed to the
  scrollback at the previous width. Both loops now learn the size first.

- **A component no longer stamps hyperlink markers outside its own rect.**
  `Paragraph` and `Markdown` embed OSC 8 runs into the cells of a linked label
  after painting, and the placement was clipped to the *buffer* rather than to
  the rect the component was laid out in. A markdown block whose links ran past
  the rows or columns it was given wrote an escape onto a neighbouring
  component's cell — visible as a stray link, or an orphaned escape once the
  neighbour repainted. Placements outside the area are now dropped.

- **Streamed markdown renders identically to a one-shot render.** The settled
  prefix is cached at the last blank line that ends a block, and two constructs
  were mistaken for one:
  - A fence line carrying an info string (`` ```rust `` inside an already-open
    block) was treated as a closer, so a later blank line settled a boundary
    *inside* a code block; everything after it rendered as markdown while a
    re-render of the same source rendered it as code.
  - A blank line between list items settled the prefix, so the rest of the list
    was parsed in isolation and its numbering restarted — a streamed
    `1. … 2. … 3. …` rendered as `1. … 1. … 1. …`. A blank line inside a list is
    a boundary only once a following top-level block proves the list ended, and
    never on the strength of an in-flight partial line (`1` before it becomes
    `1.`).

  Both were found by a new streaming/one-shot fuzz differential, and both
  depended on where the stream's chunk boundaries happened to fall.

- **A link's underline stops at the URL.** When markdown wrapped a run of spans,
  the space it re-inserted between two words inherited the style and OSC 8
  target of the word *before* it, so the underline and the link's hit area ran
  one cell past a `[label](url)` or a bare URL into the text that followed. The
  separator now carries the style of the source whitespace it stands for, and a
  boundary with no whitespace behind it no longer has a space invented for it.

## [0.10.0] - 2026-08-17

Released alongside `tuika-charts` 0.1.1, `tuika-codeformatters` 0.4.3,
`tuika-html` 0.1.3, and `tuika-mermaid` 0.3.1. Their tuika dependency
requirements now track 0.10.

### Added

- **Components**: `ProgressBar::label_style` styles the caption set by
  `label`, the same theme-by-default/explicit-override pattern as `colors`. The
  trailing `NN%` of `percent` stays separate chrome on the theme's muted style.
- **Components**: `SelectionAnchor` selects how `Table` and `SelectList` window
  themselves around the selection when they resolve their own window —
  `Center` (today's recentering behavior, still the default) or `Edge`, which
  moves the window only when the selection would leave it.
- **Virtualization**: `VirtualWindow::keeping` is the edge-following policy as a
  pure function of the caller's current start; `SelectViewportState::resolve`
  now uses it, so the persistent and stateless paths share one implementation.
- **Charts**: both axes carry tick labels, so a chart states its scale instead
  of leaving the reader to guess it. Ticks land on round values (a power of ten
  times 1, 2, 2.5, or 5) and label precision follows the tick step.
  `Axis::format` replaces the numeric formatter, `Axis::ticks` targets a tick
  count, and `Axis::hidden` opts an axis out. Labels are drawn as cells in
  **both** render modes — the graphics image covers the plot only — so the same
  numbers land in the same cells whether or not the terminal has graphics.

### Changed

- **Breaking:** Mouse capture is now opt-in in alternate-screen mode, preserving native OSC 8
  link activation, terminal selection, and scrolling by default. Applications
  that need pointer or wheel events use `ScreenMode::with_mouse_capture`,
  `MouseCapture::Enabled`, or `AltScreen::enter_with_mouse_capture`; captured
  sessions may use runner selection and `ctrl_click_url` as application-side
  fallbacks.
- `MarkdownState::links` preserves labeled and bare hyperlink targets across
  incremental updates, settled-prefix caching, wrapping, and resize, so hosts
  drawing streaming lines can apply native OSC 8 links without re-parsing.
- **Charts**: `Axis::categories` names positions instead of measuring them,
  which is what most bar and area charts actually want. `Chart::stack` combines
  bar and area series — `Stack::Normal` sums them, `Stack::Percent` scales every
  position to 100 — while unstacked bar series split their category band into
  slots and sit side by side. `Chart::horizontal` swaps the axes so a category
  gets a whole row of width for its name. `Series::markers` and `Series::labels`
  annotate samples, `Chart::focus` marks one position and lists every series'
  value there, and `Series::donut` with `Chart::center` draws a ring. A rule is
  drawn along zero whenever the value domain straddles it.
- **Breaking:** chart axis labels are on by default, so a chart that previously
  filled its whole area now gives a few columns to the y-axis gutter.
  `Chart::x_axis` and `Chart::y_axis` with `Axis::hidden` restore the old
  geometry exactly; the x labels reuse the margin row the plot already reserved
  and cost no plot height. Aligning both renderers on one cell grid also removed
  the graphics path's internal margins, so a graphics plot now covers the same
  data cells its portable counterpart does.

### Fixed

- **Charts**: graphics-mode plots are transparent where nothing is drawn rather
  than filled with the theme background, so cell-drawn text — value labels, a
  donut's centre text — is no longer hidden by the image composited over it.
- **Charts**: automatic y domains reach the zero baseline whenever a chart
  carries a bar or area series. These marks encode their value in the filled
  span, so a domain starting at the data minimum drew a bar of 1 beside a bar
  of 5 as a sliver beside a full column — a ratio the data never contained.
  Line, step, and scatter series are read as positions and keep the tighter
  domain that resolves their variation; `Chart::y_domain` still overrides both.

## [0.9.0] - 2026-08-15

Released alongside `tuika-charts` 0.1.0 — its first release — plus
`tuika-mermaid` 0.3.0, `tuika-codeformatters` 0.4.2, and `tuika-html` 0.1.2.
Their tuika dependency requirements now track 0.9.

### Highlights

**Charts, in a companion crate.** `tuika-charts` renders line, area, bar, step,
and scatter series and picks its own fidelity: terminal graphics where the
emulator supports them, portable half-block and Braille cells everywhere else,
from the same `Chart` description.

![chart demo](https://raw.githubusercontent.com/everruns/tuika/v0.9.0/docs/charts/line-graphics.png)

**Trees and multi-pane interaction.** `TreeList` brings stable-id expansion,
selection, and persistent scrolling over host-owned rows, on the same reusable
primitives — `SelectViewportState`, `FocusRegistry::focus`, hit testing — that
let several panes in one screen share focus and the mouse.

![tree list demo](https://raw.githubusercontent.com/everruns/tuika/v0.9.0/docs/demos/tree_list.gif)

- **Runner**: one `FrameSource` boundary and one `Signal` replace ten `run*` methods
  across the two runtimes; the synchronous loop is now drivable from a
  caller-owned terminal, so a whole application can be tested with no tty.

### Added

- `Runner::run_driven_by` is the synchronous counterpart to
  `AsyncRunner::run_driven_by`: the loop against a caller-owned terminal and
  event source, with no `TerminalSession`, no terminal construction, and no
  stdout-facing graphics or clipboard work. `run` and `run_with_backend` are now
  built on it, so every synchronous entry point shares one loop. It is generic
  over the run's error type, so an infallible backend (`TestBackend`) pairs with
  an infallible source.
- `EventSource` is where that loop gets its input, and `scripted_events` builds
  one from a known sequence of events — replaying them in order, then going idle
  like a quiet terminal so tick pacing is unchanged. Together they let a host
  (or a test) drive a whole application with no tty: previously the synchronous
  loop could not be exercised at all, and only its pieces (`RunnerCore`, the
  clock, selection) had coverage. Seven end-to-end tests now cover the initial
  frame, event-driven state, clean updates neither rebuilding nor repainting,
  the borrowed application boundary, redraw requests, deferred resize repaints, and
  split-footer publishing.
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
- `AsyncRunner::run_with_messages` delivers a typed application stream beside
  terminal events and ticks, with deterministic completion/error/redraw/exit
  behavior and no shared mutable state or polling.
- `AsyncApplication` is the borrowed-view application boundary for the async runner,
  the counterpart to `Application`: previously a frame could borrow application
  state only on the synchronous runner, so a Tokio host had to clone into an
  owned tree or share through `Rc<RefCell<_>>`.
- `FrameSource` / `AsyncFrameSource` is the single boundary every run method now
  takes, with exactly two implementors: `&mut app` for an `Application`, and
  `from_fn(&mut state, view, update)` (`async_from_fn` for the async runner) for
  the closure form. `no_messages()` is the empty application-message stream.
- `AsyncRunner::redraw_handle` matches `Runner::redraw_handle`, so a `Live`
  value (or any background producer) can mark the next frame stale on either
  runner. `RedrawHandle` now registers the waiting task's waker, so the async
  loop is woken by a request instead of only noticing one on its next tick;
  bursts still coalesce into a single repaint.

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

- The published crate excludes the generated `site/` bundle and the public
  `docs/` tree, keeping the 0.9.0 archive below crates.io's 10 MiB upload limit.
  README guide and image links now use release-tag-pinned absolute URLs, and the
  split-footer recording lives in its focused guide rather than being
  duplicated in the README.
  Committed IAI benchmark result snapshots are excluded from both crates that
  own them; benchmark source remains available.
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

**Breaking: the runner's `run*` surface is one boundary plus two axes.** The methods
were a cross product of frame source, runtime, terminal ownership, and extra
input, encoded as names — ten of them on `AsyncRunner` — with arbitrary holes in
the product. The frame source is now a `FrameSource` argument rather than a
name, and messages ride the existing `Signal`, which leaves only *where the
terminal and the input come from* in the names.

`Signal` gained a message type parameter, `Signal<M = Infallible>`. Because the
default is uninhabited, an existing `match signal { Signal::Tick => …,
Signal::Event(e) => … }` **still compiles and stays exhaustive**. A `match` on a
*reference* (`match &signal`) does need a `_` arm added. `AsyncSignal<M>` is now
a deprecated alias for `Signal<M>`.

| Before | After |
| --- | --- |
| `Runner::run(theme, state, view, update)` | `Runner::run(theme, from_fn(state, view, update))` |
| `Runner::run_app(theme, app)` | `Runner::run(theme, app)` *(shim, deprecated)* |
| `Runner::run_with_backend(theme, backend, state, view, update)` | `Runner::run_with_backend(theme, backend, from_fn(state, view, update))` |
| `Runner::run_app_with_backend(theme, backend, app)` | `Runner::run_with_backend(theme, backend, app)` *(shim, deprecated)* |
| `AsyncRunner::run(theme, state, view, update)` | `AsyncRunner::run(theme, async_from_fn(state, view, update))` |
| `AsyncRunner::run_with_messages(theme, state, messages, view, update)` | `AsyncRunner::run_with_messages(theme, async_from_fn(state, view, update), messages)` |
| `AsyncRunner::run_with_backend(theme, backend, state, view, update)` | `AsyncRunner::run_with_backend(theme, backend, async_from_fn(state, view, update))` |
| `AsyncRunner::run_with_events(terminal, theme, state, events, view, update)` | `AsyncRunner::run_driven_by(terminal, theme, async_from_fn(state, view, update), events, no_messages())` *(shim, deprecated)* |
| `AsyncRunner::run_with_events_and_messages(terminal, theme, state, events, messages, view, update)` | `AsyncRunner::run_driven_by(terminal, theme, async_from_fn(state, view, update), events, messages)` *(shim, deprecated)* |

Methods marked *shim* keep working with a deprecation warning. The rest changed
shape under the same name, so they fail to compile rather than warn — the
migration is mechanical and the table above is exhaustive.

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
block-HTML boundary, plus `tuika-codeformatters` 0.4.0 and `tuika-mermaid` 0.2.0.
The existing companions adopt tuika 0.7's breaking view-measurement API and
update their tuika dependency requirement in the same release.

### Highlights

**Responsive application primitives.** Docked panels, target-relative overlays,
virtualized collections, semantic styles, and single-line input give hosts a
coherent framework for complex terminal applications without retained UI state.

![primitives demo](https://raw.githubusercontent.com/everruns/tuika/v0.7.0/docs/demos/primitives.gif)

**Rich HTML in Markdown.** Presentational inline HTML now shares Markdown's
styles, while the new parser-free `HtmlBlockRenderer` boundary lets `tuika-html`
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

- `Clock` and `SystemClock` provide one monotonic time boundary.
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
- `StyleRole` and `StyleResolver` form an open semantic styling boundary for hosts
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
- **One structured markdown block boundary.** `MarkdownBlockRenderer` receives a
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
  the markdown boundary. `<sub>`/`<sup>` now transliterate there too, so one
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
| `tuika::` | the framework spine — `View`, `Element`, `RenderCtx`, layout, events, `Theme`, `Surface`, the host boundary |
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

- **New**: `components::markdown::FencedBlockRenderer` — the boundary, plus
  `Markdown::block_renderer`, `MarkdownState::with_block_renderer`, and
  `markdown::{to_lines_with_renderer, to_linked_lines_with_renderer}`.
- **New crate**: [`tuika-mermaid`](https://crates.io/crates/tuika-mermaid) —
  `MermaidRenderer`, an mmdflux-backed implementation for ```` ```mermaid ````
  fences. Diagram layout stays out of tuika core.

**Long transcripts scroll by item, not by line.** `ItemScroll` scrolls a list of
laid-out elements — the shape a chat log or an agent transcript actually has —
and the text input grew the boundaries a composer needs.

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
- **Ratatui interop is named for the boundary, not for its only type.**
  - Before: `tuika::ratatui_view::RatatuiView`
  - After: `tuika::interop::RatatuiView`

**Items renamed**

- **The OSC encoders share one shape** — a pure `encode`, a thin writer.
  - Before: `osc52`, `write_clipboard`, `osc8`, `osc8_with`, `encode_pointer_shape`, `write_pointer_shape`
  - After: `term::clipboard::{encode, write}`, `term::hyperlink::{encode, encode_with}`, `term::pointer::{encode, write}`
- **`tuika::highlight` was a module and a function at once.** The module is the
  `Highlighter` boundary; the function paints a selection.
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
* feat(components): add `ItemScroll` and the composer token boundaries, plus the `codex` example
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
[Unreleased]: https://github.com/everruns/tuika/compare/v0.11.0...HEAD
[0.10.0]: https://github.com/everruns/tuika/compare/v0.9.0...v0.10.0
