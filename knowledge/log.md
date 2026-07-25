# Knowledge Log

Significant changes to tuika's durable knowledge are recorded here, in the same
change that makes them — an entry says why the knowledge moved, and that reason
is rarely recoverable later. Routine wording, formatting, and link fixes do not
need entries.

## 2026-07-25 — Demo format follows whether motion carries information

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

## 2026-07-25 — Positioned as the default Rust TUI application framework

- The project's goal is to become the default framework Rust developers build
  terminal *applications* on, but public material described a "small composable
  toolkit" — accurate about the code, silent about the ambition, and easy to
  read as one more widget crate.
- Recorded the positioning in [Product goal](specs/goal.md) and led with it in
  the README and the crates.io description/keywords, with the guard rails that
  keep the claim honest: additive to ratatui rather than competing with it, and
  backed by runnable examples and the showcases rather than asserted popularity.

## 2026-07-25 — Screen modes: a split footer over live scrollback

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

## 2026-07-25 — docs.rs is a build CI has to rehearse, not assume

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


## 2026-07-25 — The tag history starts after the extraction

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

## 2026-07-25 — Non-Unix CI covered the workspace, not just the root package

- CI's macOS and Windows legs ran Cargo's default package scope, which quietly
  exempted `tuika-codeformatters` — the only member that compiles C — from every
  non-Unix platform it is published for. Scoped both legs to `--workspace`.
- [Testing](processes/testing.md) gained a *Platform coverage* section recording
  why: this is the same blind spot as the MSRV, where local development cannot
  reveal the break and the CI invocation is the entire guarantee. Worth stating
  because the failure mode is silence — a too-narrow scope reports green.

## 2026-07-25 — TerminalSession makes modified keys real

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

## 2026-07-25 — Markdown fences can replace source with rich blocks

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

## 2026-07-25 — A theme can be inherited from the terminal

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

## 2026-07-25 — Markdown's two passes become its file layout

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
## 2026-07-25 — Markdown gets a guide of its own

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

## 2026-07-25 — The crate root becomes a decision, not an accumulation

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

## 2026-07-25 — The bundle now states and enforces its own upkeep

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

## 2026-07-25 — Process concepts split out of `specs/`

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

## 2026-07-25 — Codegen shifts the instruction-count gate

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

## 2026-07-24 — Element viewports and composer token seams

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

## 2026-07-24 — Owned composition primitives

- Added owned scenes and dialogs, arbitrary-child two-axis viewports,
  responsive forms, and a closure-backed drawing view. Persistent input and
  control state remains host-owned; the additions are frame descriptions over
  the existing `View`, `Surface`, `OverlaySpec`, `FocusRegistry`, and
  `ScrollState` seams.
- Added semantic success/warning/danger/info styles without expanding the
  public `Theme` struct. The roles derive from each theme's existing syntax
  colors, preserving source compatibility for downstream struct literals.

## 2026-07-24 — Demo recordings can no longer be silently clipped

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

## 2026-07-24 — Showcases

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

## 2026-07-24 — Changelog format: demos in, commit links out

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

## 2026-07-24 — Signed history, signing identities, PR policy

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

## 2026-07-24 — First green CI after extraction

- The `iai-baseline.json` files carried over from yolop measured yolop's copy of
  the code, not this repository's: the scroll benches sat 15–80% above them at
  import, while the markdown and highlighter benches matched. Re-blessed both
  baselines against the imported code. The invariants those benches exist to
  guard — windowed render is O(viewport), paging is O(1) per event — hold at the
  new counts; only the constants moved.
- Recorded two constraints the extraction exposed: snapshot grids are LF-only
  (see [Testing](processes/testing.md)), and the PTY smoke needs the `gallery`
  example built inside the coverage run's instrumented target directory.

## 2026-07-24 — Extraction from yolop

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
