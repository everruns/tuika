---
name: ship
description: Goal-oriented workflow for landing a requested change to tuika safely. Use when the user asks to ship, fix and ship, take a change through validation, or drive PR/CI/merge to completion.
metadata:
  internal: true
user-invocable: true
---

# Ship

Goal: land the requested change safely, with evidence, and merge only after CI is green.

This skill implements [`knowledge/processes/shipping.md`](../../../knowledge/processes/shipping.md). Keep operational guidance here. Keep the shipping success bar and constraints in the spec.

This skill is outcome-oriented. Do not blindly walk a fixed checklist. Start from the goal and changed risk surface, then choose the smallest path that proves the change is ready.

## When To Use

Use this skill when the user asks to:

- ship or fix and ship a change
- take work through validation, PR creation, CI, and merge
- prove a branch is merge-ready

## Required Outcomes

**ALL outcomes below are MANDATORY. Do not skip or weaken any requirement.**

1. **The branch state is safe.**
   - A PR is required only for **external contributions**; as a maintainer you
     may land directly on `main` once CI is green. Work on a branch anyway when
     the change is risky or wants review.
   - The working tree must be clean before the final push.
   - Prefer rebasing onto the latest `origin/main` before merge.
   - Every commit you push MUST be signed and verify (`git log --format='%h %G?'`
     shows `G`/`U`, never `N`). A rebase or amend re-signs only when told to:
     `git rebase --gpg-sign`. Prefer the human maintainer's existing SSH or
     OpenPGP signing configuration. Doppler's OpenPGP identity is a fallback
     when no usable personal key is configured — see `AGENTS.md` § Signing.

2. **The requested goal is achieved with evidence.**
   - Review the delta with `git diff origin/main...HEAD` and `git log origin/main..HEAD`.
   - Confirm the requested behavior is actually implemented.
   - Validation must match risk. For bugs, prefer a failing test first when practical.

3. **The changed feature is tested before merge.**
   - Every behavior change MUST be covered by an automated test that exercises
     the new or changed behavior directly — not merely adjacent code that still
     compiles. Add or update the test as part of the change.
   - Rendering changes assert **cells**, through `tuika::testing::{render,
     render_sizes, grid}` or a golden snapshot — not escape bytes, unless the
     change *is* to an escape encoder, in which case `tests/pty_smoke.rs` is the
     right layer.
   - A new component needs a size sweep (it must not panic or write outside its
     clip at degenerate sizes) and, if it is themed, a test pinning its cells to
     the `Theme` slots it claims to use.
   - If a behavior is genuinely impractical to cover automatically, say so
     explicitly in the PR body, describe the manual verification you performed
     instead, and list the gap under **Follow-ups**. "Hard to test" is not a
     silent pass.
   - Docs-only or config-only changes with no behavior change are exempt (state why).

4. **API changes are declared.**
   - tuika is a published library: adding, renaming, or removing any `pub` item,
     or changing its signature, is an API change. This includes `tuika::testing`,
     which hosts depend on.
   - Say so explicitly in the PR body so it reaches the changelog. Removals and
     signature changes are breaking and need a migration note.
   - Adding a runtime dependency needs an explicit justification: the default
     answer for a heavy concern is a trait the host implements.

5. **The changed code is fit to merge.**
   - Simplify obvious duplication or accidental complexity.
   - Perform the structured security review below.
   - Fix issues you find and refresh the evidence.

6. **Relevant artifacts stay in sync.**
   - Review whether the change alters durable behavior, intent, architecture,
     policy, constraints, terminology, or maintainer process.
   - If it does, update the affected `knowledge/` concepts in the same change;
     update `knowledge/index.md` for added, removed, renamed, or reclassified
     concepts and `knowledge/log.md` for significant knowledge changes.
   - If it does not, record "No knowledge update required" with a short reason in
     the PR body. Keep `AGENTS.md`, `README.md`, `docs/`, and rustdoc aligned.
   - A change to how a component *looks* means regenerating its demo recording
     (`scripts/gen-demos.sh <scene>`); a new component means adding a `DEMOS`
     scene, a `docs/components.md` entry, and a rustdoc embed.
   - Validate changed knowledge with
     `python3 scripts/validate_okf.py knowledge --check-links`.
   - Run `cargo run --example demo -- check` whenever gallery assets or their
     references moved.

