<p align="center">
  <img src="https://raw.githubusercontent.com/everruns/tuika/v0.11.0/logo.svg" width="144" alt="tuika logo: two offset rounded interface panels intersect at a gold anchor point">
</p>

<h1 align="center">tuika</h1>

<div align="center">

[![crates.io](https://img.shields.io/crates/v/tuika.svg)](https://crates.io/crates/tuika)
[![docs.rs](https://img.shields.io/docsrs/tuika)](https://docs.rs/tuika)
[![downloads](https://img.shields.io/crates/d/tuika.svg)](https://crates.io/crates/tuika)
[![license](https://img.shields.io/crates/l/tuika.svg)](https://github.com/everruns/tuika/blob/main/LICENSE)
![msrv](https://img.shields.io/badge/rust-1.88%2B-blue.svg) \
[Website](https://tuika.dev) · [Rust API](https://docs.rs/tuika) · [Getting started](https://github.com/everruns/tuika/blob/v0.11.0/docs/getting-started.md) · [Components](https://github.com/everruns/tuika/blob/v0.11.0/docs/components.md) · [Layout](https://github.com/everruns/tuika/blob/v0.11.0/docs/layout.md) ·
[Markdown](https://github.com/everruns/tuika/blob/v0.11.0/docs/markdown.md) · [Charts](https://github.com/everruns/tuika/blob/v0.11.0/docs/charts.md) ·
[Terminal features](https://github.com/everruns/tuika/blob/v0.11.0/docs/features.md) ·
[Keymap](https://github.com/everruns/tuika/blob/v0.11.0/docs/keymap.md) · [Input routing](https://github.com/everruns/tuika/blob/v0.11.0/docs/routing.md) ·
[Styling](https://github.com/everruns/tuika/blob/v0.11.0/docs/styling.md) ·
[Themes](https://github.com/everruns/tuika/blob/v0.11.0/docs/themes.md) \
[Showcases](https://github.com/everruns/tuika/blob/v0.11.0/docs/showcases.md) · [Examples](#runnable-examples) ·
[Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) ·
[Report a bug](https://github.com/everruns/tuika/issues)

</div>

<p align="center">
  <img src="https://raw.githubusercontent.com/everruns/tuika/v0.11.0/docs/hero.gif" width="880" alt="Animated tuika gallery: a terminal window with tabs, an activity panel of spinners, progress bars and a loader, a command palette, a commit-message input, and a status bar — all animating.">
</p>

<div align="center">

### tuika's goal is to become the default TUI [application](https://github.com/everruns/tuika/blob/v0.11.0/docs/showcases.md) framework for Rust

**Build the app, not the render loop.**

</div>

<details>
<summary>Table of contents</summary>

- [Install](#install)
- [Model](#model)
- [Crate layout](#crate-layout)
- [Components](#components)
- [Example](#example)
- [Owned scenes, dialogs, and forms](#owned-scenes-dialogs-and-forms)
- [Markdown and syntax highlighting](#markdown-and-syntax-highlighting)
- [Theming](#theming)
- [Runnable examples](#runnable-examples)
- [Declarative DSL (`view!`)](#declarative-dsl-view)
- [Ratatui interoperability](#ratatui-interoperability)
- [Screen modes, lifecycle, and runner](#screen-modes-lifecycle-and-runner)
- [Images](#images)
- [Mouse, selection, and clipboard](#mouse-selection-and-clipboard)
- [Testing your UI](#testing-your-ui)
- [Used in](#used-in)
- [Compatibility](#compatibility)
- [Extending](#extending)
- [License](#license)

</details>

`ratatui` gives you a cell buffer and widgets to draw into it; everything above
that — layout, overlays, focus, input, the terminal lifecycle — has been left to
each application to build for itself. tuika is that missing layer, and wants to
be the standing answer to "what do I build a Rust TUI *application* on?": start
with `cargo add tuika`, describe your screen, and get a real app instead of a
render loop.

You write views; tuika owns the rest:

- **A whole app, not a widget set** — [flexbox layout](#model), anchored
  [overlays](#owned-scenes-dialogs-and-forms), focus, a declarative
  [keymap](https://github.com/everruns/tuika/blob/v0.11.0/docs/keymap.md), [themes and stylesheets](#theming), and a
  [runner](#screen-modes-lifecycle-and-runner) that owns raw mode, the alternate
  screen (or a [split footer](#screen-modes-lifecycle-and-runner) over live
  scrollback), and event translation.
- **Batteries the terminal era expects** — [30+ components](#components)
  including streaming [Markdown](https://github.com/everruns/tuika/blob/v0.11.0/docs/markdown.md) with pluggable syntax
  highlighting, [images](#images) over Kitty/iTerm2/Sixel, adaptive
  [charts](https://github.com/everruns/tuika/blob/v0.11.0/docs/charts.md), mermaid diagrams,
  [mouse selection and clipboard](#mouse-selection-and-clipboard), and
  [native OSC 9;4 progress](#native-terminal-progress).
- **No lock-in** — it is additive to ratatui, not a replacement: wrap any
  ratatui widget in [`RatatuiView`](#ratatui-interoperability) and it composes
  like a built-in, and your own types implement the same `View` trait the
  built-ins do (see [Extending](#extending)).
- **Boring where it counts** — no reconciler, no retained tree, no runtime, no
  macro DSL you are forced into. Views are rebuilt each frame and `ratatui`
  diffs the buffer. Rendering is deterministic, so
  [UI is unit-tested](#testing-your-ui) against an in-memory buffer with no
  terminal at all.
- **Small enough to adopt without a second thought** — a self-contained crate
  depending only on `ratatui-core`, `crossterm`, `unicode-segmentation`,
  `unicode-width`, and `pulldown-cmark`. Anything heavy — grammars, diagram
  layout, image decoding — lives behind a trait in a companion crate or your
  host.

It is host-agnostic: it knows nothing about the application embedding it, and no
type, feature, or default exists to serve one host. tuika renders none of
ratatui's own widgets, so it builds against `ratatui-core` directly rather than
the `ratatui` umbrella — keeping `ratatui-widgets`, `ratatui-macros`, and their
transitive weight out of its dependency tree. Your application still uses any
ratatui widget it likes (see [Compatibility](#compatibility)). (The optional
`async` feature adds Tokio for [`AsyncRunner`](#screen-modes-lifecycle-and-runner);
it is off by default.)

See what that buys in practice: the [showcases](https://github.com/everruns/tuika/blob/v0.11.0/docs/showcases.md) are
recordings of real applications running on tuika (also listed under
[Used in](#used-in)), and the [`codex` example](examples/codex) is a whole
coding-agent UI built with nothing else.

## Install

```bash
cargo add tuika
```

`ratatui` and `crossterm` are part of `tuika`'s public interoperability
surface, so add `ratatui` to your own crate and pin a compatible minor version.
tuika depends on `ratatui-core`, which the `ratatui` umbrella re-exports, so
Cargo unifies the shared `Buffer`/`Rect`/`Style` types and your widgets compose
with tuika's surfaces (see [Compatibility](#compatibility)).

## Model

- **Views** (`view::View`) are rebuilt from application state every frame. This
  is cheap because `ratatui` diffs the resulting cell buffer, so there is no
  reconciler.
- **State** that must survive across frames — scroll offset, selection index,
  focus, dock visibility — lives in host-persisted `*State` structs (the
  `StatefulWidget` idiom), not in the view tree.
- **Live data** (`Live` / `LiveView`) is shared application state read at render
  time. Updates request a redraw from the runner; Tuika does not spawn data
  sources or reconcile a retained widget tree.
- **Layout** is an [integer-native flexbox subset](https://github.com/everruns/tuika/blob/v0.11.0/docs/layout.md) (`layout`): wrapped flex lines,
  independent basis/grow/shrink/min/max child styles, cross-line alignment, and
  exact boundary rounding over one direction-agnostic solver. `Flow` packages
  intrinsic wrapping; `Grid` is the smaller equal-column, row-major alternative
  to adopting CSS Grid.
- **Overlays** (`overlay`) anchor a view over the base tree; the **host**
  (`host`) owns the alternate screen, translates crossterm input, and
  composites the frame.
- **Keymap** ([`keymap`](https://github.com/everruns/tuika/blob/v0.11.0/docs/keymap.md)) resolves declarative key bindings to
  named commands: chords (`ctrl+r`) and multi-stroke sequences (`g g`) grouped
  into prioritized, mode-gated `Layer`s, dispatched from a translated `Key` and
  queryable for help/`KeyHints` surfaces. Character chords are exact logical
  text (`A`, `?`, `ж`), so the active keyboard layout is applied before
  matching; Shift stays explicit for non-character keys such as `Shift+Enter`.
  Host-agnostic, so it unit-tests without a terminal. See the
  [keymap guide](https://github.com/everruns/tuika/blob/v0.11.0/docs/keymap.md).
- **Input routing** ([`routing`](https://github.com/everruns/tuika/blob/v0.11.0/docs/routing.md)) delivers an event to the
  surface that owns input this frame — every event kind through one
  registration, so a paste cannot take a different path from a key. `Router`
  reads the focus registry an overlay-bearing `Scene` already synchronized, and
  reports through `Delivery` which surface received what, including the case
  where nothing did. See the
  [routing guide](https://github.com/everruns/tuika/blob/v0.11.0/docs/routing.md).
- **Motion** (`anim`, `components::{Spinner, ProgressBar, Loader}`,
  `term::progress::TerminalProgress`) animates from a host-supplied frame counter and
  can drive the terminal's own OSC 9;4 progress indicator. `anim::Timeline` adds
  a scheduler-free keyframe track (values eased over frame offsets, with
  looping/ping-pong) sampled purely from that counter.
- **Pixels** (`framebuffer`) — a mutable RGBA `FrameBuffer` the host draws into
  (`set`/`blend`/`fill_rect`/`blit`, a per-pixel `shade` shader post-pass, and
  `Sprite` spritesheet frames). `FrameBufferView` paints it into cells with
  half-blocks on any terminal, or hand `to_image_data()` to the crisp graphics
  protocols.

## Crate layout

Four places, so you can guess where something is:

| Path | Holds |
| --- | --- |
| `tuika::` | the framework spine — `View`, `view_fn`, `Element`, `ScopedElement`, `RenderCtx`, layout, events, `Theme`, `Surface`, the host boundary |
| `tuika::components` | every widget: `Flex`, `Boxed`, `Text`, `Scroll`, `Markdown`, `Table`, … |
| `tuika::term` | everything out-of-band: `clipboard` (OSC 52), `hyperlink` (OSC 8), `progress` (OSC 9;4), `pointer` (OSC 22), `image`, `capabilities`, `palette` (the terminal's own colors) |
| `tuika::prelude` | the spine and the components in one glob import |

Application code usually wants the prelude:

```rust
use tuika::prelude::*;
```

Everything else stays behind its module path on purpose — `themes::by_name`,
`probe::RectProbe`, `width::str_cols`, `term::clipboard::write` — so a short
path always means "you will use this constantly".

## Components

See the [component gallery](https://github.com/everruns/tuika/blob/v0.11.0/docs/components.md) for an animated demo of each
component. Linked names below jump straight to their demo.

| Component | Purpose |
| --- | --- |
| [`Text`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/text.md#text) / `Paragraph` | Literal styled lines / word-wrapped prose with bare web links |
| `Wrap` | Word-wraps pre-styled lines, preserving per-span styles |
| [`Markdown`](https://github.com/everruns/tuika/blob/v0.11.0/docs/markdown.md) (+ `MarkdownState`) | CommonMark → styled lines; `MarkdownState` streams incrementally — see the [markdown guide](https://github.com/everruns/tuika/blob/v0.11.0/docs/markdown.md) |
| [`CodeBlock`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/markdown-code.md#codeblock) | Themed, framed code block with a pluggable `Highlighter` and optional line-number gutter |
| [`Html`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/markdown-code.md#html) | HTML fragment → styled lines (companion crate [`tuika-html`](crates/tuika-html/)) |
| `Diff` | Line diff (LCS), unified or side-by-side, with `+`/`-` gutters and line numbers |
| `AsciiFont` | Large "figlet-style" block-letter banner text |
| `QrCode` (+ `QrEcc`) | QR code (byte-mode v1–4 encoder) rendered with half-blocks |
| [`Rule`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/text.md#rule) | Horizontal separator: optional title + fill glyph to width |
| [`Flex`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#flex) | Flexbox container (the composition primitive) |
| [`Flow`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#flow) | Intrinsic-width items wrapped into flex lines |
| [`Grid`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#grid) | Small equal-column, row-major terminal grid |
| `Responsive` / `Constrained` | Breakpoint selection and min/max measurement |
| [`Boxed`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#boxed) | Border + padding + title, focus-aware |
| `Scene` / `ScopedScene` / `Dialog` | Owned or frame-borrowed root + anchored overlays |
| [`ConfirmDialog`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#dialog-presets) / `ChoiceDialog` / `MultiChoiceDialog` / `InputDialog` | Stateful presets for common modal flows |
| `Spacer` | Flexible filler |
| [`Scroll`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#scroll--scrollstate) (+ `ScrollState`) | Vertical scroll viewport + scrollbar over lines |
| [`ItemScroll`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#itemscroll) | The same viewport over laid-out items (panels, tables, nested layouts) |
| `Viewport` | Two-dimensional clipping/panning over any child view |
| [`Scrollbar`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#scrollbar--virtualwindow) / `VirtualWindow` | Reusable bars and clamped ranges for virtualized collections |
| `Form` / `FormField` (+ `FormState`) | Responsive labeled controls and validation |
| `DrawView` / `CanvasView` | Closure-based custom cell drawing |
| [`SelectList`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#selectlist--selectstate) (+ `SelectState`) | Selectable list, including host-windowed collections |
| [`TreeList`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#treelist--treestate) (+ `TreeState`) | Stable-id expandable tree over host-provided rows |
| [`SelectionScreen`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#selectionscreen) | Responsive full-screen action/agent/permission pickers |
| [`KeyedTable`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#keyedtable--keyedselectstate) (+ keyed single/multi-selection) | Borrowed, virtualized slice or projected rows whose selection follows stable application keys |
| [`CompletionPalette`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#completionpalette--completionstate) (+ `CompletionState`, `CompletionItem`) | Filter-ranked command and token completion |
| `Slider` (+ `SliderState`) | One-row value picker over a numeric range |
| [`TextInput`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#textinput--textinputstate) (+ `TextInputState`) | Multi-line composer: soft-wrap, placeholder, highlighted ranges, `@`/`/` tokens |
| [`StatusBar`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/layout.md#statusbar) | One-row left/right status segments |
| [`Tabs`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/interactive.md#tabs--tabsstate) / `KeyHints` | Host-state tab navigation and command hints |
| `TabSelect` (+ `TabSelectState`) | Value-selecting segmented control |
| `Toasts` / `ToastList` | Transient notification stack with frame-driven expiry |
| `Console` (+ `ConsoleLog`) | Captured stdout/log ring buffer + tailing overlay view |
| [`Spinner`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/motion.md#spinner) | Frame-cycled activity glyph |
| [`ProgressBar`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/motion.md#progressbar) | Determinate (sub-cell) / indeterminate bar |
| [`ActivityList`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/motion.md#activitylist) | Multi-step lifecycle status with optional per-step progress |
| [`Loader`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/motion.md#loader) | Spinner + message + hint row |

## Example

Layout reads top-down with the [`view!`](#declarative-dsl-view) DSL:

```rust
use tuika::prelude::*;

let theme = Theme::default();
let root = view! {
    col(gap = 1) {
        fixed(1) { node(Spinner::new(frame)) }
        fixed(1) { node(ProgressBar::determinate(0.6).percent(true)) }
        grow(1) { text("body") }
    }
};

// In a `terminal.draw(|f| ...)` closure:
paint(f.buffer_mut(), f.area(), &theme, root.as_ref(), &[]);
```

## Owned and scoped scenes, dialogs, and forms

`Scene` owns a root `Element` and ordered `SceneOverlay`s. Each layer retains
its `OverlaySpec`, so it resolves against the current terminal size inside
rendering; callers do not retain pre-resolved `Rect`s.
`Dialog` composes `Boxed`, `Flex`, and optional `KeyHints` into a centered modal
with size clamps, clear/dim behavior, and an optional focus-owner id:

```rust
use tuika::prelude::*;

let scene = Scene::new(element(base)).dialog(
    Dialog::new("Confirm", element(Text::raw("Delete this item?")))
        .min_size(30, 7)
        .max_size(70, 20)
        .key_hints([("enter", "delete"), ("esc", "cancel")])
        .dim_backdrop(true)
        .focus_owner("confirm"),
);
scene.sync_focus(&mut focus);
paint_scene(buffer, area, &theme, &scene);
```

For popovers, menus, and tooltips, wrap the trigger with a
`probe::RectProbe` and attach the probe to a `SceneOverlay`. The root renders
first, so placement uses the trigger's current rect in the same frame. It can
align on any side, keep a gap, flip when the preferred side runs out of room,
and clamp to the screen margin:

```rust
use tuika::overlay::Extent;
use tuika::prelude::*;
use tuika::probe::RectProbe;

let trigger = RectProbe::new();
let root = element(Flex::column().fixed(
    1,
    trigger.wrap(Text::raw("Open actions")),
));
let menu_size = OverlaySpec {
    width: Extent::Cells(28),
    height: Extent::Cells(7),
    ..OverlaySpec::centered(0, 0).margin(1)
};
let scene = Scene::new(root).overlay(
    SceneOverlay::new(element(Text::raw("Run action")), menu_size).target(
        &trigger,
        TargetPlacement::below().align(TargetAlign::Start).gap(1),
    ),
);
```

Custom views can import `Rect`, `Color`, `Style`, `Modifier`, `Line`, and `Span` from `tuika::ui` or the prelude without adding `ratatui-core` directly.

`Element` is an owned, boxed view. `ScopedElement<'_>` is its frame-borrowed
counterpart: `element(view)` chooses the lifetime from `view`, and containers
accept it at any depth. `ScopedScene` borrows the resulting root for one paint
while continuing to own ordinary `SceneOverlay`s and `Dialog`s:

```rust
use tuika::prelude::*;

struct Dashboard<'a> {
    messages: &'a [String],
}

impl View for Dashboard<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        available
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        for (row, message) in self.messages.iter().take(area.height as usize).enumerate() {
            surface.set_string(area.x, area.y + row as u16, message, ctx.theme.text_style());
        }
    }
}

let dashboard = Dashboard { messages: &app.messages };
let scene = ScopedScene::new(&dashboard).dialog(
    Dialog::new("Confirm", element(Text::raw("Delete this item?")))
        .dim_backdrop(true)
        .focus_owner("confirm"),
);
scene.sync_focus(&mut focus);
paint(buffer, area, &theme, &scene, &[]);
```

The borrow lasts only as long as the scoped scene, matching Tuika's
frame-by-frame view model. No transcript clone, leaked allocation, custom
wrapper view, or application compositor is needed. The `view!` macro preserves
the same lifetime through nested `Flex` and `Boxed` containers.

Measurement receives the same `RenderCtx` as rendering. A custom view whose
geometry depends on the active theme or stylesheet resolves it there, and every
container passes that context to the children it measures.

For a bespoke region, `view_fn` takes those two methods as closures and returns
a normal `View`. The closures can borrow the same application state as the
surrounding frame; they are `Fn`, so repeated measurement or rendering observes
that state without cloning it or requiring `Rc<RefCell<_>>`:

```rust
use tuika::prelude::*;

struct App {
    query: String,
    match_label: String,
    results: Vec<String>,
}

let app = App {
    query: "view".into(),
    match_label: "3 matches".into(),
    results: vec!["src/view.rs".into(), "src/components/app_shell.rs".into()],
};
let search_header = view_fn(
    |available, _ctx| Size::new(available.width, available.height.min(2)),
    |area, surface, ctx| {
        let row = area.bottom().saturating_sub(1);
        surface.set_string(area.x, row, &app.query, ctx.theme.text_style());
        surface.set_string(
            area.right().saturating_sub(10),
            row,
            &app.match_label,
            ctx.theme.muted_style(),
        );
    },
);
let screen = AppShell::new(view_fn(
    |available, _ctx| available, // growing body
    |area, surface, ctx| {
        for (row, result) in app.results.iter().take(area.height as usize).enumerate() {
            surface.set_string(area.x, area.y + row as u16, result, ctx.theme.text_style());
        }
    },
))
.header(search_header);
let _ = screen;
```

In the AGF search-header port that motivated this adapter, the named wrapper's
`struct` plus `View` scaffold is 9 nonblank lines around the render logic; the
equivalent `view_fn` scaffold is 5. The render body is unchanged, and the call
site no longer needs a named type.

`Form` lays out arbitrary control `Element`s beside responsive labels, stacking
on narrow terminals. Help and validation rows are built in; `FormState` owns
only focus traversal, while values and cursor state stay in existing host-owned
`TextInputState`, `SelectState`, or application models. Their `handle` methods
share `InputOutcome`: ignored events bubble, recognized no-ops are consumed,
state changes are distinct from submit/cancel intent, and submitted values are
read from the state instead of duplicated in the outcome.

## Arbitrary-child viewports and drawing

`Viewport` clips and pans any child view in both axes. The host supplies the
full content `Size` and mirrors offsets through the same `ScrollState` used by
line-oriented `Scroll`. It renders only the visible source window, so a large
logical canvas does not allocate a full off-screen buffer.

`DrawView` (also named `CanvasView`) turns a render-only closure receiving `(Rect,
&mut Surface, &RenderCtx)` into a normal view. The surface is already clipped,
making it suitable for terminal grids, charts, emulators, and incremental
migrations. It reports either all available space or a fixed intrinsic size;
use `view_fn` when measurement itself is custom. Import `DrawView` explicitly
from `tuika::view`; custom canvases stay outside the application prelude.

Run `cargo run --example primitives` for one composition using `Scene`,
`Dialog`, `Form`, `Viewport`, and `DrawView`.

### Builder syntax (alternative)

`view!` expands to plain builder calls, so the same tree can be written without
the macro:

```rust
use tuika::prelude::*;

let root = Flex::column()
    .gap(1)
    .fixed(1, element(Spinner::new(frame)))
    .fixed(1, element(ProgressBar::determinate(0.6).percent(true)))
    .grow(1, element(Text::raw("body")));
```

## Markdown and syntax highlighting

`Markdown` renders CommonMark to styled lines, word-wrapping prose while drawing
code and tables verbatim. `MarkdownState` is its streaming form: fed deltas as a
message arrives, it re-parses only the in-flight tail and caches everything
before the last stable block boundary, so long transcripts don't re-tokenize and
settled code blocks aren't re-highlighted every frame. Its `links()` metadata
keeps OSC 8 targets aligned with the cached lines through streaming and resize.

Highlighting is a boundary, not a dependency: `tuika` owns the *presentation* of code
(framing, background, language label, wrapping) via `CodeBlock`, and takes token
colors from any `Highlighter` you supply — keeping the toolkit free of grammar
crates. The companion crate
[`tuika-codeformatters`](https://crates.io/crates/tuika-codeformatters) ships a
ready-made tree-sitter `Highlighter`.

Structured blocks can replace their source with a different, width-aware
presentation through `MarkdownBlockRenderer`. The
[`tuika-mermaid`](crates/tuika-mermaid/) companion uses that boundary with mmdflux:
a `mermaid` fence becomes a Unicode cell diagram inside the surrounding
Markdown, with no browser, SVG, or image protocol. Unsupported or invalid input
falls back to the ordinary code block.

```rust
use tuika::prelude::*;
use tuika_mermaid::MermaidRenderer;

let mermaid = MermaidRenderer::new();
let document = Markdown::new(
    "```mermaid\nflowchart LR\n  Parse --> Layout --> Paint\n```",
)
.block_renderer(&mermaid);
# let _ = document;
```

Run the complete integration demo with
`cargo run -p tuika-mermaid --example mermaid_markdown`.

<img src="https://raw.githubusercontent.com/everruns/tuika/main/crates/tuika-mermaid/examples/mermaid_markdown/mermaid.gif" width="880" alt="Mermaid diagram rendered as Unicode cells inside tuika Markdown">

Images use the same host-extension pattern: supply an `ImageResolver` and
`![alt](url)` renders as real pixels (see [Images](#images)).

Markdown in the wild carries HTML. The presentational inline tags — `<b>`,
`<em>`, `<code>`, `<kbd>`, `<mark>`, `<a>`, `<br>`, `<sub>`/`<sup>` — render in
tuika itself, each through the same `StyleSheet` role as the markdown it
mirrors. Block-level HTML is a boundary, for the same reason highlighting is: an
HTML parser is a dependency tuika will not carry. Attach a
`MarkdownBlockRenderer` and `<details>`, `<table>`, and `<div>` lay out too. The
same ordered renderer chain handles fenced diagrams and block HTML with one
context, including the active stylesheet.
[`tuika-html`](crates/tuika-html/) is the ready-made one, and it also supplies
the [`Html`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/markdown-code.md#html) component for markup that is not inside
markdown at all. See the markdown guide for
[inline HTML](https://github.com/everruns/tuika/blob/v0.11.0/docs/markdown.md#inline-html) and
[block HTML](https://github.com/everruns/tuika/blob/v0.11.0/docs/markdown.md#block-html).

## Theming

Every component styles itself from a `Theme` passed through the render context —
no color is hard-coded, so swapping the theme handed to `paint` restyles the
whole tree at once. A `Theme` is a plain `Copy` struct of colors, and tuika
bundles a few standard palettes as full `const Theme` structures in the
[`themes`](https://docs.rs/tuika/latest/tuika/themes/index.html) module —
reachable directly, by constructor, or by name:

```rust
use tuika::themes;

let a = themes::GRUVBOX_DARK;                          // the struct
let b = tuika::Theme::gruvbox_dark();                  // named constructor
let c = tuika::themes::by_name("gruvbox-dark").unwrap(); // config / --theme
```

See the [theme gallery](https://github.com/everruns/tuika/blob/v0.11.0/docs/themes.md) for a screenshot of each bundled
palette, or `themes::PRESETS` to enumerate them for a picker.

An app can also inherit the palette the user already configured in their
terminal, rather than bringing its own — either implicitly with `themes::TERMINAL`
(ANSI slots, no I/O) or by asking the terminal for its actual colors and deriving
a full theme from the reply. It is opt-in: tuika never probes unless a host asks
it to. The query lives with the other out-of-band escapes, in `term::palette`. See
[inheriting the terminal's colors](https://github.com/everruns/tuika/blob/v0.11.0/docs/features.md#inheriting-the-terminals-colors).

Where a `Theme` is the color *tokens*, a `StyleSheet` is the *rules* — a mapping
from a semantic role (heading, link, inline code, a panel's border and fill, …)
onto the style it draws with. Override a role in one place and every element with
that role restyles at once; markdown, toast severities, diff rows, and key hints
are role-driven too. Companion crates and applications can define namespaced
`StyleRole`s and install a `StyleResolver` without expanding tuika's closed data
model. See the [styling guide](https://github.com/everruns/tuika/blob/v0.11.0/docs/styling.md). `StyleBundle::padding` is layout, not a
paint-only hint: `Boxed` resolves panel padding during both measurement and
rendering; an explicit `Boxed::padding` remains the per-instance override.

## Runnable examples

Each takes over the terminal — the alternate screen, or a pinned footer for
[`split_footer`](examples/split_footer.rs); press `q` (or `esc`) to quit.
Pass `--theme <name>` after Cargo's `--` to run any example with a bundled
palette, for example `cargo run --example gallery -- --theme gruvbox-dark`.

| Example    | Command                                   | Shows                                              |
| ---------- | ----------------------------------------- | -------------------------------------------------- |
| [`gallery`](examples/gallery.rs)  | `cargo run --example gallery`    | motion components + native OSC 9;4 progress and OSC 8 links |
| [`markdown`](examples/markdown.rs) | `cargo run --example markdown`   | streaming `MarkdownState` + highlighted `CodeBlock`, native OSC 8 links, following the stream until you scroll back |
| [`select`](examples/select.rs)   | `cargo run --example select`     | interactive multi-select with aliases, numbers, and mouse hit-testing |
| [`keyed_table`](examples/keyed_table.rs) | `cargo run --example keyed_table` | dynamic borrowed rows, stable keyed selection, filtering, and reordering |
| [`tree_list`](examples/tree_list.rs) | `cargo run --example tree_list` | expandable stable-id tree, refresh, mouse selection, and persistent scrolling |
| [`overlay`](examples/overlay.rs)  | `cargo run --example overlay`    | Target-following popover + input routing           |
| [`primitives`](examples/primitives.rs) | `cargo run --example primitives` | owned dialog scene + form + arbitrary-child viewport |
| [`ratatui_dashboard`](examples/ratatui_dashboard.rs) | `cargo run --example ratatui_dashboard` | mixed Ratatui widgets + responsive live data |
| [`async_dashboard`](examples/async_dashboard.rs) | `cargo run --example async_dashboard --features async` | typed background messages waking `AsyncRunner`, no shared mutable state |
| [`mouse`](examples/mouse.rs)     | `cargo run --example mouse`      | drag-to-select + highlight + OSC 52 copy, clickable buttons |
| [`image`](examples/image.rs)     | `cargo run --example image`      | `Image` over reserved cells (Kitty/iTerm2/Sixel), alt-text fallback |
| [`inherit`](examples/inherit.rs) | `cargo run --example inherit`    | adopting the terminal's own palette — probe, derive, and the no-I/O fallback |
| [`split_footer`](examples/split_footer.rs) | `cargo run --example split_footer` | a pinned footer over live terminal scrollback, published through `Scrollback` |
| [`codex`](examples/codex)        | `cargo run --example codex`      | a scripted Codex CLI interface replica: streaming transcript, composer, `@`/`/` pickers, approval prompt |
| [`codex --split-footer`](examples/codex) | `cargo run --example codex -- --split-footer` | the same agent UI with its transcript published into the terminal's own scrollback |

Each of the single-topic examples above quits on `q`/`esc`. [`codex`](examples/codex)
is the composite one — those keys are text there, so it quits with `⌃C`.

## Declarative DSL (`view!`)

`view!` is optional sugar over the builders — it expands to the exact same
`Flex`/`Boxed`/`element(...)` calls, so there is no runtime cost and nothing new
in the model. It just makes nested layout read top-down:

```rust
let root = crate::view! {
    col(gap = 1, padding = tuika::Padding::all(1)) {
        boxed(title = " body ") { text("hello") }
        grow(1) { spacer() }
        node(status_bar)          // any expression that is `impl View`
    }
};
```

Grammar (each keyword consumes exactly one node):

- `col(attrs) { … }` / `row(attrs) { … }` — flex containers. Attrs (all
  optional): `gap`, `row_gap`, `column_gap`, `padding`, `align`, `justify`,
  `wrap`, `align_content`, `background`.
- `boxed(attrs) { child }` — bordered container. Attrs: `title`, `border`,
  `padding`, `background`.
- `text(expr)`, `spacer()` — leaves.
- `grow(n) { node }` / `fixed(n) { node }` — set a child's main-axis size
  (default auto).
- `when(condition) { node }` / `for(pattern in iterable) { node }` — conditional
  and repeated children, still expanding to ordinary builder calls.
- **`node(expr)`** — splice any `impl View`. This is the escape hatch, and how
  a component **from another crate** participates in the DSL:

  ```rust
  use other_crate::CustomView;
  crate::view! { col { node(CustomView::new(&data)) } };
  ```

`node(...)` accepts any type that already implements Tuika's `View`; it does
not make a Ratatui `Widget` implement `View`. A node may borrow frame data; the
macro returns `ScopedElement<'_>` in that case and naturally coerces an
all-owned tree to `Element`. Use `RatatuiView` for Ratatui widgets. The
`tuika-gallery` demo is built entirely with `view!`.

## Ratatui interoperability

Tuika deliberately does not duplicate Ratatui's widget catalog. Wrap existing
widgets in `RatatuiView`; they render into an isolated buffer and only the
assigned clip is composited into the frame:

```rust
use ratatui::widgets::{Sparkline, Widget};
use tuika::prelude::*;

let values = vec![1, 4, 2, 8];
let chart = RatatuiView::sized(Size::new(20, 4), move |area, buffer| {
    Sparkline::default().data(&values).render(area, buffer);
});
```

The closure form supports widgets that borrow captured data. Stateful widgets
can capture host-owned synchronized state and call `StatefulWidget::render`
inside the same closure. `Surface::render_ratatui` is the lower-level escape
hatch for custom views that need several widgets. Neither API exposes the
frame's mutable buffer.

## Responsive and live views

`Responsive` chooses complete compact/wide view trees from the current width;
this supports row-to-column reflow and intentionally omitted secondary
content. `Constrained` supplies min/max intrinsic measurements to flex layout.

`DockState` is the small host-owned lifecycle for an auxiliary panel. A visible
panel docks beside the main view on wide frames; below its breakpoint it stays
passive and hidden until focused, then resolves as an overlay drawer. It returns
rectangles only—the host keeps the panel view, focus id, keymap, and state.

```rust
use tuika::prelude::*;

let mut activity = DockState::new();
activity.show_passive();
let layout = activity.resolve(Rect::new(0, 0, 120, 30), DockSpec::right(90, 40));
// Paint the main view into `layout.main` and, when present, the panel into
// `layout.panel`.
```

`Live<T>` is shared application data with a narrow read/update API. `LiveView`
derives a fresh view from its current value each frame. Connect it to
`Runner::redraw_handle()` — or `AsyncRunner::redraw_handle()`, which wakes a
parked `select!` rather than waiting for its next tick — when background
producers should invalidate the screen. Producers retain ownership of their
threads, tasks, retries, and lifecycle.

## Screen modes, lifecycle, and runner

`ScreenMode` picks which part of the terminal a frame owns:

- `ScreenMode::Alternate` (the default) takes the whole window on the alternate
  buffer and restores the user's screen and scrollback on exit. It leaves mouse
  handling to the terminal, so native OSC 8 links, selection, and scrolling work.
- `ScreenMode::split_footer(rows)` reserves those rows at the bottom of the
  *main* screen. Everything above stays the terminal's own scrollback: the shell
  prompt that launched the app, the wheel, and mouse selection all keep working,
  and the output the app publishes is still there after it exits. This is the
  shape for a long-running tool with a live composer, status line, or progress
  panel over output the user wants to keep.

The [screen-modes guide](https://github.com/everruns/tuika/blob/v0.11.0/docs/features.md#screen-modes-alternate-screen--split-footer)
shows the mode in motion and covers its terminal contract in detail.

In split-footer mode a host must not `println!` — the footer owns the cursor.
`Runner::scrollback()` (and `AsyncRunner::scrollback()`) returns a `Scrollback`
handle instead: a cheap, cloneable, `Send + Sync` queue of *views*, which the
runner renders and commits above the footer, one whole block at a time. A host
driving its own loop can skip the queue with `screen::publish_block`, which
commits one view immediately and takes no `Send` bound — so a block may own
frame state that could never cross a thread. Blocks are painted without a
background fill, so they blend into the surrounding shell session rather than
looking like a pasted panel.

```rust,ignore
use tuika::prelude::*;

let runner = Runner::new(RunnerConfig {
    tick_rate: Duration::from_millis(80),
    screen_mode: ScreenMode::split_footer(5),
});
let scrollback = runner.scrollback();

// From any thread; committed above the footer on the next loop iteration.
scrollback.write(|_width| element(Text::raw("build finished in 12 ms")));
```

The footer's height is fixed for the life of the terminal, so a host whose
footer grows (a composer, a completion popup) reserves the tallest state it
needs. There is a `scrolling-regions` feature, but it is a compatibility mirror
of ratatui's, not an optimization to reach for: rows scrolled out of a DECSTBM
region are discarded by the terminal instead of entering its scrollback, which
is the one thing this mode exists to provide.

[`split_footer`](examples/split_footer.rs) is the runnable version of all of
this, and [`codex`](examples/codex) runs its whole coding-agent UI this way with
`--split-footer`: each finished transcript entry is handed to the terminal, and
the composer keeps the bottom rows. Hosts driving their own loop reserve and
release the footer's rows with `screen::pin_footer` and `screen::close_footer`.

`TerminalSession` is the complete RAII guard for either mode: it owns raw mode,
enhanced keyboard reporting, the alternate screen, optional mouse capture, and cursor visibility,
including rollback after partial initialization, and restores exactly what it
took. Enhanced reporting preserves
non-character modifiers, so `Shift+Enter` reaches `TextInputState` as a
different chord from `Enter`; iTerm2 and tmux get their required protocol
variants, while Windows uses the modifier state already carried by its native
console events. Character codes carry the logical text produced by the active
keyboard layout, while modifiers remain separate for non-character chords. It
preserves raw mode and any keyboard-reporting stack entries the caller had
already enabled. `AltScreen` remains available for hosts that intentionally own
raw mode, keyboard modes, and cursor visibility themselves.
`TerminalSession::enter_config(TerminalSessionConfig)` keeps the same rollback
guarantees while independently configuring raw mode, enhanced keyboard
reporting, mouse capture, and cursor visibility. `Runner::with_session_config`
uses that policy without replacing the runner loop.

`Runner` is an optional synchronous event loop for dashboards and small tools.
It owns `TerminalSession`, frame scheduling, Crossterm event translation, and
state-driven redraws. An `Application` keeps state and update policy together;
its pure `view(&self)` may return a `ScopedElement<'_>` that borrows that state
for exactly one frame. The initial frame is painted once; a tick or input only
repaints when update returns `UpdateResult::Dirty`, while resize always repaints
and a `RedrawHandle` can wake the loop from another thread:

```rust,ignore
use std::time::Duration;
use tuika::prelude::*;

let runner = Runner::new(RunnerConfig {
    tick_rate: Duration::from_secs(2),
    ..RunnerConfig::default()
});
impl Application for Stats {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        match signal {
            Signal::Tick if self.refresh() => UpdateResult::Dirty,
            Signal::Event(Event::Key(k))
                if k.plain() && k.code == KeyCode::Char('q') => UpdateResult::Exit,
            _ => UpdateResult::Clean,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        element(Text::raw(self.summary()))
    }
}

let mut app = Stats::default();
runner.run(&Theme::default(), &mut app)?;
```

`UpdateResult::Clean` leaves an input available to runner defaults such as text
selection. Return `Consumed` when the application handled it without changing
the frame, `Dirty` when handling changed the frame, or `Exit` to stop.

Every run method takes a `FrameSource`, and there are exactly two: `&mut app`
for an `Application`, as above, or `from_fn(&mut state, view, update)` for the
closure form over an owned `Element` tree. The
[`borrowed_app`](examples/borrowed_app.rs) example implements a custom `View`
that directly borrows a `String` from its application.

`Runner::with_clock` replaces the default `SystemClock` when a replayable host
or deterministic test owns monotonic time. The same `Clock` boundary drives
`SelectionState::handle_with_clock`, so double-click timing never has to depend
on wall-clock sleeps. Frame animation, keymap timeouts, and toast expiry remain
explicitly host-driven and therefore need no internal clock.

`RunnerCore` is the runtime-neutral state machine underneath both runners. It
turns dirty/clean/exit results and external invalidations into
`RunnerAction::{Wait, Render(frame), Exit}` without knowing about Crossterm,
Tokio, clocks, or terminal backends.

`AsyncRunner` (behind `features = ["async"]`) uses the same state/view/signal
model for applications that already have a Tokio runtime — anything doing
network or disk I/O — and lets its update closure `.await`. It ties
`TerminalSession`, `paint`, and `translate_event` to crossterm's async
`EventStream` and a tick timer in one `tokio::select!`, so the host keeps a
single event loop. Its update closure returns the same
`UpdateResult::{Clean, Consumed, Dirty, Exit}` as `Runner`, so awaited work only
rebuilds and repaints when it actually changes visible state. `AsyncApplication`
is its `Application` twin — the same borrowed-view boundary, with an awaiting
`update` — and `&mut app` is a `FrameSource` for it just as it is for `Runner`.

`run_with_messages` adds a typed host stream beside terminal events and ticks.
Messages arrive as `Signal::Message`, so a background producer can wake and
update the UI immediately without `Arc<Mutex<_>>`, fabricated keys, or a short
polling interval — and the frame source is the same one `run` takes, at
`Signal<M>` instead of the default. The lower-level `run_driven_by` accepts a
caller-owned terminal and both streams, which also makes completion and error
behavior deterministic under `TestBackend`.

`Signal<M>` is that one signal type: `M` defaults to the uninhabited
`Infallible`, so a loop with no message stream has no `Message` variant to
handle and a two-arm `match` stays exhaustive. What is left in the method names
is only where the terminal and the input come from — `run`, `run_with_backend`,
`run_with_messages`, `run_driven_by`. The runtime is the runner type; everything
else is `RunnerConfig` or a builder method.

`run_driven_by` is the loop with nothing around it — no session, no terminal
construction, no stdout-facing work — and both runners build their other entry
points on it. It is also how you test an application end to end: give it a
`TestBackend` and, on the synchronous side, `scripted_events([...])`, and the
whole loop runs with no tty, no raw mode, and no real clock. A host that already
owns its terminal and input can use it the same way; implement `EventSource` for
a custom input.

Enabling `async` adds Tokio (timer + `select!`) and crossterm's `event-stream`
feature; it stays off by default so sync-only hosts pull in no runtime. The
[`async_dashboard`](examples/async_dashboard.rs) example demonstrates that
variant with no shared state at all.

## Native terminal progress

`term::progress::TerminalProgress` emits the OSC 9;4 sequence, which drives the
terminal's own progress indicator — a bar across the top of the window in
Ghostty, the taskbar in Windows Terminal / ConEmu, and similar in
WezTerm / Konsole / mintty. It is out-of-band (no cursor movement, no cells),
so it works in both the inline and full-screen renderers; terminals that don't
understand it ignore the sequence. A host typically shows it (indeterminate)
while long work runs and clears it when idle.

## Images

`Image` paints real pixels — an avatar, a chart, a rendered diagram — over the
cells it reserves, using whichever terminal graphics protocol
`ImageSupport::detect()` finds: **Kitty** (Kitty, Ghostty, WezTerm, Konsole),
**iTerm2**, or **Sixel** (foot, xterm +sixel, mlterm, contour). Terminals with
none show the alt text, so the same view tree renders everywhere.

<img src="https://raw.githubusercontent.com/everruns/tuika/v0.11.0/docs/demos/image.svg" width="880" alt="Two terminal windows side by side: on a Kitty/Ghostty/WezTerm/Konsole terminal an Image view renders a red/green gradient in place; on every other terminal the same view shows a dimmed italic '[image: a red/green gradient]' placeholder.">

Decoding stays in the host — a heavy dependency, kept out like the highlighter
boundary — so you hand in raw RGBA via `ImageData::from_rgba` and `tuika` owns the
protocol encoding (base64, PNG, and Sixel encoders are inline, so no image-codec
dependency). `Runner` detects the terminal protocol, collects placements, and
emits pixels after each cell frame.

```rust
use tuika::prelude::*;
use tuika::term::image::ImageData;

let data = ImageData::from_rgba(2, 2, vec![0u8; 2 * 2 * 4]).unwrap();
let _image = Image::new(data, 20, 10)      // 20×10 cells on screen
    .alt("a 2×2 swatch");                  // shown where graphics aren't supported
```

Custom hosts that call `paint_with_context` directly can install
`ImageSupport` and an `ImageLayer` with `RenderCtx::with_image_graphics`, then
emit and clear the layer after flushing the cell frame.

Markdown `![alt](url)` renders too, in both the one-shot `Markdown` view and the
streaming `MarkdownState`: attach a host `ImageResolver` (URL → `ImageData`, the
same boundary as the highlighter) and resolved images become real pixels — a
link-styled placeholder for the rest, never a dropped URL.

To check support across every terminal feature in one place — `graphics`,
`hyperlinks`, `clipboard`, `progress`, `truecolor` — use `Capabilities`:
`Capabilities::from_env()` is an instant advisory guess, and
`Capabilities::query(timeout)` adds a Device Attributes probe that confirms Sixel
(the one protocol the environment can't reliably reveal).

## Mouse, selection, and clipboard

Mouse handling stays with the terminal by default, including in alternate-screen
mode. That preserves native OSC 8 link activation, click-drag selection, and
terminal scrolling. An app that needs pointer or wheel events opts into capture
with `ScreenMode::Alternate.with_mouse_capture()`,
`ScreenMode::split_footer(rows).with_mouse_capture()`, an enabled
`TerminalSessionConfig::mouse_capture`, or
`AltScreen::enter_with_mouse_capture()`.

Capture is a deliberate trade: the terminal stops activating OSC 8 links and
performing its own selection/scrolling because it hands those mouse events to
the app instead. `Runner` and `AsyncRunner` then restore selection over the
final rendered grid: a plain left drag highlights text, a same-cell double
click selects a word, and releasing copies through OSC 52. Wheel events reach
application scrolling. An
application claims a mouse gesture by returning `UpdateResult::Consumed` (no
repaint) or `UpdateResult::Dirty` (repaint), and can disable the default
entirely with `with_text_selection(false)`.

Hosts with their own loop use the `mouse` module to build the same affordances:

- **Text selection.** `SelectionState` turns a left-button `Down → Drag → Up`
  gesture into a `SelectionRange` (a plain click selects nothing; a new press
  clears the old selection). `selected_text(buffer, area, range)` reads the text
  back out of the rendered `ratatui::Buffer` — linear/stream selection like a
  terminal's own, wide glyphs intact — and `mouse::paint_selection(buffer, area, range,
  style)` paints it in. A same-cell double click selects a word;
  `handle_with_clock` accepts a virtual monotonic `Clock`, while `handle` uses
  `SystemClock`.
- **Application link fallback.** `ctrl_click_url` resolves an OSC 8 target or
  bare URL under a captured Ctrl-click. The host must open it itself. This is
  opt-in fallback behavior for an app that chose capture, not a replacement for
  native terminal activation.
- **Clicks and regions.** `HitMap<T>` maps screen rects to values (a button, a
  link, a row); the last-pushed match wins, so children/overlays registered
  after their parents take precedence. `ClickTracker` turns a same-cell
  `Down`/`Up` into a `Click` and lets an intervening drag cancel it.
- **Clipboard.** `clipboard::write(out, text)` copies via **OSC 52**
  (`clipboard::osc52` is the pure encoder) — no platform clipboard library,
  works over SSH. Same tmux caveat as OSC 8: needs `allow-passthrough on`.

The enriched event model carries what selection and clicks need: `MouseKind` is
`Down/Up/Drag(MouseButton)`, `Moved`, and `ScrollUp/Down/Left/Right`, and every
`Mouse` reports `shift/ctrl/alt`. **Shift-drag** is deliberately left to the
terminal — most emulators use it to bypass app mouse capture for a native
selection — so a host should act on `plain()` left-drags.

**Touch** arrives as mouse events: terminal emulators translate a tap to a
`Down`+`Up` and a swipe to scroll or a drag, so touch flows through this same
path — there is no separate touch event to handle.

> See the [terminal features guide](https://github.com/everruns/tuika/blob/v0.11.0/docs/features.md) for these
> terminal-integration capabilities — OSC 8 hyperlinks, mouse selection and
> clicks, OSC 52 clipboard, OSC 9;4 progress, and Kitty/iTerm2/Sixel images —
> plus `Capabilities` detection, with demos and runnable examples.

## Testing your UI

Rendering is deterministic, so UI built on tuika can be tested without a real
terminal or `TestBackend` setup. The [`testing`](https://docs.rs/tuika/latest/tuika/testing/index.html)
module draws a `View` into an in-memory ratatui `Buffer` and reads it back:

- `render(view, width, height, &theme) -> Buffer` — draw once at a fixed size.
- `render_with_sheet(view, width, height, &theme, sheet) -> Buffer` — the same
  harness with an explicit stylesheet.
- `grid(&buffer) -> String` — the buffer as a plain glyph grid, ready for a
  snapshot assertion.
- `render_sizes(view, sizes, &theme) -> Vec<Buffer>` — the same view across a set
  of sizes, for resize and degenerate-size sweeps.
- `TestHarness<State>` — drive `Signal`s through state/update/view functions,
  resize deterministically, and receive a buffer only for dirty updates.
  `render_app` / `step_app` do the same for an `Application`, including scoped
  views and mandatory resize redraws.

```rust
use tuika::testing::{grid, render};
use tuika::Theme;

let buffer = render(my_view.as_ref(), 20, 3, &Theme::default());
assert!(grid(&buffer).contains("expected text"));
```

For static command output that should remain in scrollback, `render_once` and
`write_once` measure and render a view as ordinary ANSI-styled UTF-8. They do
not enter raw mode, capture input, hide the cursor, or own a screen.

## Used in

- [**yolop**](https://github.com/everruns/yolop) — a terminal coding agent whose
  experimental full-screen renderer is built on tuika.
- [**LLMSim**](https://github.com/chaliy/llmsim) — an LLM traffic simulator whose
  live stats dashboard is a tuika screen.

See the [showcases](https://github.com/everruns/tuika/blob/v0.11.0/docs/showcases.md) for a recording of each. Building
something on tuika? Open a PR adding it here.

## Compatibility

- Minimum supported Rust version: **1.88**, declared as `rust-version` and
  checked in CI.
- Tuika 0.x follows Cargo semver: minor releases may make deliberate breaking
  API changes; patch releases do not.
- Ratatui and Crossterm are part of Tuika's public interoperability surface.
  Tuika builds against `ratatui-core` directly, not the `ratatui` umbrella; the umbrella re-exports that same `ratatui-core`, so a
  matching `ratatui` minor line in your application resolves to one shared
  `ratatui-core` and Cargo deduplicates the core types. Widgets such as
  `Block`/`Paragraph`/`Table` live in `ratatui-widgets` (pulled in by your
  `ratatui` dependency, not by tuika) and compose through
  [`Surface::render_ratatui`](https://docs.rs/tuika/latest/tuika/surface/struct.Surface.html#method.render_ratatui)
  and [`RatatuiView`](https://docs.rs/tuika/latest/tuika/interop/struct.RatatuiView.html),
  whose boundary is a raw `ratatui-core` `Buffer`.

## Extending

tuika is extended from your own crate — no fork, no registration step, no trait
the built-ins get that yours don't:

- **Custom components.** Implement [`View`](https://docs.rs/tuika/latest/tuika/view/trait.View.html)
  on your own type and splice it anywhere with `node(your_view)`, or hand it to
  any container — they accept any `impl View`. The built-in components are on
  equal footing with yours; nothing special-cases them.
- **Existing Ratatui widgets.** Wrap one in `RatatuiView` rather than
  reimplementing it — see [Ratatui interoperability](#ratatui-interoperability).

The [`view!`](#declarative-dsl-view) DSL reaches your components through the same
`node(...)` escape hatch, so they compose exactly like the built-ins.

## Contributing

Issues and pull requests are welcome at
[everruns/tuika](https://github.com/everruns/tuika). See
[CONTRIBUTING.md](CONTRIBUTING.md) for the local checks (`cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features`) and the commit and review conventions.

The separately published companion crates live in this repository:

- [`tuika-charts`](crates/tuika-charts/) renders one line/bar/area/scatter/step grammar as
  smooth terminal graphics or a portable Unicode cell plot.
- [`tuika-codeformatters`](crates/tuika-codeformatters/) supplies the
  tree-sitter `Highlighter`.
- [`tuika-mermaid`](crates/tuika-mermaid/) renders Mermaid fences as Unicode
  terminal diagrams through mmdflux.
- [`tuika-html`](crates/tuika-html/) lays out block-level HTML with html5ever —
  inside Markdown through the `MarkdownBlockRenderer` boundary, or standalone
  through its own [`Html`](https://github.com/everruns/tuika/blob/v0.11.0/docs/components/markdown-code.md#html) component.

All four keep specialized rendering and heavier parsers or grammars out of
tuika core.

## License

MIT — see [LICENSE](LICENSE).
