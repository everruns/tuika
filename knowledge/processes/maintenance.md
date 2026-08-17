---
type: Process Specification
title: Maintenance Specification
description: Defines the success criteria for tuika's repository maintenance and release readiness.
---

# Maintenance Specification

## Abstract

This specification defines goal-oriented maintenance for tuika. Maintenance
improves release readiness and repository health with evidence, not by
mechanically executing a fixed checklist.

The canonical agent workflow lives in
[`.agents/skills/maintenance/SKILL.md`](../../.agents/skills/maintenance/SKILL.md).
That skill is user-invocable so maintenance can be requested directly as
`/maintenance`.

## Design Goals

1. Make the maintenance scope explicit.
2. Improve the repository in concrete ways or produce crisp findings with
   evidence.
3. Match validation depth to the actual risk surface.
4. Keep release claims honest.
5. Detect surface drift: capability that exists in one place (API, rustdoc,
   README, guide, demo scene, spec, test) but is missing or stale in another.
6. Keep the dependency set as small as the design promises.
7. Reduce accidental complexity: remove over-abstraction, dead code, and
   premature generalization the codebase no longer earns.

## Ownership Boundary

- This spec owns the maintenance intent, constraints, and success bar.
- The skill owns the execution workflow, heuristics, and example commands.

## Constraints

- Maintenance is risk-proportional, not sweep-proportional.
- The selected scope must be explained, including what was skipped and why.
- If maintenance changes code or behavior, affected artifacts must stay in sync:
  `README.md`, `docs/`, rustdoc, `AGENTS.md`, `knowledge/`, and generated
  demo assets.
- Maintenance prefers concrete fixes over ceremonial audits when a safe local fix
  exists.
- Dependency upgrades should respect a short release-age floor (≥1 day for
  patch, ≥7 days for minor/major) to avoid landing same-day yanks.

## CI Health Gate

GitHub Actions on `main` is the CI source of truth. The latest run on `main` must
be green before a maintenance pass is reported complete:

- A red `main` is the first maintenance item, ahead of any other scope.
- If the pass cannot fix the failure, it must open a tracked issue and report the
  pass as **blocked**, not complete.

The nightly cross-terminal workflow is a separate signal. Its tmux leg asserts
and a failure there is a real regression; the GUI legs are best-effort artifacts
and a red one is not, by itself, a blocker.

## Deferred Findings

Findings too large to fix inline (multi-file refactors, upgrades needing
non-trivial rework) are deferred, not dropped:

- each deferred finding becomes a GitHub issue with scope and reproduction
- the issue numbers appear in the maintenance report

Deferred items are not failures. Untracked ones are.

## Surface Drift

A capability is not complete merely because the code exists. tuika's surfaces are
the public API, rustdoc, `README.md`, the guides in `docs/`, the demo scene
registry, `knowledge/`, and the test suite. Maintenance should catch:

- `pub` items with no rustdoc, or rustdoc that no longer matches behavior
  (`#![warn(missing_docs)]` catches absence, not staleness)
- components present in the API but absent from the `docs/components.md` index,
  absent from its family pages, or without a demo scene
- guides or specs describing behavior the crate no longer has
- demo recordings that no longer look like what the code renders
- shipped behavior with no test exercising it
- rendering behavior tested only through escape bytes instead of cells

The outcome is either a small fix that reconnects the surfaces or a crisp finding
naming the missing surface and its user-visible impact — not a generic "tech
debt" note.

## Dependency Discipline

The small dependency set is a designed property, not an accident
([goal.md](../specs/goal.md)). Maintenance defends it:

- **No new runtime dependency without an explicit justification in the PR.** The
  default answer for a heavy concern is a trait the host implements, or a
  separately published companion crate.
- The `ratatui-core` / `ratatui-crossterm` / `crossterm` versions must stay
  mutually consistent, and `crossterm` must match what `ratatui-crossterm` pins
  so Cargo dedups it. A split here silently doubles the backend.
- `ratatui` stays a dev-dependency only. If it appears under `[dependencies]`,
  the umbrella has crept back in.
- Feature flags stay off by default when they add a runtime (`async` adds Tokio).
- No known CVEs in the tree (`cargo audit` when available, plus Dependabot
  alerts); duplicate transitive versions reviewed (`cargo tree --duplicates`) and
  either fixed or noted as unfixable.
- A major-version bump of `ratatui-core` is an interoperability event, not a
  routine upgrade: it changes the `Buffer` type on the public boundary, so hosts must
  move in lockstep. Treat it as a breaking release.

## Release Readiness Standard

Before tagging a release:

- `cargo test --all-features` is green on `main`
- `cargo clippy --all-targets --all-features -- -D warnings` is clean
- the MSRV job passes, and the declared `rust-version` still matches reality
- `cargo run --example demo -- check` passes and the committed recordings match
  current rendering
- `cargo publish --dry-run` succeeds for the root and every companion crate
- the iai baseline is current for the released code
- the README's component table and guide links match the API

## Security And Threat Posture

tuika performs no network or filesystem I/O, spawns no processes, and holds no
credentials, so the surface is narrow and specific:

- **Escape emission** — every out-of-band sequence must come from tuika's own
  encoder, never from interpolated caller text. Maintenance verifies that
  property still holds ([out-of-band.md](../specs/out-of-band.md)).
- **Untrusted content** — markdown and code passed to `Markdown`/`CodeBlock` must
  degrade rather than panic or allocate unboundedly. The property tests and size
  sweeps are the standing defense; keep them running over the parsers.
- **Terminal state** — a path that leaves the alternate screen, cursor, or mouse
  capture unrestored is a user-visible failure the PTY smoke exists to catch.

[`SECURITY.md`](../../SECURITY.md) is the public statement of this posture and
must stay consistent with it.

## Code Simplification And De-Abstraction

Complexity accretes: an abstraction added for a second caller that never arrived,
a trait with one impl, a knob nobody sets. A deep maintenance pass treats
removing that complexity as real work. The bias is toward deletion.

Maintenance should look for and collapse:

- single-use abstractions (one-impl traits, forwarding wrappers,
  single-instantiation generics, builders for trivial structs) — unless the boundary
  is load-bearing
- premature generalization shaped for hypothetical futures, not current callers
- indirection with no payoff: helpers that only rename a stdlib call, modules
  that re-export one item, always-default knobs
- under-abstraction: the same block pasted across components where a shared
  helper genuinely reduces total code
- deep nesting and long match arms that a flatten or extraction makes legible
- names that hide intent

Constraints:

- A simplification must preserve behavior, verified by build, clippy, and the
  test suite. A behavior change disguised as cleanup is a regression.
- Keep simplifications small and independently reviewable.
- **Removing a `pub` item is a breaking change**, not cleanup. It must be called
  out and released as such.
- A simplification too large to land inline is deferred to a tracked issue naming
  the abstraction and why it no longer pays its way.

## Spec Hygiene

Specs preserve design intent, rationale, and constraints — not implementation
details readable from code. Maintenance should:

- replace duplicated struct/enum/field tables with links to source
- replace exhaustive component or feature lists with links to source
- keep the "why" and constraints; link to code for the "what"

## Related

- [shipping.md](./shipping.md)
- [release.md](./release.md)
- [documentation.md](../specs/documentation.md)
- [testing.md](./testing.md)
