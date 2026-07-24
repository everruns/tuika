---
type: Process Specification
title: Release Specification
description: Defines the release contract for publishing tuika and tuika-codeformatters to crates.io.
---

# Release Specification

## Abstract

This spec defines how tuika is cut, published, and verified. Releases are
agent-prepared, human-merged, and CI-published to crates.io.

The canonical agent workflow lives in
[`.agents/skills/release/SKILL.md`](../../.agents/skills/release/SKILL.md). That
skill is user-invocable as `/release`.

## Versioning

tuika follows [Semantic Versioning](https://semver.org/):

- **MAJOR** (X.0.0): reserved for 1.0 and beyond.
- **MINOR** (0.X.0): new capability, and — pre-1.0 — any breaking API change.
- **PATCH** (0.0.X): bug fixes, documentation, dependency bumps that do not
  change the public API.

Pre-1.0, a minor release may carry breaking changes; they must be listed in the
changelog with a before/after migration snippet. A patch release may not.

**Every `pub` item is public API.** Removing one, renaming one, or changing a
signature is breaking — including in `tuika::testing`, which hosts use to test
their own views. A `ratatui-core` major bump is also breaking, because the
`Buffer` on the interoperability seam is part of the contract.

## Release Targets

| Target | Surface | How users install |
| --- | --- | --- |
| GitHub Release | tag `vX.Y.Z`, source archive | `gh release view vX.Y.Z` |
| crates.io | `tuika` and `tuika-codeformatters` | `cargo add tuika` |

There are no binaries and no package-manager formulae: tuika is a library.

## Two crates, independent versions

`tuika` and `tuika-codeformatters` version independently. The repository's tag
tracks the **root package** (`tuika`), because the tag names the repository
state, and `release.yml` reads the version from the root `Cargo.toml`.

`tuika-codeformatters` is published as a side effect of a tuika release, only
when its in-tree version is not already live. Bump it when its own API changes
*or* when it must track a new tuika range — a tuika bump usually forces a
formatter bump, because the published formatter pins a compatible tuika version.
When it does, its `tuika` dependency requirement must be updated in the same
change, or the published formatter will resolve against the old tuika.

`publish.yml` derives the dependency-first order from Cargo metadata
(`scripts/publish_order.py` — currently `tuika`, then `tuika-codeformatters`) and
skips versions already live, so a new workspace member cannot be silently
omitted or published out of order.

## Release Flow

```
┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐
│ Human    │   │ Agent    │   │ Agent    │   │ Human    │   │ CI       │   │ Agent    │
│ asks     │──>│ prepares │──>│ verifies │──>│ merges   │──>│ tags +   │──>│ monitors │
│ release  │   │ PR       │   │ publish  │   │ PR       │   │ publishes│   │ crates.io│
└──────────┘   └──────────┘   └──────────┘   └──────────┘   └──────────┘   └──────────┘
```

Skipping the publish-readiness verification risks tagging a release that fails to
publish. Skipping post-merge monitoring risks declaring "shipped" while the
publish job silently failed.

### Human Steps

1. **Ask the agent** to create a release ("Cut release v0.5.0", "Prepare a patch
   release").
2. **Review the PR**, including its publish-readiness report.
3. **Squash and merge** — CI handles the GitHub Release and the crates.io
   publish.
4. **Ask the agent to monitor** until crates.io reports the new version.

### Agent Steps

When asked to release, the agent:

0. **Ensure full git history.** Cloud sandboxes are often shallow-cloned, which
   silently hides commits and yields a wrong commit count or changelog. Run
   `git fetch --unshallow origin main 2>/dev/null || git fetch origin main`
   before counting or listing commits.

1. **Determine the version.** Use the version the human specified, or propose the
   next one from the unreleased commits and confirm before proceeding. An API
   removal or signature change forces at least a minor bump.

2. **Update `CHANGELOG.md`.** Add a `## [X.Y.Z] - YYYY-MM-DD` section, list PRs
   in descending order with GitHub-style links and contributor handles, end with
   `**Full Changelog**: URL`. For minor bumps carrying API changes, add an
   explicit `### Breaking Changes` block with before/after snippets.

3. **Bump the versions.** The root `Cargo.toml` for `tuika`; and
   `crates/tuika-codeformatters/Cargo.toml` when its API changed or it must
   track a new tuika range — updating its `tuika` dependency requirement to
   match. Regenerate `Cargo.lock`.

4. **Run local verification:**
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --all-features`
   - `cargo run --example demo -- check`
   - the MSRV check, if any newer language feature landed in the release

5. **Verify publish-readiness** (this catches what local tests do not — the
   packaging step, missing files, version drift):
   - `cargo publish --dry-run -p tuika` must succeed.
   - `cargo publish --dry-run -p tuika-codeformatters` may fail locally when it
     requires a tuika version that is not yet live. That is expected, not a
     broken release: CI publishes tuika first and then the formatter resolves.
   - Confirm `Cargo.toml` and `Cargo.lock` agree on `X.Y.Z`.
   - Confirm `X.Y.Z` is greater than the latest published version
     (`cargo search tuika --limit 1`).
   - Confirm the packaged file list still excludes the repository machinery and
     heavy GIFs (`cargo test --test packaging`).
   - Fix the root cause and re-run before opening the PR. **Do not** merge a
     release PR with a known-broken publish path.

6. **Commit and push** on a feature branch with message
   `chore(release): prepare vX.Y.Z`.

7. **Open a PR** titled `chore(release): prepare vX.Y.Z`, including the changelog
   excerpt and a publish-readiness report (which dry-runs ran, what the registry
   currently shows).

8. **Monitor and verify post-merge publishing.** After the squash-merge:
   - Watch `release.yml` complete; confirm tag `vX.Y.Z` and the GitHub Release.
   - Watch `publish.yml` to completion and surface any failure immediately.
   - Independently check crates.io — workflow success is not enough.
   - Declare the release **shipped** only when crates.io serves `X.Y.Z` for every
     crate the release was meant to publish, and docs.rs has built. If one fails,
     open a hotfix PR rather than leaving the release half-published.

## CI Automation

### `release.yml`

- **Trigger**: push to `main` whose commit message starts with
  `chore(release): prepare v`, or manual `workflow_dispatch`.
- **Actions**: extracts the version from the commit subject, verifies it matches
  the root `Cargo.toml`, extracts the matching `CHANGELOG.md` section as release
  notes, creates the GitHub Release with tag `vX.Y.Z`, then explicitly dispatches
  `publish.yml` against the new tag.
- **Why explicit dispatch**: a GitHub Release created with `GITHUB_TOKEN` does
  not fire `release: published` events (anti-recursion), so the downstream
  workflow must be kicked manually.

### `publish.yml`

- **Trigger**: `release: published`, or `workflow_dispatch --ref vX.Y.Z` from
  `release.yml`.
- **Actions**: installs the pinned toolchain, verifies the tag matches the root
  `Cargo.toml`, publishes each crate in dependency order (skipping versions
  already live), then runs `scripts/verify_crates_publish.py`.
- **Secret**: `CARGO_REGISTRY_TOKEN`.

## Pre-Release Checklist

- [ ] All CI checks pass on `main`.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test --all-features` clean.
- [ ] MSRV job green; declared `rust-version` still accurate.
- [ ] `cargo run --example demo -- check` passes; recordings match current
      rendering.
- [ ] `CHANGELOG.md` has an entry for every change since the last release, and
      API changes are called out.
- [ ] `Cargo.toml` and `Cargo.lock` both read `X.Y.Z`.
- [ ] `cargo publish --dry-run -p tuika` succeeds.
- [ ] `X.Y.Z` is greater than the latest crates.io version.
- [ ] Manual terminal matrix walked (below) if the renderer or an escape encoder
      changed.

## Manual Terminal Matrix

Automated tests cover the *protocol* the renderer emits — `tests/pty_smoke.rs`
drives the `gallery` example and asserts alternate-screen enter/exit, OSC 9;4
progress, OSC 8 hyperlinks, 24-bit truecolor SGR, and Braille glyphs. What they
cannot verify is how a specific emulator actually *paints* those bytes. Walk this
before a release when the renderer changed; tick a box only after confirming it
yourself.

Run `cargo run --example gallery` in each terminal and check alt-screen
enter/exit, Braille and wide glyphs, truecolor, mouse-wheel scroll, and that the
footer URL is a clickable OSC 8 link:

- [ ] Ghostty
- [ ] iTerm2
- [ ] WezTerm
- [ ] Kitty
- [ ] Windows Terminal
- [ ] Konsole
- [ ] tmux (truecolor needs `Tc`/`RGB` in `terminal-overrides`)

**Native OSC 9;4 progress** support is a fixed property of each terminal, not
something to re-verify per release. Terminals that render it: **Ghostty** (bar at
the top of the window), **Windows Terminal** and **ConEmu** (taskbar),
**WezTerm**, **Konsole**, **mintty**. Others (e.g. **iTerm2**, **Kitty**)
silently ignore the unknown OSC, so emitting it is safe everywhere.

**OSC 8 hyperlinks** (`HyperlinkBackend`) wrap `http(s)` URL runs so a supporting
terminal makes them clickable: **Ghostty**, **iTerm2**, **WezTerm**, **Kitty**,
**Konsole**, recent **GNOME Terminal / VTE**. Others ignore the escape and render
the URL as plain text, so emitting it is safe everywhere. Unlike OSC 9;4 this one
*is* worth re-checking, because it writes styled spans straight to the terminal:
confirm the link is clickable **and** that surrounding text, colors, and wrapping
are undamaged.

**Graphics protocols** are the exception to "unknown escapes are harmless" and
are gated on capability detection ([images.md](./images.md)); re-check the
`image` example on any terminal whose detection changed.

### Nightly cross-terminal job

`.github/workflows/nightly-terminals.yml` runs the `gallery` example inside real
terminal emulators nightly (and on `workflow_dispatch`), narrowing how much of
the matrix a human has to walk. Legs differ in maturity:

| Leg | Runner | Capture | Status |
|-----|--------|---------|--------|
| tmux | Linux | `capture-pane` text | **Asserted** — `scripts/assert-gallery.sh` gates the job on the box chrome, a real Braille glyph, and the footer URL. |
| kitty | Linux (Xvfb, software GL) | remote-control text + screenshot | Best-effort — artifact only; assertion is a warning. |
| iTerm2 | macOS | AppleScript session text + `screencapture` | Best-effort — artifact for inspection. |
| Windows Terminal | Windows | screenshot | Best-effort — artifact for inspection. |

A green **tmux** leg means the alt-screen / Braille / layout / footer rows are
already verified in a real emulator, so the manual walk reduces to per-emulator
painting. Promote a best-effort leg to asserting once its capture is proven
stable on the runner. The best-effort legs are `continue-on-error`, so a flaky
GUI runner never reports the nightly red on its own.

## Post-Release Verification

Run after the publish workflow finishes. The release is not complete until
crates.io serves `X.Y.Z`.

```bash
cargo search tuika --limit 1                        # shows X.Y.Z
cargo search tuika-codeformatters --limit 1         # if it was part of the release
gh release view vX.Y.Z --repo everruns/tuika        # tag + notes present
```

Also confirm docs.rs built the new version: a docs.rs build failure ships a
release whose documentation is missing, which for a library is close to shipping
nothing. `[package.metadata.docs.rs]` sets `all-features` and the `docsrs` cfg,
so a feature-gated item that fails to compile there is a real defect.

If crates.io is missing the version, inspect the workflow run
(`gh run view <run-id> --log-failed`) and either re-run (transient) or open a
hotfix PR (packaging bug).

## Changelog Format

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Highlights

- 2–5 bullet points summarizing the most impactful changes.

### Breaking Changes

- **Short description**: what changed, why, migration.
  - Before: `old_api`
  - After: `new_api`

### What's Changed

* feat(scope): description ([#42](https://github.com/everruns/tuika/pull/42)) by @contributor
* fix(scope): description ([#41](https://github.com/everruns/tuika/pull/41)) by @contributor

**Full Changelog**: https://github.com/everruns/tuika/compare/vA.B.C...vX.Y.Z
```

Rules:

- PRs listed newest-first by number.
- `### Breaking Changes` only when present; required whenever a `pub` item was
  removed, renamed, or changed signature.
- `### Highlights` is the human summary; `### What's Changed` is the mechanical
  PR list.

## Hotfix Releases

1. Ask the agent: "Cut patch release vX.Y.Z+1 for the &lt;fix&gt;".
2. The agent branches from the latest tag, cherry-picks the fix, runs the same
   pre-release checklist, and opens the PR.
3. Human reviews and merges.

## Rollback

If a published version is broken, yank it:

```bash
cargo yank --version X.Y.Z tuika
```

Yanked versions remain usable by existing `Cargo.lock` files but are not selected
for new resolves. Yanking `tuika` without yanking a `tuika-codeformatters`
release that requires it leaves the formatter unresolvable for new users — yank
or supersede both.

## Authentication

**Repo secrets** (Settings → Secrets and variables → Actions):

| Secret | Used by | Source |
| --- | --- | --- |
| `CARGO_REGISTRY_TOKEN` | `publish.yml` | https://crates.io/settings/tokens — publish scope |

## Related

- [`.agents/skills/release/SKILL.md`](../../.agents/skills/release/SKILL.md)
- [shipping.md](./shipping.md)
- [maintenance.md](./maintenance.md)
