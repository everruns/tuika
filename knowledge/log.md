# Knowledge Log

Significant changes to tuika's durable knowledge are recorded here. Routine
wording, formatting, and link fixes do not need entries.

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