7. **Smoke test impacted functionality.**
   - Always exercise the change in a real terminal, in addition to the automated
     test in outcome 3. Pick the example that covers the surface:
     `cargo run --example gallery`, `--example markdown`, `--example image`,
     `--example overlay`, `--example mouse`, or
     `cargo run --example demo -- <scene>`.
   - `cargo run --example demo -- <scene> --dump` prints one frame as text and is
     the fastest proof when a terminal is unavailable.
   - Docs-only or config-only changes may skip smoke testing with explicit
     justification.

8. **Follow-ups are surfaced, not silently dropped.**
   - Default to implementing everything in scope before merging.
   - For each candidate, decide explicitly: **implement now** (preferred) or **defer**.
   - For anything deferred, list it under a **Follow-ups** section in the PR body with a one-line rationale.
   - If there are no follow-ups, state "No follow-ups." in the PR body.

9. **The change lands safely.**
   - Push the branch (or `main`, for maintainer work that needs no PR).
   - Create or update the PR when one is required.
   - Address every review comment — including low-confidence suggestions, nits, and bot comments. For each comment, post an inline reply on the same thread explaining the resolution (and apply a code change too when one is needed), then mark that thread resolved. An inline reply is required even when the fix is a pure code change; no comment may be left unanswered or unresolved before merge.
   - Wait for CI to go green.
   - Merge with squash only after CI is green and the final review sweep is clean.
   - After merging, monitor main CI for the merge commit. If it fails, fix or revert promptly.

## Operating Model

- Start from the goal and risk surface, not checklist order.
- Choose the highest-signal path first: targeted diff review, focused tests, relevant builds, then a real-terminal smoke.
- "Fix and ship" means implement first, then switch into shipping mode.
- Stop only for blockers you cannot safely resolve alone: merge conflicts, ambiguous product intent, or CI failures you cannot reproduce.

## Security Review

Mandatory for every change that touches code, configuration, or infrastructure. tuika does no network or filesystem I/O, spawns no processes, and holds no credentials, so the categories are narrow and specific — which makes a thorough review cheap, not optional:

- **TM-ESC** — terminal escape emission. Every out-of-band sequence (OSC 8, OSC 52, OSC 9;4, graphics protocols) must be produced by tuika's own encoder from data the host passed explicitly. Check that no caller-supplied string reaches the terminal where a control sequence is interpreted. A new out-of-band feature must also be classified: if an unsupporting terminal would show garbage rather than nothing, it needs capability detection and a fallback.
- **TM-STATE** — terminal state restoration. Every enter has a matching restore: alternate screen, cursor visibility, mouse capture, progress state. An early return or `?` on a path that skips teardown leaves the user's terminal broken.
- **TM-PARSE** — untrusted content. Markdown, code, and image data may come from anywhere. Parsing and layout must degrade (slow, truncated, unstyled), never panic, index out of bounds, or allocate proportional to attacker-controlled nesting. Look for unbounded recursion, `unwrap` on parsed input, and arithmetic on untrusted widths.
- **TM-CLIP** — bounds and clipping. A write outside the clipped region corrupts unrelated UI. Degenerate sizes (`0×0`, one cell, width narrower than a wide grapheme) are the cases to check.
- **TM-DEP** — dependency risk. A new crate needs a one-line justification and must not pull a heavy transitive tree; `ratatui` must stay a dev-dependency.

For every relevant category, check the diff for: injection (escape sequences), resource exhaustion (unbounded loops or allocation), input validation at trust boundaries, and panics reachable from library input.

Document the review in the PR body under **Security**. Changes that are purely docs, comments, or test-only may state "No security-relevant code changes" with a one-line justification.

## Common Evidence Commands

Pick only what matches the changed surface:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo run --example demo -- check` (gallery assets and references)
- `cargo +1.88 check -p tuika --all-targets` (MSRV, when using newer language features)
- `cargo publish --dry-run -p tuika` (when the packaged file set or manifest metadata changed)
- `cargo bench --bench <name> -- --baseline before` (hot render paths)
- `cargo doc --all-features --no-deps` (when rustdoc or feature gating changed)

## PR And Merge

- Use a Conventional Commit style PR title under 70 characters.
- In the PR body, explain what changed, why, how it was validated, notable risks, any API change, and an explicit **Follow-ups** section (or "No follow-ups.").
- Attach a Before / After with proof a reviewer can check: for visual/TUI changes a
  before/after screenshot, recording, or `--dump` capture; for other behavior, CLI
  output or logs. State explicitly when a change has no observable behavior.
- After CI is green, wait at least 2 minutes for async reviewer bots, then do one last comment sweep before merge.
- Merge with `gh pr merge --squash` only after CI is green and the final review sweep is clean.
- Do not use auto-merge: async review bots can post after the last push.
