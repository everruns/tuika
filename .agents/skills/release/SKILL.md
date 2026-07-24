---
name: release
description: Cut a new tuika release. Prepares the release PR, runs publish-readiness checks, and monitors CI publish of tuika and tuika-codeformatters to crates.io. Use when the user asks to release, cut a version, or publish to crates.io.
metadata:
  internal: true
user-invocable: true
---

# Release

Goal: cut a new tuika release and verify every crate it was meant to publish
lands on crates.io with documentation built.

This skill implements [`knowledge/specs/release.md`](../../../knowledge/specs/release.md). Keep
operational guidance here. Keep design intent in the spec.

## When To Use

Use this skill when the user asks to:

- release / cut a release / publish vX.Y.Z
- ship to crates.io
- prepare a release PR or a hotfix release

For a generic "ship this change" request (PR → CI → merge of a non-release
change), use [`/ship`](../ship/SKILL.md) instead.

## Required Outcomes

**All outcomes below are MANDATORY.**

1. **The version is correct.** The root `Cargo.toml` and `Cargo.lock` agree on
   `X.Y.Z`, and `X.Y.Z` is strictly greater than the latest version on
   crates.io. If `tuika-codeformatters` is part of the release, its version and
   its `tuika` dependency requirement are consistent too.
2. **The changelog is honest.** `CHANGELOG.md` lists every change landed since
   the previous tag, in descending order, with PR numbers and authors. **Every
   `pub` API addition, rename, removal, or signature change is called out** —
   removals and signature changes under `### Breaking Changes` with a
   before/after snippet.
3. **Publish-readiness is proven before merge.** `cargo publish --dry-run -p
   tuika` succeeds and the packaging test passes. The PR body includes a
   publish-readiness report.
4. **Post-merge verification is a hard gate.** After the release PR merges, the
   agent monitors CI and independently confirms crates.io serves `X.Y.Z` and
   docs.rs built it. A release is not done, shipped, or closed out until then.
5. **A failure rolls forward, not backward.** If a publish fails, open a hotfix
   PR (or fix forward in the same PR if not yet merged). Do not leave the release
   half-shipped.
6. **Durable knowledge is release-current.** Validate the OKF bundle and verify
   that significant released behavior, architecture, policy, and process changes
   since the previous tag are represented, without exposing internal knowledge
   through public documentation links.

## Operating Model

- Releases are agent-prepared, human-merged, CI-published.
- Start by gathering the unreleased commit set, not by guessing the version.
- The agent never tags the release directly — `release.yml` does that when the
  `chore(release): prepare vX.Y.Z` commit lands on `main`.
- The repository tag tracks the **root package** (`tuika`).
  `tuika-codeformatters` versions independently and is published only when its
  in-tree version is not already live.

## Step-By-Step

### 0. Sync local state

Shallow clones lie. Before counting commits, force a full history:

```bash
git fetch --unshallow origin main 2>/dev/null || git fetch origin main
git fetch --tags
```

Cross-check the commit count if anything looks off:

```bash
LATEST=$(git describe --tags --abbrev=0)
git log "$LATEST"..origin/main --oneline | wc -l
gh api "repos/everruns/tuika/compare/$LATEST...main" --jq '.total_commits'
```

If those disagree, the clone is still shallow.

### 1. Pick the version

If the user gave a version, use it. Otherwise propose based on the diff:

- only `fix`, `docs`, `chore`, `refactor`, `test` commits, and no API change →
  patch
- one or more `feat` commits, or any new `pub` item → minor
- any `pub` removal, rename, or signature change, or a `ratatui-core` major bump
  → minor (pre-1.0) with an explicit `### Breaking Changes` block

Check the API delta rather than trusting commit subjects:

```bash
git diff "$LATEST"..HEAD -- src crates/tuika-codeformatters/src | grep -E '^[-+]\s*pub '
```

Confirm with the user before proceeding.

### 2. Update the changelog

`CHANGELOG.md` lives at the repo root. Add a section at the top under any intro:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Highlights

- 2–5 bullets summarizing the most user-visible changes.

### Breaking Changes

- (only when applicable; required whenever a `pub` item changed incompatibly)

### What's Changed

