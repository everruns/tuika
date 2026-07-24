---
name: maintenance
description: Goal-oriented repository maintenance and release-readiness work for tuika. Use when the user asks for maintenance, release prep, repo health review, dependency refreshes, knowledge/docs alignment, demo asset drift, test gap review, or general cleanup without prescribing an exact sequence.
metadata:
  internal: true
user-invocable: true
---

# Maintenance

Goal: leave the repo materially healthier and closer to release-ready, with evidence.

This skill implements [`knowledge/specs/maintenance.md`](../../../knowledge/specs/maintenance.md). Keep operational guidance here. Keep design intent and constraints in the spec.

This skill is outcome-oriented. Choose the smallest set of actions that closes the real maintenance risk in front of you.

## When To Use

- release-readiness review
- CI health on `main`, or a red nightly cross-terminal run
- dependency refreshes (especially the ratatui/crossterm line)
- knowledge or docs drift
- surface drift across API / rustdoc / README / guides / demo scenes / tests
- stale demo recordings that no longer match what the code renders
- test coverage gaps
- code simplification / removing over-abstraction and dead code
- security hygiene review
- performance review of recently changed code
- AGENTS / skills hygiene

## Required Outcomes

1. **The maintenance scope is explicit.** If the user provided one, use it; otherwise state the inferred scope.
2. **The work produces concrete improvement.** Fix small/local issues; capture crisp findings for the rest.
3. **Validation matches risk.** Run checks that prove the updated areas are healthy.
4. **A release claim is backed by evidence.** Do not declare release-ready unless the changed surfaces were actually checked.

## Operating Model

- Start from goals and risk surface, not checklist order.
- A red CI on `main` outranks every other scope. Fix it first or open an issue and report the pass as blocked.
- Highest-signal first: recent diffs, failing checks, stale knowledge, outdated ratatui line.
- Skip untouched areas with a reason. Prefer fixing over reporting.
- For bugs uncovered, prefer a failing test before the fix when practical.
- Keep changes PR-sized. Defer anything larger to a GitHub issue and record the issue number in the report.

## Maintenance Surfaces

### CI Health

- check the latest workflow runs on `main` (`gh run list --branch main --limit 5`)
- any red run is a hard gate: the pass is not complete while `main` is red
- check the nightly cross-terminal workflow separately. Its **tmux** leg asserts,
  so a failure there is a real regression; the kitty / iTerm2 / Windows Terminal
  legs are `continue-on-error` artifacts and a red one is not, by itself, a
  blocker — but download the artifact and look before dismissing it
- if the failure is out of reach, open an issue with the failing run linked and report blocked

### Dependency Health

The small dependency set is a designed property, not an accident. Defend it.

Actions:
- check for newer `ratatui-core` / `ratatui-crossterm` / `crossterm` releases
  (`cargo search ratatui-core --limit 1`). These three must stay mutually
  consistent, and `crossterm` must match what `ratatui-crossterm` pins, or Cargo
  builds two backends
- confirm `ratatui` is still only a **dev**-dependency; under `[dependencies]` it
  means the umbrella crept back in
- treat a `ratatui-core` major bump as an interoperability event, not a routine
  upgrade: it changes the `Buffer` type on the public seam, so hosts must move in
  lockstep. It is a breaking release for tuika
- check `pulldown-cmark`, `textwrap`, `unicode-*` minors; verify
  `pulldown-cmark` still builds with default features off
- for `tuika-codeformatters`, check the tree-sitter grammar crates and
  `tree-sitter-highlight` together — a grammar pinned to an older `tree-sitter`
  duplicates the parser runtime
- run `cargo update` for transitive drift
- review `cargo tree --duplicates`; fix or note why unfixable
- run `cargo audit` when available; otherwise check Dependabot alerts
- grep for direct dependencies no longer used

