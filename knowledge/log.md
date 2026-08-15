# Knowledge Log

## 2026-08-15

- **The generated website is not part of the published crate**
  - `site/` mirrors public documentation and bundles generated assets for the
    tuika.dev deployment; crates.io renders `README.md` directly and cannot use
    it. The first 0.9.0 release attempt included that tree and exceeded
    crates.io's 10 MiB archive limit, so the package boundary now excludes it
    and tests that boundary explicitly.

- **A run loop that cannot be driven without a terminal cannot be tested**
  - The synchronous loop had no end-to-end coverage for as long as every entry
    point reached crossterm; only its extracted pieces were testable. Giving it
    the same caller-owned-terminal seam the async runner already had made the
    loop itself assertable, and the tests followed immediately.
  - Input is a trait (`EventSource`) rather than a stream on this side, because
    a blocking loop consumes a timeout instead of selecting. An implementation
    owes that timeout: returning "nothing" without waiting would spin.

## 2026-08-14

- **One seam, named axes only for what the names must carry**
  - `FrameSource` makes the frame source an argument, and a message type
    parameter on `Signal` makes the message stream a type rather than a method
    name. The `run*` surface collapses to terminal ownership plus the presence
    of a stream, and the combinations that were missing cannot recur.
  - An uninhabited default message type is what keeps that unification cheap for
    callers: existing matches stay exhaustive, so the generalization is a
    compile-time no-op unless a host actually wants messages.

- **The application seam is a runner axis, not a synchronous privilege**
  - `AsyncApplication` gives the async runner the borrowed-view seam the sync
    runner already had, and carries its signal type as a parameter so a
    message-consuming application is the same seam with a different signal
    rather than a third trait.
  - Both frame sources are kept deliberately: the application seam is the more
    general in what a frame returns and states render purity in its receiver,
    while the closure seam's `FnMut` view is the more permissive in what it
    captures. Neither is legacy.

- **A redraw request must reach a parked loop**
  - `RedrawHandle` registers the waiting task's waker, making it a runner-neutral
    wakeup rather than a flag only a polling loop can notice, and both runners
    expose one. It stays executor-agnostic: the wait is a plain `poll_fn`, not a
    Tokio primitive.

## 2026-08-11

- **Collection interaction state owns the window hosts need to preserve**
  - Selectable lists and tables can resolve an explicit persistent window and
    reuse it for rendering and mouse mapping instead of deriving a centered
    slice independently each frame.
  - Trees keep domain traversal in the host while Tuika owns stable-id
    selection, expansion, ancestry fallback, navigation, and viewport behavior.
  - Async hosts can deliver typed producer messages through the runner's event
    wait, so data arrival itself wakes the UI without shared mutation or polling.

- **Prose owns hyperlinks; literal text does not**
  - `Paragraph`, like `Markdown`, retains complete bare-URL targets before
    wrapping and emits them through the buffer-link path under a web-only
    default policy.
  - `Text`, `Wrap`, `CodeBlock`, and `Console` remain literal. Backend-wide URL
    inference remains an explicit host choice because it cannot distinguish
    prose from logs, source code, or diagnostics.

## 2026-08-10

- **Mouse capture does not remove basic text selection from runner apps**
  - Synchronous and asynchronous runners apply unclaimed plain left drags to
    the final rendered cell buffer, paint the theme selection style, and copy
    through OSC 52 on release while continuing to route wheel input normally.
  - `UpdateResult` now distinguishes unhandled clean input from handled input
    that needs no repaint, giving draggable controls explicit ownership without
    requiring every read-only view to rebuild terminal selection itself.
  - Custom-loop hosts retain the lower-level `SelectionState` path; selection
    remains runner policy rather than hidden component state.