* feat(scope): description ([#42](https://github.com/everruns/tuika/pull/42)) by @contributor
* fix(scope): description ([#41](https://github.com/everruns/tuika/pull/41)) by @contributor

**Full Changelog**: https://github.com/everruns/tuika/compare/vA.B.C...vX.Y.Z
```

Build the PR list mechanically:

```bash
git log "$LATEST"..HEAD --pretty=format:'%s' --reverse \
  | grep -v '^chore(release): prepare v'
```

Map commits to PRs via `gh pr list --state merged --base main --limit 200` when
commit subjects don't carry the PR number.

### 3. Bump the version(s)

Edit the root `Cargo.toml`:

```toml
[package]
name = "tuika"
version = "X.Y.Z"
```

Bump `crates/tuika-codeformatters/Cargo.toml` when its own API changed, **or**
when it must track a new tuika range — and in that case update its dependency
requirement in the same edit, or the published formatter resolves against the
old tuika:

```toml
tuika = { version = "X.Y.0", path = "../.." }
```

Refresh the lockfile:

```bash
cargo update -p tuika
cargo update -p tuika-codeformatters   # only if it was bumped
```

### 4. Run local verification

Review commits since the previous tag for durable knowledge impact. Update the
relevant concepts, `knowledge/index.md`, and `knowledge/log.md` when needed, then
verify the bundle and the code:

```bash
python3 scripts/validate_okf.py knowledge --check-links
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run --example demo -- check
cargo doc --all-features --no-deps
```

`cargo doc` matters more here than in a normal change: docs.rs builds with
`all-features` and the `docsrs` cfg, and a docs.rs failure ships a library whose
documentation is missing.

Confirm `README.md` and `docs/` describe released behavior while remaining
independent of internal `knowledge/` and `.agents/` paths.

### 5. Verify publish-readiness

This is the step that catches what local tests don't — the `cargo publish`
packaging boundary, missing files referenced by `Cargo.toml`, version drift:

```bash
cargo publish --dry-run -p tuika
cargo test --test packaging          # the .crate contents guard
cargo search tuika --limit 1         # confirm CURRENT crates.io version < X.Y.Z
grep '^version' Cargo.toml           # confirm reads X.Y.Z
python3 scripts/publish_order.py     # confirm the order CI will use
```

**Two crates, ordered publish.** `tuika-codeformatters` depends on `tuika` by
version, so crates.io requires tuika live first. `publish.yml` derives the
dependency-first order from Cargo metadata (`tuika`, then
`tuika-codeformatters`) and skips versions already live. A consequence for this
step: **`cargo publish --dry-run -p tuika-codeformatters` fails locally** when it
requires a tuika version that is not yet on crates.io. That is expected, not a
broken release — CI validates it after tuika publishes.

If the tuika dry-run fails, fix the root cause and re-run. Do **not** open a
release PR with a known-broken publish path.

### 6. Commit and push

Stage explicitly — never `git add .`:

```bash
git add CHANGELOG.md Cargo.toml Cargo.lock
git commit -m "chore(release): prepare vX.Y.Z"
```

Publish the branch through the GitHub API (`push_files`) so the release commit
lands **Verified** — see `AGENTS.md` § Signed commits. A release commit that
shows *Unverified* on `main` is a defect; fix it before opening the PR.

### 7. Open the PR

Title: `chore(release): prepare vX.Y.Z` (under 70 chars).

Body must include:

- The full `## [X.Y.Z] - …` changelog section.
- A **Publish-readiness** block:

  ```markdown
  ## Publish-readiness

  - [x] `cargo fmt --all -- --check`
  - [x] `cargo clippy --all-targets --all-features -- -D warnings`
  - [x] `cargo test --all-features`
  - [x] `cargo run --example demo -- check`
  - [x] `cargo publish --dry-run -p tuika` (`tuika-codeformatters` is validated
        in CI after tuika publishes)
  - [x] crates.io currently serves `A.B.C` → publishing `X.Y.Z`
  - [x] `Cargo.toml` + `Cargo.lock` agree on `X.Y.Z`
  ```

- A **Post-merge verification** block stating that the release is not complete
  until the agent has checked:

  ```markdown
  ## Post-merge verification

  - [ ] `release.yml` created tag `vX.Y.Z` and the GitHub Release
  - [ ] `publish.yml` finished green
  - [ ] crates.io serves `X.Y.Z`
  - [ ] docs.rs built `X.Y.Z`
  ```

### 8. Monitor publishing after merge

Subscribe to PR activity for the release PR so the loop wakes you on each
workflow completion. Then watch:

```bash
gh run list --workflow=release.yml --limit 1
gh run list --workflow=publish.yml --limit 1
```

Confirm each finishes green, then run the post-release checks yourself. Do not
ask the user to verify these manually and do not declare the release shipped from
workflow status alone.

```bash
cargo search tuika --limit 1                    # shows X.Y.Z
cargo search tuika-codeformatters --limit 1     # if it was part of the release
gh release view "vX.Y.Z"                        # tag + notes present
curl -sSI "https://docs.rs/tuika/X.Y.Z/tuika/"  # docs built
```

Declare **shipped** only when crates.io reports `X.Y.Z` for every crate the
release was meant to publish and docs.rs has built. If a workflow fails, inspect
logs (`gh run view <id> --log-failed`) and either re-run (transient — network or
registry propagation) or open a hotfix PR (packaging bug — see
[`knowledge/specs/release.md`](../../../knowledge/specs/release.md) § Hotfix
Releases).

## Common Pitfalls

- **Shallow clone.** Cloud sandboxes default to depth ≈ 50 and silently drop
  older commits from `git log`. Always `git fetch --unshallow` first.
- **Forgetting the formatter's dependency requirement.** Bumping tuika without
  updating `tuika-codeformatters`'s `tuika = { version = … }` publishes a
  formatter that resolves against the old tuika. The path dependency hides this
  locally — the workspace builds fine either way.
- **An unpublishable formatter.** If `tuika-codeformatters` requires a tuika
  version that never gets published, it becomes unresolvable for new users. Bump
  the two together or not at all.
- **Yanking only half.** Yanking tuika without yanking a formatter release that
  requires it leaves the formatter unresolvable. Yank or supersede both.
- **Tag/Cargo drift.** A same-day patch release is almost always caused by
  version drift between `Cargo.toml` and what `cargo publish` actually sees. The
  dry-run catches it.
- **Stale demo assets in the tarball.** `docs/hero.gif` ships in the crate. If it
  no longer matches the current look, the crates.io page misrepresents the
  release — regenerate before cutting.
- **Auto-merge.** Do not enable auto-merge on the release PR. A human must click
  the squash button so a real reviewer sees the changelog.

## Authentication

Required repo secret — set up once, see
[`knowledge/specs/release.md`](../../../knowledge/specs/release.md) § Authentication.

- `CARGO_REGISTRY_TOKEN` — crates.io publish scope.
