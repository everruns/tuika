# Knowledge Log

Significant changes to tuika's durable knowledge are recorded here. Routine
wording, formatting, and link fixes do not need entries.

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
