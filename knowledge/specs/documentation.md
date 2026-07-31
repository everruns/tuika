---
type: Policy
title: Documentation Specification
description: Defines the boundary between tuika's public documentation and its internal project knowledge, and the contract for its generated visual assets.
---

# Documentation Specification

## Purpose

This specification defines the boundary between tuika's public documentation and
its internal development memory. Public documentation must help someone build a
terminal UI with tuika without requiring knowledge of repository internals.

## Information architecture

- `README.md` is the public entry point **and the crates.io README**. It
  explains the model, presents the primary APIs, and links to the guides. Its
  relative links resolve against the repository, so they must stay valid there.
- `docs/` is public, task-oriented documentation. A page must stand on its own
  for someone who added `tuika` to their `Cargo.toml`:
  - `docs/components.md` — the component gallery. **Presentational only**: no
    build or regeneration instructions.
  - `docs/features.md`, `docs/keymap.md`, `docs/markdown.md`, `docs/styling.md`,
    `docs/themes.md` — focused guides, each with its generated assets in a
    same-named subdirectory, except `docs/markdown.md`, which reuses the
    gallery's `DEMOS` scenes rather than owning a second copy of them. A
    component earns a guide of its own when its surface outgrows a gallery
    entry; the gallery then keeps the entry and links out, rather than the guide
    duplicating it.
  - `docs/showcases.md` — applications built on tuika: what each one is, where to
    find it, and a recording of its UI. It exists to answer "what does this look
    like carrying a real product?", which the component gallery cannot. Entries
    are host-owned software, so the page states what it shows and links out; it
    does not document the hosts.
- rustdoc is public documentation too. The crate-level `//!` header in `lib.rs`
  is what docs.rs renders as the front page; component demos are embedded inline
  on the relevant type via `raw.githubusercontent.com` URLs so they resolve
  there.
- docs.rs is a *different build*, not a stricter one: nightly, `--cfg docsrs`,
  and therefore the only configuration in which the crate's
  `cfg_attr(docsrs, feature(…))` line exists at all. An ordinary `cargo doc`
  cannot prove that build works — 0.4.0 published with a since-removed nightly
  feature gated behind exactly that cfg and rendered no documentation at all,
  while every local and CI doc build stayed green. CI therefore builds the docs
  twice: once as a consumer does, once as docs.rs does.
- `knowledge/` is internal durable memory: intent, constraints, tradeoffs, and
  architectural decisions for maintainers, split into `specs/` (the product) and
  `processes/` (working on it).
- `.agents/`, `AGENTS.md`, and `CONTRIBUTING.md` are contributor and agent
  workflow material, not user guidance. `CONTRIBUTING.md` addresses someone
  changing tuika rather than using it, so it is the one contributor-facing file
  a reader arrives at without being sent, and it may name internal machinery
  where a contributor would otherwise be asked for something unexplained.

### Every public component is in the gallery

`docs/components.md` carries **every** component: a name, a description, an API
link, and its demo — including a component that ships in a companion crate,
whose entry names the crate. A reader asking "does tuika have an X view?" must be
able to answer it from one page, without knowing how the workspace is split
across published crates. A component whose only documentation is its rustdoc is
undiscoverable to someone who does not already know it exists.

The gallery stays presentational: an entry describes and shows, and links to a
guide when the surface is larger than one entry.

### The README indexes; the guides explain

`README.md` is the entry point, and its job is to say what exists and where to
read about it. Detail belongs in `docs/`. A feature earns at most a short
paragraph and a link there, plus a row in the component table — not a code
sample, and not a screenshot per entry point. Two symptoms mean the boundary has
slipped: an example that appears in both the README and a guide, and a README
section that grows every time the feature does.

The guides take the opposite posture: `docs/markdown.md` and its siblings are
where examples, options, caveats, and recordings accumulate.

### A demo sits with what it demonstrates

A recording is evidence for a claim, so it goes immediately after the prose or
API it depicts, with a line saying what to look at when the frame is not
self-evident. A recording placed above the first section, or on a page whose
subject it only partly covers, reads as chrome and is skipped — a demo nobody
connects to a feature is as good as absent.

The same rule decides which page: the seam's demo goes where the seam is
documented, the component's where the component is.

### rustdoc carries an example and a demo

Every public component and every host-facing seam documents itself twice over:

- **An example that compiles** — a doctest, so it cannot rot silently. For a
  seam, the example shows an *implementation*, not only how to attach one; the
  reader's question is what their `impl` has to do.
- **The demo**, embedded by absolute `raw.githubusercontent.com` URL so it
  resolves on docs.rs, plus the command that runs the example it came from.

docs.rs is where most readers meet the API, and it cannot follow a repository
relative link. Rustdoc that only names a guide leaves them with nothing to look
at.

## Direction of links

The public documentation boundary is one-way:

- `README.md` and files below `docs/` MUST NOT link to internal documents below
  `knowledge/` or `.agents/`, or require users to read them. These two paths are
  the whole of the restriction, and CI checks exactly them.
