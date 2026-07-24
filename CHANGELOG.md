# Changelog

All notable changes to `tuika` and `tuika-codeformatters` are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/) loosely, and
the crates follow [Semantic Versioning](https://semver.org/) — pre-1.0, a minor
release may carry breaking API changes, which are always listed under
**Breaking Changes**.

Releases up to and including `tuika` 0.4.0 and `tuika-codeformatters` 0.2.0 were
cut from the [everruns/yolop](https://github.com/everruns/yolop) workspace, where
these crates began. Their history is in
[that repository's changelog](https://github.com/everruns/yolop/blob/main/CHANGELOG.md).

## [Unreleased]

`tuika` 0.5.0 · `tuika-codeformatters` 0.3.0

### Highlights

- tuika and `tuika-codeformatters` now live in their own repository,
  [everruns/tuika](https://github.com/everruns/tuika), with their own CI,
  documentation, knowledge bundle, and release pipeline.
- Terminal-native link activation: OSC 8 targets are left for the emulator to
  activate, and hosts can signal clickable regions with an OSC 22 pointer shape.
- Mouse events now report the Super/Command modifier, so macOS native link
  gestures are distinguishable from plain drags.

### Breaking Changes

- **`Mouse` gained a `super_key` field.** Code that constructs a `Mouse` with a
  struct literal must set it; `Mouse::plain()` now also requires Super to be
  released.
  - Before: `Mouse { column, row, kind, shift, ctrl, alt }`
  - After: `Mouse { column, row, kind, shift, ctrl, alt, super_key }`

### Added

- `PointerShape`, `encode_pointer_shape`, and `write_pointer_shape` — OSC 22
  mouse-pointer control, for hosts that capture pointer motion and want a
  pointing-hand cursor over link runs. Unsupported terminals ignore the
  sequence. `AltScreen`/`TerminalSession` teardown restores the default pointer,
  including on unwind.
- `LinkPolicy::allows` — ask a policy whether a URL is a valid target, so hosts
  can filter link runs without duplicating the sanitizer.
- `tests/pty_smoke.rs`: a PTY smoke test that drives the `gallery` example under
  a pseudo-terminal and asserts the terminal-facing protocol — alternate-screen
  and cursor/mouse-capture lifecycle pairs, OSC 9;4 progress, OSC 8 hyperlinks,
  and truecolor and Braille cells through a reference terminal parser.

### Changed

- `homepage` and `repository` metadata, and every documentation asset URL, now
  point at `everruns/tuika`.
- `tuika-codeformatters` requires `tuika` 0.5.

### Fixed

- Broken and redundant rustdoc intra-doc links in `anim`, `components::diff`,
  `framebuffer`, `markdown`, and `themes`. Documentation now builds clean under
  `-D warnings`, which CI enforces.