Good evidence:
- `cargo build --all-targets` + `cargo test --all-features` after bumps
- the MSRV job still passing (a dependency can raise the effective MSRV without
  touching this crate's code)

### Knowledge And Docs Alignment

- read `knowledge/index.md`, then inspect concepts affected by behavior changed
  since the last maintenance point
- run `python3 scripts/validate_okf.py knowledge --check-links`
- check for durable behavior, intent, architecture, policy, constraints,
  terminology, or maintainer process that is missing from or contradicted by the
  bundle; staleness is based on contradiction or missing coverage, not age alone
- ensure `knowledge/index.md` covers every concept and accurately classifies it
- remove or mark superseded obsolete concepts; update `knowledge/log.md` for
  significant additions, removals, or changes
- keep `README.md`, `docs/`, rustdoc, and `AGENTS.md` aligned without linking
  public docs into `knowledge/` or duplicating source-level detail

### Surface Drift

A capability is ready only when its surfaces agree: public API, rustdoc,
`README.md`, `docs/`, the `DEMOS` scene registry, `knowledge/`, and tests.

- diff the component list in `README.md` against what `components/mod.rs` exports
- check that every component with a demo reference has a scene, and vice versa
  (`cargo run --example demo -- check` proves the asset half; the README table is
  a manual read)
- check recently shipped work (`git log` since the last tag) for a test that
  exercises it and a docs mention
- look for `pub` items whose rustdoc no longer matches behavior —
  `#![warn(missing_docs)]` catches absence, never staleness
- outcome: a small reconnecting fix, or a finding naming the missing surface and
  its user-visible impact

### Demo Asset Drift

Recordings are documentation and go stale silently.

- `cargo run --example demo -- check` — scenes, recordings, and references in sync
- for components whose look changed since the recording, regenerate with
  `scripts/gen-demos.sh <scene>`; the hero and theme galleries share
  `examples/screenshot.rs`, so a look change usually means regenerating both
  (`scripts/gen-hero.sh`, `scripts/gen-theme-demos.sh`)
- a recording that no longer matches the code is a documentation defect, not
  cosmetic debt
- if VHS is unavailable, `--dump` a scene and compare against the GIF by eye
  before declaring it current

### Code Simplification And De-Abstraction

A first-class maintenance surface, not just incidental cleanup on touched
files. A deep pass actively hunts for complexity the codebase no longer earns.
Bias toward deleting code: the best maintenance often removes more than it adds.

On code touched during the pass, always:

- delete dead code, unreachable branches, commented-out blocks
- drop TODOs that are already resolved

On a deep pass, also scan for and collapse over-abstraction:

- **Single-use abstractions** — traits with one impl, a wrapper type that only
  forwards, a generic with one instantiation, a builder for a two-field struct.
  Inline them unless the seam is load-bearing (a real second impl, a public
  extension point, a test double that earns its keep).
- **Premature generalization** — code shaped for hypothetical futures instead of
  current needs. Delete the unused flexibility; the git history keeps it.
- **Indirection with no payoff** — a helper called once that only renames a
  standard-library call, a module that re-exports one item, a config knob no
  caller sets to anything but the default.
- **Duplication that wants a helper** — the inverse: the same 5+ lines pasted in
  three places is under-abstraction. Consolidate only when it genuinely reduces
  total code and reads clearer, not to chase a DRY score.
- **Deep nesting and sprawling match arms** — flatten with early returns, `let
  ... else`, or extracted functions where it lowers cognitive load.
- **Unclear names** — rename functions, variables, and types so intent is
  legible without chasing definitions.

Guardrails: keep each simplification small and independently reviewable; do not
bundle a de-abstraction sweep with an unrelated fix. Verify with
`cargo build`, `cargo clippy`, and `cargo test` — a simplification that changes
behavior is a bug, not a cleanup. **Removing a `pub` item is a breaking change,
not cleanup**: it needs a changelog entry and a minor bump, so weigh it against
the release schedule rather than slipping it into a cleanup PR. When a
simplification is too large to land inline, defer it to a GitHub issue naming the
abstraction and why it no longer pays its way.

### Security And Threat Posture

tuika does no network or filesystem I/O, spawns no processes, and holds no
credentials. The surface is what it writes to the terminal and what untrusted
content does to it.

- confirm every out-of-band escape is still produced by tuika's own encoder from
  host-supplied data, never from interpolated caller text
- confirm every terminal-state enter has a matching restore on all exit paths,
  including error paths — `tests/pty_smoke.rs` asserts the pairs
- confirm image emission is still gated on capability detection with a text
  fallback (the one protocol family where an unsupporting terminal shows garbage
  rather than nothing)
- exercise the parsers against pathological input: deeply nested markdown, very
  long lines, unterminated fences, wide/zero-width graphemes at clip boundaries.
  Property tests and the size sweep are the standing defense — check they still
  cover the parsers after a change
- keep `SECURITY.md` consistent with this posture

### Test And Runtime Confidence

- `cargo test --all-features` clean
- `cargo run --example demo -- check` clean
- the PTY smoke passing on a Unix host (it is `#![cfg(unix)]`)
- MSRV job green, and the declared `rust-version` still accurate
- the iai baseline current for `main`

## Common Evidence Commands

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo run --example demo -- check`
- `cargo doc --all-features --no-deps`
- `cargo publish --dry-run -p tuika`
- `cargo search ratatui-core --limit 1`
- `cargo tree --duplicates`
- `cargo outdated` (when available)
- `cargo audit` (when available)

## Deliverable

Report:

- what scope was covered
- what was fixed or found
- what evidence was gathered
- deferred findings, each with its GitHub issue number
- what was intentionally skipped and why
- **blocked** status if `main` CI is red and out of reach

If the user asks to ship after maintenance, hand off to [`/ship`](../ship/SKILL.md).