- Public pages MAY link to other public pages, docs.rs, external standards, and
  source files when those links help complete a task.
- Specs MAY link to public documentation to identify the user-facing surface.
- Internal contributor material MAY link to both.

Removing an internal link must not remove information users need. Move or
summarize the relevant operational guidance in public docs first.

CI enforces the link direction and runs the OKF bundle validator; review still
owns clarity, task completeness, working examples, and accurate warnings.

## Change requirements

- A new public API needs rustdoc that says what it is for, and a README or guide
  mention if a user would not otherwise discover it.
- Behavior changes update the affected README or guide in the same change.
  Architectural changes update the affected spec as well.
- Public and internal descriptions must agree, but must not duplicate exhaustive
  source-level detail. Specs keep the *why*; code is the *what*.
- Renames and removals repair inbound links in the same change.
- Documentation-only changes are validated with the boundary check and a review
  of changed relative links.

## Generated visual assets

Every visual in this repository is generated from code that is checked in, not
staged by hand. The rule is that the *scene registry is the source of truth*:

- Component demos come from the `DEMOS` registry in `examples/demo.rs`; VHS
  tapes are generated per scene into a temp dir and are **not committed**.
  Motion scenes are GIFs; settled scenes are full-color PNG screenshots.
- The README hero and the theme gallery come from the shared `scene()` in
  `examples/screenshot.rs`, so hero and themes cannot drift apart.
- The stylesheet gallery comes from the variant list in `examples/styling.rs`.
- A recording of a whole runnable example is driven against that example's own
  binary under VHS, so it cannot drift from what the example does, and is
  committed beside the example rather than under `docs/` so the example directory
  stays self-contained. The README's runnable-examples section links to the
  example source instead of embedding the full recording; the recording belongs
  on the relevant guide or showcase page. These sit outside the `demo -- check`
  invariant, which is about single-component scenes and their gallery references.
  The `tuika-mermaid` and `tuika-html` recordings follow this rule and ship with
  their companion crates so each crates.io README can show what it documents.
  A crate with more than one entry point records more than one scene —
  `tuika-html` has a demo for the markdown seam and another for its standalone
  component, because one recording cannot answer both questions.
- `tuika-codeformatters` follows the same example-driven rule for its language
  gallery; `scripts/gen-language-demo.sh` records the real `languages` example.
- `scripts/gen-all-demos.sh` is the repository-wide inventory and regeneration
  entry point. A default run includes component assets, the hero, theme and
  styling galleries, generated SVGs, companion-crate galleries, the Codex
  example, and external showcases. Local-only work may explicitly pass
  `--skip-showcases`; it must not be described as refreshing the showcases.
- The image demo is rendered directly by `examples/image_demo.rs` rather than
  recorded, because VHS captures through `ttyd` + `xterm.js`, which implements
  no graphics protocol and would only ever show the text fallback.
