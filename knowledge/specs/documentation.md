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
  - `docs/features.md`, `docs/keymap.md`, `docs/styling.md`, `docs/themes.md` —
    focused guides, each with its generated assets in a same-named subdirectory.
  - `docs/showcases.md` — applications built on tuika: what each one is, where to
    find it, and a recording of its UI. It exists to answer "what does this look
    like carrying a real product?", which the component gallery cannot. Entries
    are host-owned software, so the page states what it shows and links out; it
    does not document the hosts.
- rustdoc is public documentation too. The crate-level `//!` header in `lib.rs`
  is what docs.rs renders as the front page; component demos are embedded inline
  on the relevant type via `raw.githubusercontent.com` URLs so they resolve
  there.
- `knowledge/` is internal durable memory: intent, constraints, tradeoffs, and
  architectural decisions for maintainers, split into `specs/` (the product) and
  `processes/` (working on it).
- `.agents/`, `AGENTS.md`, and `CONTRIBUTING.md` are contributor and agent
  workflow material, not user guidance. `CONTRIBUTING.md` addresses someone
  changing tuika rather than using it, so it is the one contributor-facing file
  a reader arrives at without being sent, and it may name internal machinery
  where a contributor would otherwise be asked for something unexplained.

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
- The README hero and the theme gallery come from the shared `scene()` in
  `examples/screenshot.rs`, so hero and themes cannot drift apart.
- The stylesheet gallery comes from the variant list in `examples/styling.rs`.
- A recording of a whole runnable example is driven against that example's own
  binary under VHS, so it cannot drift from what the example does, and is
  committed beside the example rather than under `docs/` so the example directory
  stays self-contained. These sit outside the `demo -- check` invariant, which is
  about single-component scenes and their gallery references. The
  `tuika-mermaid` recording follows this rule and ships with that companion
  crate so its crates.io README can show the integration it documents.
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
non-empty recording, no orphan GIF lingers, every `demos/<name>.gif` referenced
by the gallery markdown or a rustdoc embed maps to a real scene, and no scene is
clipped by the frame it records into. It runs in CI, so gallery drift fails the
build instead of shipping a broken image to docs.rs.

The clipping assertion exists because a scene's recorded height is a hand-picked
number in the registry, and nothing about a too-small frame *looks* wrong from
inside the harness — the terminal just paints fewer rows. Several demos shipped
with their bottoms cut off before it was added. The check re-renders each scene
with room to spare and compares: any line the taller frame shows and the recorded
one does not is a line the GIF loses. Scenes that overflow deliberately — a
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
the glyphs at a size that was only ever legible at full resolution.

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

The root package's demo, showcase, theme, and styling GIFs total ~14 MiB and are
consumed only by the GitHub-rendered README and `docs/*.md`. docs.rs renders the
hand-written `//!` header, which references none of them, so bundling them only
bloats the published `.crate`. Root `Cargo.toml`'s `exclude` keeps them — and the
repository machinery (`knowledge/`, `.agents/`, `.github/`, `scripts/`) — out
of that tarball; only `docs/hero.gif` and `docs/demos/image.svg`, which its
crates.io README embeds by relative path, ship. A companion may ship a small
recording beside an example when its own crates.io README embeds it, as
`tuika-mermaid` does. `tests/packaging.rs` guards the root split.

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
