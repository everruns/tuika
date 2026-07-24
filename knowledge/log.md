# Knowledge Log

Significant changes to tuika's durable knowledge are recorded here. Routine
wording, formatting, and link fixes do not need entries.

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

## 2026-07-24 — Signed history, Doppler secrets, PR policy

- Rewrote the repository's history so every commit is GPG-signed and verifies.
  Signing is now a hard requirement rather than a convention; a rewrite that
  drops signatures is a defect, since a later commit cannot restore them.
- The signing key is held in Doppler (`everruns-dev` / `dev`) as
  `COMMIT_SIGNING_KEY_B64`, with its fingerprint in `COMMIT_SIGNING_KEY_ID`.
  Doppler is now the stated home for every secret this repository touches — the
  repository itself holds none.
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
  (see [Testing](specs/testing.md)), and the PTY smoke needs the `gallery`
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
