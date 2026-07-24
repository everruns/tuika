---
type: Process Specification
title: Testing Specification
description: Defines how tuika's rendering is tested hermetically and which performance measurements gate a change.
---

# Testing Specification

## Abstract

tuika is a rendering library, so "does it work" means "did the right cells get
painted". This spec defines the test layers, what each is responsible for, and
which measurements are gates versus archives.

## Principle: assert cells, not bytes

Layout and rendering are tested by painting into an in-memory ratatui `Buffer`
and reading the cells back — no real terminal, no PTY, no timing. This is the
reason nearly every behavior in the crate is testable in a unit test, and it is
why the input path translates crossterm events into tuika's own types at the
boundary: everything above that line takes synthetic events.

A test that asserts on a raw byte stream instead of cells is testing the
terminal encoder, not the component, and belongs in the PTY layer.

## Layers

| Layer | Location | Responsible for |
| --- | --- | --- |
| Unit | each module's `#[cfg(test)] mod tests` | layout math, component rendering, interactive state, keymap dispatch, compositor, easing, OSC encoders, palette slots |
| Cross-module | `src/integration.rs` | behavior spanning several modules with no single owner: composed trees, degenerate screens where scroll and overlay interact |
| Property | `src/proptests.rs` | solver and overlay invariants for *any* input — children stay in bounds, flex fills exactly |
| Golden snapshot | `src/snapshots.rs` | whole screens diffed against checked-in glyph grids |
| Size sweep | unit | no panic and no out-of-clip writes from `0×0` upward |
| PTY smoke | `tests/pty_smoke.rs` | the terminal-facing protocol: alt-screen and cursor/mouse lifecycle pairs, OSC 9;4, OSC 8, truecolor and Braille cells through a reference terminal parser, resize survival, clean exit |
| Packaging | `tests/packaging.rs` | what the published `.crate` contains |

Snapshots refresh with `UPDATE_SNAPSHOTS=1`. A snapshot diff is a prompt to
look, not a prompt to bless: regenerate only after confirming the new grid is
what the change intended.

The consumer-facing subset of this machinery — `testing::{render, render_sizes,
grid}` — is public API, so hosts test their own views the same way. Changes to
it are API changes.

## Why a PTY layer exists at all

Cell assertions cannot see the alternate screen, cursor visibility, mouse
capture, or out-of-band escapes, because none of those are cells. The PTY smoke
drives the `gallery` example under a real pseudo-terminal and replays the byte
stream through a reference terminal (vt100), so the assertions still read a cell
grid rather than a byte soup.

It asserts *pairs*: every enter has its matching restore. A renderer that leaves
the terminal in the alternate screen, with the cursor hidden or mouse capture on,
is the failure mode a user notices most and a buffer test can never catch.

Because the assertions read the gallery's on-screen text, that example's box
titles, spinner style, and footer URL are load-bearing — changing them means
updating `scripts/assert-gallery.sh` too.

## Cross-terminal checks

In-repo tests prove tuika emits the right bytes. Whether a specific emulator
*paints* them correctly is a different question, answered by
`.github/workflows/nightly-terminals.yml`: the tmux leg asserts on captured
text; the GUI legs (kitty under Xvfb, iTerm2, Windows Terminal) capture
artifacts best-effort and do not fail the nightly. Promote a best-effort leg to
asserting once its capture is proven stable on the runner.

## Performance: one gate, one archive

Two benchmark families with deliberately different standing:

- **Criterion** (`benches/*.rs`, not `*_iai`) measures wall-clock. Shared CI
  runners are too noisy to gate on, so CI runs these only on `main` and on
  demand, and uploads the output as an artifact. Regression-checking is local
  and baseline-to-baseline (`--save-baseline` / `--baseline`). They still
  *compile* on every PR, so they cannot rot.
- **iai-callgrind** (`benches/*_iai.rs`) counts CPU instructions under Valgrind.
  Counts are deterministic and machine-independent for a fixed toolchain and
  libc, so the numbers are committed to `*/benches/iai-baseline.json` and CI
  **fails** past the tolerance. This is a real gate.

The baseline is a snapshot test: when a change legitimately shifts counts — a
renderer change, a dependency bump, a toolchain upgrade — regenerate with
`python3 benches/check_iai.py --update` and commit it *with* the code change. A
baseline updated in a separate commit hides which change moved the numbers. To
refresh from CI's exact environment, dispatch the workflow manually and commit
the uploaded `iai-baseline` artifact.

## MSRV

The MSRV (1.88) is compiled by its own CI job, because `rust-toolchain.toml`
pins a newer toolchain for development and an MSRV break therefore never shows
up locally. Raising the MSRV is a deliberate, changelog-worthy decision, not a
side effect of reaching for a new language feature.

## Requirements

- Every behavior change carries a test that exercises the changed behavior
  through its real entry point.
- A bug fix prefers a failing test before the fix when practical.
- A rendering change asserts cells, not escape bytes, unless the change *is* to
  the escape encoder.
- A change touching the published file set updates `tests/packaging.rs`
  expectations in the same commit.

## Related

- [shipping.md](./shipping.md)
- [architecture.md](./architecture.md)
- [out-of-band.md](./out-of-band.md)
