---
type: Process Specification
title: Shipping Specification
description: Defines the evidence and safety bar for landing changes in tuika.
---

# Shipping Specification

## Abstract

This specification defines goal-oriented shipping for tuika. Shipping completes
the requested goal, gathers convincing evidence, creates a mergeable PR, and
merges only after CI is green.

The canonical agent workflow lives in
[`.agents/skills/ship/SKILL.md`](../../.agents/skills/ship/SKILL.md). That skill
is user-invocable so shipping can be requested directly as `/ship`.

## Design Goals

1. Reach the requested goal, not just perform rituals around it.
2. Match validation depth to the actual risk surface.
3. Keep affected artifacts in sync (`README.md`, `docs/`, rustdoc, `AGENTS.md`,
   `knowledge/specs/`, generated demo assets).
4. Merge only from a safe branch state with green CI.

## Ownership Boundary

- This spec owns the shipping intent, constraints, and success bar.
- The skill owns the execution workflow, heuristics, and commands.

## Required Outcomes

Every shipped change MUST satisfy ALL of these. These are mandatory, not
suggestions.

1. **Safe branch state.** No shipping from `main`. Working tree clean before
   final push. Prefer rebasing onto latest `origin/main` before merge.
2. **Goal achieved with evidence.** The requested behavior is implemented and
   validated with proof matching the risk.
3. **Feature tested before merge.** Every behavior change is covered by an
   automated test that exercises the new or changed behavior directly — driving
   its real entry point, not merely adjacent code that still compiles. Rendering
   changes assert cells through `tuika::testing`, not escape bytes, unless the
   change is to an escape encoder; see [testing.md](./testing.md). Docs-only or
   config-only changes with no behavior change are exempt with stated
   justification.
4. **API change declared.** Adding, renaming, removing, or changing the
   signature of any `pub` item is an API change: say so in the PR body so it
   lands in the changelog. Removals and signature changes are breaking and are
   only acceptable in a minor release with a migration note.
5. **Merge-ready code.** Touched code is reviewed for avoidable complexity. A
   structured security review is performed (see the ship skill § Security
   Review). Issues are addressed or explicitly blocked.
6. **Synced artifacts.** Affected artifacts are updated: README, `docs/`,
   rustdoc, `AGENTS.md`, specs. A change to a component's appearance regenerates
   the affected demo asset; `cargo run --example demo -- check` must pass. No
   code-duplicating prose.
7. **Smoke test impacted functionality.** In addition to the automated test,
   exercise the change in a real terminal through the relevant example
   (`cargo run --example gallery|markdown|image|demo -- <scene>`). Mandatory
   unless the change is docs-only or config-only with explicit justification.
8. **Follow-ups surfaced.** TODOs, partial fixes, declined suggestions, missed
   edges, and spec/doc drift are explicitly listed under **Follow-ups** in the PR
   body (or `"No follow-ups."` if none).
9. **Safe merge.** PR uses the template, CI is green, every review comment is
   addressed (via a code change when needed), answered inline on its own thread
   with a written reply, and marked resolved before merge. An inline reply is
   required even when the resolution is a pure code change. Merge is squash-only
   after a final clean comment sweep. Async reviewer bots get at least 2 minutes
   to comment after CI turns green.

## Constraints

- Shipping is outcome-oriented, not a mandatory linear checklist.
- Validation starts with the smallest high-signal proof and deepens only when
  risk requires it.
- Bug fixes prefer a failing test before the fix when practical.
- Security review is mandatory for code, configuration, or infrastructure
  changes. Perceived low risk does not justify skipping it. For this repository
  the threat surface is narrow and specific: what reaches the terminal as an
  escape sequence, and what untrusted markdown/code content can do to the parser
  and layout (see [`SECURITY.md`](../../SECURITY.md)).
- Every review comment must be explicitly addressed, answered inline on its own
  thread with a written reply (in addition to any code change), and resolved
  before merge — including low-confidence suggestions, nits, and bot comments.
- Auto-merge is not used: async reviewer bots can post after the last push or
  after CI turns green.
- If a blocker cannot be resolved safely by the agent alone, shipping stops and
  reports rather than guesses.

## Validation Menu

Use the smallest set that gives high confidence.

1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-features`
4. `cargo run --example demo -- check` when any component's appearance or the
   gallery documentation changed.
5. `cargo +1.88 check -p tuika --all-targets` when reaching for newer language
   features, to confirm the MSRV still holds.
6. `cargo bench --bench <name> -- --baseline before` when the change touches a
   hot render path; the committed iai baseline is the CI gate.
7. `cargo publish --dry-run -p tuika` when the published file set, manifest
   metadata, or `exclude` list changed.
8. Run the relevant example in a real terminal for visual proof.

## Merge Discipline

- Conventional Commits PR titles under 70 characters.
- Squash and Merge only.
- GitHub Actions is the CI source of truth.
- Never merge red CI.
- After merging, monitor main CI for the merge commit. If it fails, treat it as a
  shipping regression and fix or revert promptly.

## Related

- [testing.md](./testing.md)
- [documentation.md](./documentation.md)
- [maintenance.md](./maintenance.md)
- [release.md](./release.md)