- **A workaround is carried, then dropped; containment is kept**
  - mmdflux 2.6.1 fixes the multi-line decision-node defect
    ([mmdflux#387](https://github.com/kevinswiber/mmdflux/issues/387)), so
    `tuika-mermaid` requires it and renders those fences as diagrams instead of
    recognising the shape up front and degrading them to the code block.
  - The shape-recognition guard was scar tissue tied to one upstream release
    window and left with it. The `catch_unwind` around the layout engine did
    not: it is an unconditional property of the adapter seam, owed to the frame
    whether or not a reachable panic is known.
  - The regression test inverts with the fix — the same inputs now assert every
    label line lands inside the diamond, so a re-broken upstream is caught by
    the test that used to pin the workaround.

## 2026-08-09

- **Paired adaptive chart gallery**
  - Replaced the single portable chart screenshot with matched cell and
    graphics captures for every supported series kind.
  - Documented the pinned Ghostty VHS path that makes adaptive rendering
    visible and reproducible rather than terminal-dependent prose.

- **Runners own adaptive graphics and resize scheduling**
  - Synchronous and asynchronous real-terminal runners detect graphics support,
    inject the per-frame image layer through `RenderCtx`, and emit and clear it
    after Ratatui flushes the cell frame.
  - Images and charts inherit that context automatically; explicit support and
    layers remain available for custom hosts and deterministic tests.
  - Resize bursts update application state immediately but share a 16 ms frame
    deadline, preventing one full image upload per queued resize event.
  - The image and chart examples now describe application content through
    `Runner` instead of reimplementing terminal and graphics lifecycles; the
    chart gallery uses a declarative `view!` flex tree instead of manual `Rect`
    partitioning.

- **Automatic bar domains preserve full edge bars**
  - Automatic x bounds reserve half a bar interval beyond the outermost values;
    explicit domains remain exact clipping bounds.

## 2026-08-08

- **Kitty images remain viewport-neutral**
  - Kitty placements explicitly disable post-display cursor movement. Saving
    and restoring the cursor alone cannot reverse a scroll when a bottom-edge
    image advances past the viewport.

- **Expanded adaptive chart grammar**
  - Added filled-area, scatter, and stepped-line series to both renderers.
  - Expanded the real adaptive example and committed screenshot into a four-chart
    gallery covering every supported series kind.
  - Raised portable geometry above one mark per cell: connected line, step, and
    area edges use dense quadrant subcells, scatter uses Braille point placement,
    and bars and fills remain cell-shaped.
  - Composed portable area fill and edge into one quadrant mask, avoiding both
    whole-cell fill above diagonal edges and gaps immediately below them.

- **tuika.dev uses the public guides as its only documentation source**

- Added a minimal Nimbus website deployed as Cloudflare Worker static assets.
- Site guide routes are generated from public Markdown below `docs/`; generated Markdown is
  ignored so edits cannot fork into a second source of truth.
- Split the component gallery into a complete index and family subpages so a
  reader does not have to load or navigate one thousand-line page.
- Published HTML and Markdown representations, sitemap and structured metadata,
  plus `llms.txt` discovery surfaces for search engines and coding agents.

- **Adaptive charts companion**

- Added `tuika-charts` as a separately published companion so raster charting
  stays out of core.
- Defined one portable numeric grammar (line/bar series, domains, title, legend,
  colors) shared by the graphics and Unicode-cell renderers.
- Reused core `ImageSupport`/`ImageLayer` for capability selection and protocol
  lifecycle; incomplete graphics configuration deliberately falls back to cells.
- Added a generated portable-renderer screenshot beside the runnable example and
  embedded it in the crate README, public chart guide, and `Chart` rustdoc.

## 2026-08-07

- **A block renderer owes the frame totality and a width budget**

- An unrenderable fence is a `None` and the code-block fallback, never an
  unwind: `tuika-mermaid` contains mmdflux's panics rather than assuming a
  third-party layout engine is total. Containment stops at the panic hook,
  which a per-frame renderer must not swap.
- `catch_unwind` is not by itself a workaround for an upstream defect. The
  multi-line decision-node bug ([mmdflux#387](https://github.com/kevinswiber/mmdflux/issues/387))
  also fails silently — a dropped label, a label painted over its own borders —
  so the adapter recognises the shape up front rather than only containing the
  loud case.
- `MarkdownBlockContext::width` is a budget to spend, not decoration. A diagram
  engine sized for vector output overflows a pane, and `Markdown` clips rather
  than scrolls, so the adapter re-lays out at tighter separations until it fits
  — best-effort, since a graph can be irreducibly wider than the terminal.

- **Inline custom regions remain ordinary borrowed views**

- Separate `Fn` measurement and rendering closures adapt into the existing
  `View` contract without a region-specific API or secondary layout language.
- Closures may borrow frame state, but the resulting lifetime still flows
  through `ScopedElement`; owned `Element` cannot hide or extend that borrow.
- The adapter allocates nothing and retains no identity. Heterogeneous
  containers box it exactly as they box named views.

- **Repeated selection pages share one responsive composition**

- Action, agent, permission, and resume pages can reuse one shell preset while
  keeping `SelectState` and input policy in the host.
- Borrowed rows and allocation-derived windowing avoid per-frame collection
  clones and keep the selected row visible after short-height chrome collapse.

## 2026-08-06

- **Application frames may borrow application state**

- The synchronous runner accepts an `Application` whose pure frame builder
  returns `ScopedElement<'_>` from `&self`; persistent mutations remain confined
  to signal updates through `&mut self`.
- Resize signals always invalidate the frame because terminal geometry changes
  layout even when application data does not change. The owned-element closure
  runner remains compatible.

- **Virtual collections keep domain identity in host-owned keys**

- Borrowed keyed tables adapt application rows directly and render only the
  resolved viewport, avoiding a cloned cell model for large tool collections.
- Single and multi-selection store stable row keys rather than indexes. A key
  remains selected while filtered out; hosts explicitly reconcile against an
  authoritative collection when deletion should remove it.
- These keys are domain identity only. Views remain ephemeral and Tuika gains
  no virtual DOM, keyed lifecycle, or retained row components.
- Indirect keyed row sources project visible indices into authoritative storage,
  compare computed composite identity without allocation, and join parallel
  metadata through indexed cells. Owned keys are created only on selection
  changes, not during frame rendering.

- **Composable application chrome remains frame-scoped**

- `AppShell` records one growing main view plus optional ordinary view regions;
  its short-height behavior reuses flex allocation and adds no retained screen,
  navigation, input, or host policy.

## 2026-08-05

- **Agent interaction primitives remain host-owned compositions**

- Filter-ranked completions retain only query and selection state; candidate
  sources, trigger meaning, and accepted replacement behavior remain in the
  host.
- Confirm, choice, multi-choice, and input dialogs assemble existing selection,
  text-input, overlay, and focus primitives instead of adding a modal manager.
- Activity lists describe lifecycle across steps and optionally compose the
  existing progress bar for a measurable running step. They own no scheduler.

## 2026-08-01

- **Markdown block parsing has one open contract**

- Fenced diagrams and raw HTML now arrive as `MarkdownBlock` variants through
  one `MarkdownBlockRenderer` trait and one width/theme/stylesheet context.
- Renderer registration is an ordered chain in one-shot and streaming paths;
  adding another parser-backed block no longer grows parallel component fields
  and methods. HTML fences now inherit the active host stylesheet.

- **Application and layout foundations remain integer-native and runtime-neutral**

- Flex now owns wrapped lines, grow and shrink distribution, cross-line
  alignment, per-child styles, and exact boundary rounding. Intrinsic `Flow`
  and an intentionally small equal-column `Grid` cover common terminal layouts
  without adopting Taffy or CSS Grid.
- Measurement accepts non-exhaustive known-axis and intrinsic-sizing requests;
  existing views adapt through the original method.
- One pure runner state machine drives sync and async shells. Terminal lifecycle
  policies are independently configurable, and a headless application harness
  drives the same signal/update contract without a runtime or terminal.
- Static views can be emitted once as ANSI-styled ordinary output; the DSL now
  composes conditional and repeated children.

- **Character key bindings use logical text**

- Keymap character specs now name the exact Unicode result after keyboard
  layout (`A`, `?`, `ctrl+R`) and reject layout-dependent
  `shift+character` spellings; non-character Shift chords remain explicit.

- **Monotonic time is injectable**

- `Clock` gives gesture recognition and synchronous runner scheduling one
  replaceable monotonic source while preserving system-clock conveniences.
- Animation, toast expiry, and keymap timeout policy remain host-driven; the
  async runner retains Tokio's independently controllable time source.

- **Virtualized collections share one range and scrollbar model**

- `VirtualWindow` now supplies clamped, selection-centered absolute ranges,
  and one `Scrollbar` view draws either axis with overflow-safe geometry.
- Line scroll, item scroll, arbitrary viewports, lists, and tables use those
  primitives. Lists and tables can receive only their visible records while
  retaining absolute selection and full-collection bar position.

- **Interactive components share one state contract**

- Selection, multi-selection, tabs, segmented selection, sliders, scrolling,
  forms, and text input now report one `InputOutcome` vocabulary. Hosts can
  route every component consistently while reading values from the state they
  already own.
- `Default` and zero-argument `new()` no longer disagree about initial
  selection or bottom-follow behavior. Alternate starts are named explicitly,
  and handled boundary keys are distinguishable from events a host may reuse.

## 2026-07-31

- **Async runner uses explicit invalidation**
  - Awaited updates return the same clean/dirty/exit result as synchronous
    updates, so idle ticks and handled-but-invisible events do not rebuild or
    repaint the view.
  - State/view/update semantics are now symmetric across both runner variants.

- **Responsive auxiliary panels stay host-owned**

- A small dock state now resolves one panel to wide dock, narrow focused
  drawer, or hidden passive geometry. It owns no view tree or application data,
  so hosts can reuse the lifecycle without adopting a retained window manager.
- Focused panels are always assigned a visible rectangle; passive panels may
  remain logically open through narrow resizes and reappear when space returns.

- **Overlay placement can follow laid-out targets**

- Popovers, menus, and tooltips can resolve on any side of a probed root view,
  align on the cross axis, keep a gap, and flip when the terminal edge leaves
  more room on the opposite side. Placement remains a scene concern; the
  trigger and overlay do not depend on each other's component types.
- The root paints before scene overlays, so a target uses current-frame layout
  rather than a retained geometry cache. Every result is finally clamped to the
  screen, including tiny and degenerate sizes.

- **A composable visual identity**

- Added tuika's first logo: two offset interface panels meet at a gold anchor,
  pairing the toolkit's base-view/overlay model with Everruns' navy-and-gold
  visual language. The authored SVG is the canonical source and ships with the
  crate because the crates.io README embeds it by relative path.
- Added exact-geometry dark-surface and monochrome SVG variants plus generated
  1024 px PNG exports. Only the README's primary SVG ships in the crate; the
  alternate brand assets remain repository-only.

- **Semantic styling is open and complete**

- Toasts, diffs, and keymap hints now join markdown and panels under one
  stylesheet policy; their palette defaults come from the active theme, while
  existing purpose-built instance overrides remain the final local layer.
- `StyleRole` and `StyleResolver` let hosts and companion crates add namespaced
  semantics without adding fields to tuika. Resolver bundles overlay built-in
  data, and an explicit revision keeps measurement caches correct when a live
  policy changes.

- **Measurement and rendering share one frame context**

- `View::measure` now receives the active `RenderCtx`, and composition helpers
  forward it through the whole tree. Layout can no longer use the default theme
  or stylesheet while the same view renders with host policy.
- Markdown and the companion HTML view resolve one theme/stylesheet pair for
  both halves. `Boxed` now treats stylesheet panel padding as real geometry,
  with an explicit instance padding as the local override.
- Consumer tests can use `testing::render_with_sheet`, so a custom stylesheet is
  asserted through the same in-memory buffer path as production rendering.

- **Borrowed views compose inside ordinary component trees**

- `Element` remains the owned default, while `ScopedElement` carries a frame
  borrow through the same `element`, `view!`, Flex, Boxed, form, viewport, and
  collection composition paths. Hosts no longer need a custom root `View` just
  to nest a borrowed renderer or large model reference.

- **Component geometry uses the grid's real constraints**

- Padded flex containers now measure children against the same inner box they
  later assign, and a flex container's own intrinsic measurement respects its
  declared fixed and percent dimensions. This restores one measure/solve
  contract instead of letting padding and nesting create a second geometry.
- Text input now shares the crate's grapheme-aware terminal-width model for
  wrapping, painting, and cursor placement. Its public coordinates stay as char
  indices, while cursor motion and deletion snap to grapheme boundaries so an
  editing operation cannot split a visible cell.
- Per-frame focus rings discard ids that disappeared and commit the fallback at
  the following frame boundary. Dynamic component trees therefore cannot keep
  routing input to a stale target or resurrect it when it later returns.
- **Single-line input and custom-view vocabulary**
  - Search/command state normalizes newline-bearing setters, paste, and key input
    and exposes borrowed text, keeping render paths allocation-free and pure.
  - `tuika::ui` provides the backend value types required to implement `View`,
    avoiding an otherwise redundant direct host dependency.

- **Keymap-derived responsive help**
  - Active labeled bindings drive whole-item, priority-aware footer fitting and
    a complete scrollable help view.
  - Layer priority is display priority, keeping contextual/modal actions visible
    before global actions when width is constrained.

- **Configurable selection behavior**
  - Selection state now supports opt-in terminal-picker aliases, explicit mouse
    hit-testing, and a separate multiple-selection state.
  - Policies and bounds remain host inputs so reusable state does not assume a
    particular keymap or layout.

- **Synchronous runner state and invalidation model**
  - `Runner` now mirrors the asynchronous runner's owned-state split: immutable
    view construction and mutable signal updates.
  - Repainting is explicit (`UpdateResult::Dirty`) or externally requested;
    periodic ticks alone no longer repaint. This keeps rendering pure and makes
    idle CPU/terminal work proportional to actual changes.

- **Internal follow-ups stay out of GitHub issues**

- Work discovered during maintainer or agent tasks stays in the current PR,
  durable knowledge, or local planning. A new GitHub issue requires an explicit
  maintainer request; existing externally reported issues may still be updated.
  This keeps the public tracker for intentional external coordination rather
  than exposing internal scratch queues.

- **The split-footer demo is a VHS recording in the default palette**

- The asset was drawn on a cold gray of its own — terminal background, window
  chrome, and shell prompt all hand-picked — while every other asset in `docs/`
  is captured against `Theme::default()`. Beside them it read as a screenshot of
  a different program. Every generator now takes its palette from the theme,
  recorded in [Documentation](specs/documentation.md#capture-palette).
- It was a hand-rolled SVG because VHS was assumed to be unable to record
  the mode. It can: ttyd runs the session in a real terminal, and the scrollback
  above the footer — and what is left after `q` — comes through intact. Only the
  image demo genuinely defeats a recorder, because `xterm.js` implements no
  graphics protocol.
- `docs/split-footer.gif` is now captured by
  `scripts/gen-split-footer-demo.sh` at the component gallery's density, so it
  sits beside the demos instead of at its own scale. The PTY/SVG generator stays
  as the recorder-free path and its output is no longer committed.
- It lives at `docs/split-footer.gif`, beside the hero rather than in
  `docs/demos/`: that directory belongs to the `DEMOS` registry — `demo -- check`
  rejects an asset there with no scene behind it — and is excluded from the
  `.crate`, which this recording must be in for the crates.io README to render.

## 2026-07-30

- **Render-time facts stay in render-time components**

- Tables now derive their default row window from the assigned rectangle, and
  wrapped scrolling reflows owned lines at render width. Requiring either
  height or width at construction forced hosts to create wrapper `View`s whose
  only purpose was to pass geometry back into a component.
- Selection state is optional, matching ratatui's model, and per-instance
  selection styles represent local focus/hover/inactive state without turning
  those stateful distinctions into global theme policy.
- ratatui `Line` style remains the base layer under each `Span` style throughout
  these render paths, and boxed titles follow ratatui's inset and truncation.
  These details are part of the interoperability promise, not cosmetic
  preferences.

- **Inline HTML renders; HTML documents remain a non-goal**

- Raw HTML in markdown was dropped whole, so `<b>bold</b>` rendered flat and
  `a<br>b` silently joined two lines — the markup *and* its meaning were lost.
- A fixed whitelist of presentational inline tags now resolves the same
  `StyleSheet` roles as the markdown constructs it mirrors. This adds no
  dependency and no DOM: it is tag-name matching over the string pulldown-cmark
  already isolated, so the crate stays a markdown renderer.
- The markdown non-goal was narrowed rather than deleted. Block-level HTML and
  document layout (DOM, CSS) stay out of core; a host that wants them supplies
  the parser behind a seam, as `FencedBlockRenderer` already allows for a
  fenced `html` block.
- Inline-HTML scopes close at every block end. That is both the sane reading of
  an unclosed tag and what keeps the settled-prefix cache honest — a scope
  outliving a block boundary broke the streamed-equals-one-shot invariant in
  *styles* while the text still matched, which the streaming test now covers.

- **Block HTML renders through a seam, with tuika-html behind it**

- The inline whitelist left `<details>`, `<table>`, and `<div>` blocks dropped,
  which is the majority of the HTML that actually appears in READMEs.
- `HtmlBlockRenderer` follows `FencedBlockRenderer`'s shape but also carries the
  `StyleSheet`, so an implementation resolves the same roles the surrounding
  markdown does rather than inventing colors. `Renderers` bundles both block
  seams so the free functions did not grow a second renderer argument each.
- `tuika-html` is the implementation: html5ever for the tree, tuika's own
  wrapping and stylesheet for the presentation, plus a standalone `Html` view.
  It is a fourth published crate rather than an optional feature, for the same
  reason the other two are.
- Its example runs as a real app rather than printing a rendered grid: styling
  is half of what the crate does, and a plain-text dump discards all of it. The
  recording is a screenshot, since the scene is settled.
- **Documentation placement is a rule, not a habit.** HTML landed correctly and
  was documented by accretion: the block-seam recording sat as an unlabeled hero
  above the crate README's first section, the `Html` component was missing from
  the gallery entirely, and the root README grew code samples and screenshots
  that belong in a guide. Every individual edit looked reasonable; the shape was
  wrong. [Documentation](specs/documentation.md) now states the four rules that
  were only ever implicit — every component appears in the gallery including
  companion-crate ones, the README indexes while guides explain, a demo sits with
  what it demonstrates, and rustdoc carries both a compiling example and the
  demo. Written down because the gallery's own integrity check enforces asset
  *existence*, never placement, so nothing failed while the docs drifted.
- Two renderers for one vocabulary need one *observable* result: the crate's
  own `<sub>`/`<sup>` were left untransliterated, so the same document rendered
  `H₂O` through markdown and `H2O` through tuika-html. Separate implementations
  are fine; a visible divergence is not, and only an example exercising the
  component surfaced it.
- Bounding a *parser* means bounding its input, not its traversal. html5ever
  builds and drops its tree recursively, so deeply nested markup overflows the
  stack before any traversal begins — a capped walk is no defense. Nesting is
  therefore measured on the source bytes and over-nested fragments are refused.
  Found by the shipping security review, not by the test suite, which had only
  probed nesting the input-size bound already rejected.

## 2026-07-25

- **OKF v0.2 compliance**: Declared the bundle version, normalized this log
  into date groups, and extended the validator to enforce reserved-file
  structure.

- **Demo format follows whether motion carries information**

- Component recordings treated GIF as a universal container even when a scene
  never moved. That needlessly quantized font antialiasing and themed colors to
  a 256-color palette. Settled scenes now use full-color PNG screenshots; motion
  scenes remain GIFs because their transitions demonstrate behavior.
- The scene registry declares the format through its existing `animated`
  property, and the integrity check rejects both stale formats and references
  that disagree with the registry. Capture geometry uses VHS's own default
  monospace rather than a locally installed font, and every generator clears
  `NO_COLOR`; neither choice changes tuika's palette or host-agnostic boundary.
- Added one repository-wide regeneration entry point so “all demos” includes
  generated SVGs, companion crates, the Codex example, and external showcases,
  rather than meaning only the component registry by accident.

- **Borrowing stops at the scene root**

- `Element = Box<dyn View>` deliberately remains owned and `'static`, but that
  forced a live application root either to clone large host models each frame or
  to retain its own compositor beside Tuika's owned `Scene`.
- Added `ScopedScene<'_, V>` as the narrow borrowing boundary: one concrete root
  is borrowed for the frame, while overlays remain owned `SceneOverlay`s. Owned
  and scoped scenes share rendering and focus-owner resolution so backdrop,
  clipping, placement, ordering, and focus behavior cannot drift.
- Recorded the choice in [Rendering architecture](specs/architecture.md).
  Lifetime-generic elements would make every container and component carry the
  borrowing policy even though the motivating state naturally belongs at the
  frame root.
- Superseded on 2026-07-31 by `ScopedElement`: containers now stay generic over
  their child type, so borrowed subtrees compose without forcing lifetime
  parameters onto the ordinary owned `Element` path.

- **Positioned as the default Rust TUI application framework**

- The project's goal is to become the default framework Rust developers build
  terminal *applications* on, but public material described a "small composable
  toolkit" — accurate about the code, silent about the ambition, and easy to
  read as one more widget crate.
- Recorded the positioning in [Product goal](specs/goal.md) and led with it in
  the README and the crates.io description/keywords, with the guard rails that
  keep the claim honest: additive to ratatui rather than competing with it, and
  backed by runnable examples and the showcases rather than asserted popularity.

- **Screen modes: a split footer over live scrollback**

- tuika could only own the whole terminal. That rules out the shape a
  long-running CLI wants: a live footer over output the user keeps — scrollable,
  selectable, still there after the tool exits.
- Added `ScreenMode` (`Alternate` | `SplitFooter`) as the host's first decision,
  and two publishing paths above a footer, since the footer owns the cursor and
  `println!` would land anywhere: `Scrollback` for producers on another thread,
  `publish_block` for the render loop itself, whose blocks may hold caches that
  are not `Send`. Blocks are committed once and never repainted, which is what
  makes them the terminal's content rather than tuika's.
- Judgement calls recorded in [Screen modes](specs/screen-modes.md): a split
  footer does not capture the mouse by default, the footer is pinned to the
  bottom rather than left where ratatui anchors an inline viewport, and its
  height is fixed for the terminal's life.
- Driving the mode through a real pty overturned an assumption worth recording:
  ratatui's `scrolling-regions` looked like the obvious optimization (no
  viewport repaint per published block) and is the wrong trade — a terminal
  discards rows scrolled out of a DECSTBM region instead of adding them to its
  scrollback. `TestBackend` models it the other way, so only the PTY layer could
  have caught it. The feature stays declared as a compatibility mirror, and CI
  now runs the suite on the default feature set too, which it previously only
  compiled.

- **docs.rs is a build CI has to rehearse, not assume**

- 0.4.0 shipped with no documentation on docs.rs: `src/lib.rs` gated on
  `feature(doc_auto_cfg)`, removed in Rust 1.92, behind `cfg_attr(docsrs, …)`.
  That attribute exists only under `--cfg docsrs`, which nothing outside docs.rs
  sets — so `cargo doc` was green everywhere while the one build that matters
  failed in twelve seconds.
- [Documentation](specs/documentation.md) now records docs.rs as a *different*
  build rather than a stricter one, and CI builds the docs twice: once the way a
  consumer does, once the way docs.rs does. Recorded because the failure mode is
  silence — a library's documentation surface can be entirely absent while every
  gate reports success.


- **The tag history starts after the extraction**

- tuika 0.1.0–0.4.0 and `tuika-codeformatters` 0.1.0–0.2.0 were all published
  from the yolop workspace: every publish timestamp on crates.io precedes this
  repository's first content commit, and the published 0.4.0 tarball still
  carries `AGENTS.md` and `scripts/` because it predates the `exclude` list. The
  extraction also reworded documentation as it moved the sources, so no tree in
  this history matches a published one.
- [Release](processes/release.md) now states that those versions intentionally
  have no tag and no GitHub Release here, and that "no previous tag" is a
  legitimate state for release tooling rather than a shallow clone. Recorded
  because the temptation to backfill tags will recur, and the reason not to —
  a tag is a provenance claim that `git describe`, compare links, and tag-pinned
  demo URLs all trust — is not recoverable from the tag list itself.
- [Documentation](specs/documentation.md): the crate/GitHub asset split is a rule
  per *published crate*, not per repository, and what decides it is how that
  crate's own README reaches the asset — relative path means crates.io renders
  from the tarball and it must ship, an absolute `raw.githubusercontent.com` URL
  means the packaged copy is never read and must not. `tuika-codeformatters` was
  shipping 428 KiB the second way, 94% of its download, while `tuika-mermaid`'s
  small recording is correctly kept. Stated as the criterion rather than as
  "every member excludes its GIFs", which would have been wrong for the very next
  member added.

- **Non-Unix CI covered the workspace, not just the root package**

- CI's macOS and Windows legs ran Cargo's default package scope, which quietly
  exempted `tuika-codeformatters` — the only member that compiles C — from every
  non-Unix platform it is published for. Scoped both legs to `--workspace`.
- [Testing](processes/testing.md) gained a *Platform coverage* section recording
  why: this is the same blind spot as the MSRV, where local development cannot
  reveal the break and the CI invocation is the entire guarantee. Worth stating
  because the failure mode is silence — a too-narrow scope reports green.

- **TerminalSession makes modified keys real**

- `TextInputMode` already assigned different behavior to `Enter` and
  `Shift+Enter`, but the full terminal session never requested a protocol that
  could distinguish them. The advertised composer behavior therefore did not
  work end to end.
- `TerminalSession` now owns enhanced keyboard reporting as part of the same
  lifecycle as raw mode, mouse capture, and the alternate screen. The transport
  policy follows the compatibility constraints proven by Codex: suppress event
  types for iTerm2 and tmux's xterm format, and enable `modifyOtherKeys` for
  tmux CSI-u.
- Teardown pops exactly the level tuika pushed instead of globally resetting
  keyboard reporting, preserving a mode installed by an embedding host.

- **Markdown fences can replace source with rich blocks**

- Split fenced-block extension into two contracts: `Highlighter` remains
  line-preserving token styling, while `FencedBlockRenderer` may replace a
  fence with width-aware styled lines. Conflating them would either break the
  highlighter invariant or make every syntax highlighter implement diagram
  layout concerns.
- Kept parsed fences width-independent by retaining source plus the normal code
  fallback and invoking rich rendering during flattening. This preserves
  `MarkdownState`'s settled-prefix cache: a settled diagram renders once per
  width, while resize correctly relays it out.
- Added `tuika-mermaid` as a separate mmdflux-backed crate. Mermaid parsing and
  layout are useful but heavyweight and independently versioned, so they follow
  the same companion-crate boundary as tree-sitter highlighting.

- **A theme can be inherited from the terminal**

- Added a third source for a `Theme`, beside the bundled presets and a host's own
  literal: the terminal the application was launched in. [Styling](specs/styling.md)
  now states where the line falls between what an inherited theme *reports* and
  what it *derives* — reported colors verbatim, derived tones blended and
  contrast-guarded, invented hues by convention — because that boundary is the
  whole design and is not recoverable from the code.
- Recorded the constraint that reading a terminal's configuration file is out of
  scope. The escape query is the supported interface; parsing Ghostty's or
  kitty's config would couple tuika to another project's format and its
  theme-resolution rules.
- [Out-of-band escapes](specs/out-of-band.md) gained a third family, and
  `term::palette` alongside `term::capabilities` to hold it. The other five
  capabilities *tell* the terminal something; a query is *asked*, and its answer
  arrives on stdin among the user's keystrokes. That difference carries two rules
  worth keeping: a probe is fenced by the Device Attributes request so an
  unsupported query costs a round-trip rather than a timeout, and it must run
  once at startup and stop reading at the fence so it cannot eat input.
- Restated a term that now means two things in this repository. The styling
  non-goal "no cascade, inheritance, or selectors" was about the *rule* layer;
  terminal inheritance produces a plain `Theme` and involves no cascade.

- **Markdown's two passes become its file layout**

- Splitting the 2293-line markdown module surfaced the invariant that made the
  split obvious: rendering is parse-then-flatten, separated by *what they know
  about width*. Parsing is width-independent; flattening fits lines to a width.
- Recorded that in [Markdown](specs/markdown.md), because it is the reason both
  caches work: the settled-prefix cache can hold parsed blocks across frames
  only because they carry no width, and a resize re-flattens without
  re-tokenizing. A parser that wrapped as it went would make every resize a full
  re-parse.
- The files now follow the passes rather than the vocabulary. Submodules stay
  private per [Public API surface](specs/api-surface.md) — the split is an
  implementation detail, and `components::markdown` remains the one path in.
- **Markdown gets a guide of its own**

- The component gallery is one entry per component, but markdown's user-facing
  surface is much larger than one entry: streaming, GFM table fitting, the
  highlighter seam, link policy, and images. The table renderer in particular
  had no recording at all, so the feature was documented in prose and ASCII art
  while every other component had a demo.
- Added `docs/markdown.md` and recorded a `markdown_table` scene. Recorded the
  precedent in [Documentation](specs/documentation.md): a component earns a
  guide when its surface outgrows a gallery entry, the gallery keeps the entry
  and links out, and such a guide reuses `DEMOS` scenes rather than owning
  parallel assets — which puts it inside the `demo -- check` reference gate.

- **The crate root becomes a decision, not an accumulation**

- The public tree had grown by accretion: 30 flat public modules plus 167 names
  re-exported to the crate root, so nearly every type had two equally valid
  paths and neither was canonical. Symptoms were hand-prefixed names
  (`ASCII_FONT_HEIGHT`, `qr_encode`), a `highlight` module and a `highlight`
  function colliding at the root, and `Overlay`/`OverlaySpec` living in
  different modules.
- Wrote [Public API surface](specs/api-surface.md) to state what each level
  owns — root = framework spine, `components` = widgets, `term` = escapes
  outside the cell grid, `prelude` = the one-line import — and the rules that
  place a new item: one canonical path per item, a module goes public only when
  the flat namespace fails a name it owns, and a `cfg` is never a reason to
  split a module.
- Consequences recorded in the affected concepts: the out-of-band escapes are
  now one family under `term` ([Out-of-band](specs/out-of-band.md)), images
  split protocol from view along that same line ([Images](specs/images.md)), and
  test scaffolding moved out of the crate root into `src/tests/`
  ([Testing](processes/testing.md)).

- **The bundle now states and enforces its own upkeep**

- The rule that concepts are updated by the change that invalidates them lived in
  `AGENTS.md`, three skills, and the pull-request template — everywhere except
  the bundle it governs. `index.md` read as consumption-only, so an agent that
  arrived by a grep hit or a link, rather than through `AGENTS.md`, got the read
  contract and no write contract. The index now carries the maintenance rule
  itself, which also gives the concepts a single stated update trigger instead of
  twelve unstated ones.
- `scripts/validate_okf.py` fails on a concept the index does not list. This
  enforces only the mechanical half — a moved or added file cannot become
  unreachable — deliberately leaving "did this change need a concept update?" to
  review, because a diff-shaped check for it would fire on the majority of
  changes that legitimately need nothing and train people to ignore it.
- `CONTRIBUTING.md` explains the template's Knowledge section; before, an
  external contributor met that checkbox with no explanation anywhere in their
  path. [Documentation](specs/documentation.md) now classifies `CONTRIBUTING.md`
  as contributor material and states that the no-internal-links rule covers
  `README.md` and `docs/` and nothing else — previously that scope was only
  discoverable by reading the CI grep.

- **Process concepts split out of `specs/`**

- The bundle mixed two kinds of knowledge under one directory: what tuika *is*
  (goal, architecture, and the capability concepts) and how maintainers *work on
  it*. The frontmatter already said so — four concepts carried
  `type: Process Specification` — but the directory did not, so an agent reading
  `knowledge/specs/` had no way to tell a product invariant from a workflow
  requirement without opening each file.
- [Testing](processes/testing.md), [Shipping](processes/shipping.md),
  [Maintenance](processes/maintenance.md), and [Release](processes/release.md)
  now live in `knowledge/processes/`. `specs/` holds product and architecture
  concepts only, plus the [Documentation](specs/documentation.md) policy, which
  governs the published surface rather than a maintainer workflow.
- Content is unchanged; this is a reclassification. The OKF validator walks the
  bundle recursively and does not care about directory names, so the split is a
  readability contract rather than a tooling one.

- **Codegen shifts the instruction-count gate**

- Landing `ItemScroll` and the composer token seams turned the `iai` gate red on
  `main`: seven of nine benchmarks up 3.7–5.5%, including the markdown ones,
  whose measured path (`markdown.rs`, `text.rs`, `style.rs`, `surface.rs`) was
  byte-identical to the parent commit.
- Isolating it showed why: the parent reproduced the committed baseline
  *exactly*, `textinput.rs` alone accounted for ~2.8%, and adding `ItemScroll`
  took it to ~4.5%. Growing the crate re-partitions its codegen units, so
  unrelated modules change what gets inlined on a hot path. The `scroll.rs`
  refactor in the same change cost one instruction.
- Recorded the isolation procedure in [Testing](processes/testing.md) so the
  next red gate is diagnosed rather than blessed on a hunch — a shift that
  survives the isolation is a real regression.

## 2026-07-24

- **Element viewports and composer token seams**

- Building a coding-agent TUI as an example (`examples/codex/`) surfaced two
  places where the toolkit forced a host to hand-draw: a transcript could only
  hold pre-wrapped lines, and a composer could only paint one uniform style with
  no notion of the `@`/`/` tokens every such app needs.
- Closed both as *seams*, not features: `ItemScroll` (a viewport over
  `Element`s, scrolled by row) and `Trigger`/`Token`/`TextSpan` (tuika delimits
  tokens and paints host-computed ranges; the meaning of a trigger character
  stays with the application). See [architecture.md](specs/architecture.md).
- Recorded the constraint that made `ItemScroll`'s API shape non-obvious: item
  heights depend on the render width, so the scrollbar column is reserved
  whenever the bar is enabled, and `measure_height` takes the same `scrollbar`
  flag — otherwise a host's clamp and the paint would disagree about content
  height.

- **Owned composition primitives**

- Added owned scenes and dialogs, arbitrary-child two-axis viewports,
  responsive forms, and a closure-backed drawing view. Persistent input and
  control state remains host-owned; the additions are frame descriptions over
  the existing `View`, `Surface`, `OverlaySpec`, `FocusRegistry`, and
  `ScrollState` seams.
- Added semantic success/warning/danger/info styles without expanding the
  public `Theme` struct. The roles derive from each theme's existing syntax
  colors, preserving source compatibility for downstream struct literals.

- **Demo recordings can no longer be silently clipped**

- Six gallery GIFs (`qr`, `ascii_font`, `diff`, `slider`, `timeline`,
  `hyperlink`) shipped with content cut off: each scene's recorded height is a
  hand-picked number in the `DEMOS` registry, and outgrowing it clips the
  recording without failing anything.
- Root cause: the tape heights were computed from a cell size the recorder did
  not actually use, so a scene's `rows` was never the number of rows it got. The
  harness now pins each scene to a fixed frame and the tapes ask for slightly
  more room than that, making font metrics irrelevant to what a demo shows.
- `demo -- check` now asserts a scene fits the frame it records into, and scenes
  that overflow by design declare it in the registry. `--dump` renders at the
  scene's recorded geometry, so the pre-record preview shows the real framing
  instead of a roomier one. See [Documentation](specs/documentation.md).

- **Showcases**

- Added `docs/showcases.md`: applications built on tuika (yolop, LLMSim), each
  with a recording of its real UI. It answers a question the component gallery
  cannot — what the toolkit looks like carrying a product — and gives the
  README's *Used in* list somewhere to point.
- Recorded it as an explicit exception to the "every visual is generated from
  checked-in code" rule, with two constraints written into
  [Documentation](specs/documentation.md): a showcase must record
  deterministically and offline, and it must not misrepresent the host. Both
  scenes are driven by a local LLMSim, so no provider key or live model is
  involved.

- **Changelog format: demos in, commit links out**

- Release notes now **show** the release: `### Highlights` embeds a VHS
  recording of the one or two most TUI-centric features. The recordings are
  ordinary `DEMOS` gallery scenes — a release improves the permanent gallery
  rather than leaving one-off assets behind — and they are the single place that
  pins a `raw.githubusercontent.com` URL to the release tag instead of `main`,
  so re-recording a scene cannot rewrite what a past release appeared to ship.
  Consequently `CHANGELOG.md` stays outside the `demo -- check` reference gate.
- Highlights are ordered user-facing functionality first, then a one-line
  performance note and a one-line security note, each carrying a number or a
  stated impact.
- Dropped commit links and the `compare/vA.B.C...vX.Y.Z` line from
  `### What's Changed`. This repository rewrites history when it has to, which
  rots every SHA-based URL baked into a published release note; pull-request
  references survive a rewrite, so a bare `(#42)` is still allowed.
- Contributor attribution is now the exception rather than the rule: ` by
  @handle` appears only for authors other than @chaliy, since the maintainer is
  the default and repeating it is noise.

- **Signed history, signing identities, PR policy**

- Rewrote the repository's history so every commit is signed and verifies.
  Signing is now a hard requirement rather than a convention; a rewrite that
  drops signatures is a defect, since a later commit cannot restore them.
- Maintainers use their existing GitHub-recognized SSH or OpenPGP identity.
  Doppler (`everruns-dev` / `dev`) holds a backup OpenPGP key, not a mandatory
  signing path. Shared repository secrets remain in Doppler; personal SSH
  identities remain in their normal OS/Git setup.
- Narrowed the pull-request requirement to **external contributions**.
  Maintainers land directly on `main`. The bar for a change is unchanged either
  way, so the shipping outcomes were reworded around "landing" rather than
  "merging" a PR.

- **First green CI after extraction**

- The `iai-baseline.json` files carried over from yolop measured yolop's copy of
  the code, not this repository's: the scroll benches sat 15–80% above them at
  import, while the markdown and highlighter benches matched. Re-blessed both
  baselines against the imported code. The invariants those benches exist to
  guard — windowed render is O(viewport), paging is O(1) per event — hold at the
  new counts; only the constants moved.
- Recorded two constraints the extraction exposed: snapshot grids are LF-only
  (see [Testing](processes/testing.md)), and the PTY smoke needs the `gallery`
  example built inside the coverage run's instrumented target directory.

- **Extraction from yolop**

- tuika and `tuika-codeformatters` moved out of the `everruns/yolop` workspace
  into this repository. tuika is now the root package of its own workspace; yolop
  consumes both crates from crates.io like any other host.
- Established `knowledge/` as tuika's OKF bundle, seeded from the tuika-owned
  concepts that previously lived in yolop's bundle (keymap, image rendering) plus
  newly written concepts for the toolkit's goal, architecture, markdown,
  styling, out-of-band escapes, and testing.
- Rewrote the shipping, maintenance, release, and documentation process specs
  for a published-library repository: no provider credentials, no Homebrew tap,
  two crates published in dependency order, and a real MSRV gate.
- Added a repository-owned PTY smoke test (`tests/pty_smoke.rs`) driving the
  `gallery` example, replacing the equivalent coverage that lived in yolop's
  test suite and could not follow the crate.