- Release notes embed demos too — see [Release](../processes/release.md#changelog-format) —
  and they reuse the same `DEMOS` scenes rather than adding one-off assets. They
  are the one place that pins the raw URL to a release tag instead of `main`, so
  a later re-recording cannot rewrite what a past release appeared to ship. That
  also puts `CHANGELOG.md` deliberately outside the `demo -- check` reference
  gate: shipped history may name a scene that no longer exists.

`cargo run --example demo -- check` is the integrity gate: every scene has a
non-empty recording in the format declared by the registry, no orphan or
stale-format asset lingers, every demo asset referenced by a gallery page
(`components.md`, `features.md`, `markdown.md`) or a rustdoc embed maps to a real
scene, and no scene is clipped by the frame it records into. It runs in CI, so
gallery drift fails the build instead of shipping a broken image to docs.rs.

The clipping assertion exists because a scene's recorded height is a hand-picked
number in the registry, and nothing about a too-small frame *looks* wrong from
inside the harness — the terminal just paints fewer rows. Several demos shipped
with their bottoms cut off before it was added. The check re-renders each scene
with room to spare and compares: any line the taller frame shows and the recorded
one does not is a line the recording loses. Scenes that overflow deliberately — a
scroll viewport, a log tail — declare it in the registry rather than being
special-cased in the check.

For that check to mean anything, the registry has to be what decides a scene's
frame. A VHS tape is sized in *pixels*, and the emulator divides by whatever cell
size the font gives it, so the same tape yields different row counts on different
recording hosts — the original clipping was exactly this: heights computed from a
cell size the recorder did not actually use. The harness therefore pins each
scene to a fixed column count and its registry row count, paints the surplus in
the theme background, and asks the tape for slightly more room than it needs.
Font metrics can then drift without changing what a demo shows.

Regenerate an asset whenever the behavior or appearance it depicts changes — a
stale recording is a documentation defect, not cosmetic debt.

### Showcase recordings

The showcase recordings are the one exception to "generated from code that is
checked in": they capture *other projects*, so the source of truth is each host's
own repository, and `scripts/gen-showcase-demos.sh` clones and builds them into a
cache directory to record them. They live outside `docs/demos/`, so they are also
outside the `demo -- check` invariant — nothing fails CI when one is
unreferenced, and the pairing is kept by review.

Three constraints govern any new showcase scene:

- **It must record deterministically and offline.** A demo that depends on a
  provider key, a paid API, or run-to-run model output is not reproducible by a
  maintainer and cannot be regenerated when the look changes. Both current scenes
  are driven by a local LLMSim — yolop against a *scripted* simulator, and the
  LLMSim dashboard against a traffic loop the generator drives itself.
- **It must not misrepresent the host.** The recording shows the host's real UI
  doing something it really does; where the inputs are simulated, the page says
  so.
- **It must be captured at the gallery's pixel density, in real time.** See
  [Capture geometry](#capture-geometry): a showcase sits beside component demos
  on a page, so a softer one reads as a defect.
- **A replica must be labeled as one, adjacent to the image.** The page also
  carries an in-repo example that imitates another product's UI (the Codex CLI
  replica). Imitation is legitimate — it is how the toolkit gets exercised at
  application scale — but a reader must not be able to mistake the recording for
  the product, so the entry states plainly that it is a replica, that it is
  unaffiliated with and unendorsed by the product's owner, and that nothing
  behind the interface is real. The same disclaimer belongs anywhere the
  recording is embedded.

### Capture geometry

Every VHS recording is displayed at `width="880"` (rustdoc caps embeds at roughly
the same). Crispness therefore depends on one ratio: **recorded pixels per cell
against displayed pixels per cell.** The component gallery sets the bar — 66
columns at `FontSize 40`, so ~26 px cells in a ~1800 px window, a little over 2×
the display width — and every other recording is held to it. A capture that packs
more columns into the same 880 px must scale its font up to compensate, not leave
the glyphs at a size that was only ever legible at full resolution. Generators
pin font size, window geometry, and a non-blinking cursor. They deliberately
leave the family at VHS's default monospace: naming a font would make the output
depend on what the recording host has installed and can silently fall back to a
different face. Generators also clear `NO_COLOR` so a maintainer's shell policy
cannot erase the scene's own palette.

Resolution trades against a second, non-negotiable property: **playback must run
at the speed the session really ran.** VHS captures frames through a headless
browser and encodes them at a fixed rate, so once the frame is large enough that
capture falls behind, the missing frames come out of the *duration* — the GIF
plays faster than the recorded program behaved, which is the misrepresentation
the constraint above forbids. Timing is the property to protect; resolution
yields to it.

The practical consequence for a full-screen host, which needs far more columns
than a single component: pick the smallest grid the UI genuinely needs, then the
largest font whose frame still records in real time, and verify the result — the
GIF's duration must match the tape's, not merely look sharp in a still.

### The crate/GitHub asset split

The root package's demo, showcase, theme, and styling assets total ~14 MiB and are
consumed only by the GitHub-rendered README and `docs/*.md`. docs.rs renders the
hand-written `//!` header, which references none of them, so bundling them only
bloats the published `.crate`. Root `Cargo.toml`'s `exclude` keeps them — and the
repository machinery (`knowledge/`, `.agents/`, `.github/`, `scripts/`) — out
of that tarball; only `docs/hero.gif`, `docs/demos/image.svg`, and
`docs/demos/split-footer.svg`, which its crates.io README embeds by relative
path, ship.

The split is per **published crate**, not per repository, and the deciding
question is how that crate's own README reaches the asset — because that is what
determines whether the packaged copy is ever read:

- **Relative path** — crates.io renders the page from the tarball, so the asset
  must ship. tuika's README assets (`docs/hero.gif`, `docs/demos/image.svg`, and
  `docs/demos/split-footer.svg`) and `tuika-mermaid`'s ~32 KiB recording beside
  its example are here.
- **Absolute `raw.githubusercontent.com` URL** — the packaged copy is
  unreachable from inside the `.crate` and is pure weight, so it is excluded
  wherever it lives. `tuika-codeformatters`' `docs/languages.gif` was shipping
  428 KiB this way, 94% of that crate's download.

So a member needs an `exclude` only when it has an absolute-URL asset; what every
published member does need is a case in `tests/packaging.rs`, which drives the
real `cargo package --list` for all four and asserts both directions.

### Capture toolchain

Reproducing any VHS capture needs the same tools on `PATH`: **VHS**, which
drives **ttyd** and **ffmpeg** (both installed separately) and renders frames
through a headless Chromium it fetches via `go-rod` on first run into
`~/.cache/rod`.

- `ttyd` and `ffmpeg` come from the system package manager.
- VHS ships prebuilt binaries; when a release download is unavailable, build it
  with `go install github.com/charmbracelet/vhs@latest` (`GOTOOLCHAIN=auto` lets
  Go fetch a new enough toolchain).

In a container or as root, set `VHS_NO_SANDBOX=true`; to reuse an installed
browser instead of the `go-rod` download, point `ROD_BROWSER_BIN` at its
`chrome` binary. These knobs belong in the shell around a recording, not in
generated tapes, so the tapes stay portable.

## Public surface

- [`README.md`](../../README.md)
- [`docs/`](../../docs/)
