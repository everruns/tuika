---
name: release
description: Cut a new tuika release. Prepares the release PR, runs publish-readiness checks, and monitors CI publication of tuika and its companion crates. Use when the user asks to release, cut a version, or publish to crates.io.
metadata:
  internal: true
user-invocable: true
---

# Release

Goal: cut a new tuika release and verify every crate it was meant to publish
lands on crates.io with documentation built.

This skill implements [`knowledge/processes/release.md`](../../../knowledge/processes/release.md). Keep
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
   crates.io. If a companion crate is part of the release, its version and its
   `tuika` dependency requirement are consistent too.
2. **The changelog is honest.** `CHANGELOG.md` lists every change landed since
   the previous tag, in descending order, with no commit links and contributor
   attribution for anyone other than @chaliy. **Every `pub` API addition,
   rename, removal, or signature change is called out** — removals and signature
   changes under `### Breaking Changes` with a before/after snippet.
3. **The highlights show the release.** Unless the release changes nothing
   visual, `### Highlights` embeds a VHS recording of the one or two most
   TUI-centric features, taken from the `DEMOS` gallery and pinned to `vX.Y.Z`.
4. **Publish-readiness is proven before merge.** `cargo publish --dry-run -p
   tuika` succeeds and the packaging test passes. The PR body includes a
   publish-readiness report.
5. **Post-merge verification is a hard gate.** After the release PR merges, the
   agent monitors CI and independently confirms crates.io serves `X.Y.Z` and
   docs.rs built it. A release is not done, shipped, or closed out until then.
6. **A failure rolls forward, not backward.** If a publish fails, open a hotfix
   PR (or fix forward in the same PR if not yet merged). Do not leave the release
   half-shipped.
7. **Durable knowledge is release-current.** Validate the OKF bundle and verify
   that significant released behavior, architecture, policy, and process changes
   since the previous tag are represented, without exposing internal knowledge
   through public documentation links.

## Operating Model

- Releases are agent-prepared, human-merged, CI-published.
- Start by gathering the unreleased commit set, not by guessing the version.
- The agent never tags the release directly — `release.yml` does that when the
  `chore(release): prepare vX.Y.Z` commit lands on `main`.
- The repository tag tracks the **root package** (`tuika`). Companion crates
  version independently and are published only when their in-tree versions are
  not already live.

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

**No tag yet is a valid state, not a shallow clone.** tuika 0.1.0–0.4.0 were
published from the yolop workspace before the extraction, so this repository
carries no tag for them and `git describe --tags` fails outright until the first
release is cut here. Do not invent one to satisfy the command: no commit in this
repository produced any of those `.crate` files, so a backfilled tag would point
at a tree that was never published, and every consumer of that tag — `compare`
links, tag-pinned demo URLs, "changes since" lists — would be wrong. Use the full
history instead, and take the previous version from crates.io rather than from a
tag:

```bash
git describe --tags --abbrev=0 2>/dev/null || git rev-list --max-parents=0 HEAD
curl -s https://index.crates.io/tu/ik/tuika | tail -1   # last published version
```

### 1. Pick the version

If the user gave a version, use it. Otherwise propose based on the diff:

- only `fix`, `docs`, `chore`, `refactor`, `test` commits, and no API change →
  patch
- one or more `feat` commits, or any new `pub` item → minor
- any `pub` removal, rename, or signature change, or a `ratatui-core` major bump
  → minor (pre-1.0) with an explicit `### Breaking Changes` block

Check the API delta rather than trusting commit subjects:

```bash
git diff "$LATEST"..HEAD -- src crates/*/src | grep -E '^[-+]\s*pub '
```

Confirm with the user before proceeding.

### 2. Record the release demo

Before writing the changelog, decide which one or two features of this release
are worth *watching* — the TUI-centric ones — and make sure each has a current
recording. Demos come from the `DEMOS` registry in `examples/demo.rs`; a release
never introduces a one-off asset.

```bash
cargo run --example demo -- list                 # existing scenes
cargo run --example demo -- <scene> --dump       # confirm it shows the feature
scripts/gen-demos.sh <scene>                     # (re)record it
```

If the highlighted feature has no scene, add one first (`scene_*` builder plus a
`DEMOS` entry — see [`AGENTS.md`](../../../AGENTS.md) § Adding a component demo),
then record it. If it has a scene that predates the change, re-record it: a
release note that shows the old behavior is worse than no demo.

Skip this step only when the release genuinely changes nothing visible —
dependency bumps, internal refactors, a docs-only patch.

### 3. Update the changelog

`CHANGELOG.md` lives at the repo root. Add a section at the top under any intro:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Highlights

**One-line feature title** — what a host can now do, in a sentence or two.

![<scene> demo](https://raw.githubusercontent.com/everruns/tuika/vX.Y.Z/docs/demos/<scene>.gif)

- **Performance**: one line, with a number.
- **Security**: one line, with the impact.

### Breaking Changes

- (only when applicable; required whenever a `pub` item changed incompatibly)

### What's Changed

* feat(scope): description
* fix(scope): description by @contributor
```

Three rules carry the format ([spec](../../../knowledge/processes/release.md#changelog-format)):

- **The embed is pinned to `vX.Y.Z`, not `main`.** It 404s until `release.yml`
  creates the tag — expected, and it resolves before anyone reads the notes.
- **No commit links, no `compare/…` line.** History gets rewritten here, so
  those URLs rot. A bare `(#42)` is fine for changes that came in via a PR.
- **`by @handle` only for authors other than @chaliy.**

Build the list mechanically, keeping the author beside each subject so the
attribution rule can be applied without guessing:

```bash
git log "$LATEST"..HEAD --pretty=format:'%an%x09%s' \
  | grep -v $'\t''chore(release): prepare v'
```

Everything authored by Mykhailo Chalyi (`mike@chaliy.name`) loses its
attribution; anything else keeps ` by @handle`, resolved from the contributor's
GitHub account (`gh pr list --state merged --base main --limit 200` maps an
external contribution to its PR number and handle).

### 4. Bump the version(s)

Edit the root `Cargo.toml`:

```toml
[package]
name = "tuika"
version = "X.Y.Z"
```

Bump either companion's `Cargo.toml` when its own API changed, **or** when it
must track a new tuika range — and in that case update its dependency
requirement in the same edit, or the published companion resolves against old
tuika:

```toml
tuika = { version = "X.Y.0", path = "../.." }
```

Refresh the lockfile:

```bash
cargo update -p tuika
cargo update -p tuika-codeformatters   # only if it was bumped
cargo update -p tuika-mermaid          # only if it was bumped
```

Update every release-pinned root README URL from the previous tag to `vX.Y.Z`:
the public guide links under `https://github.com/everruns/tuika/blob/vX.Y.Z/`
and the three raw image URLs (`logo.svg`, `docs/hero.gif`, and
`docs/demos/image.svg`). The packaging test derives `CARGO_PKG_VERSION` and
fails if a guide or image points at another tag.

### 5. Run local verification

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

### 6. Verify publish-readiness

This is the step that catches what local tests don't — the `cargo publish`
packaging boundary, missing files referenced by `Cargo.toml`, version drift:

```bash
cargo publish --dry-run -p tuika
cargo test --test packaging          # the .crate contents guard
cargo search tuika --limit 1         # confirm CURRENT crates.io version < X.Y.Z
grep '^version' Cargo.toml           # confirm reads X.Y.Z
python3 scripts/publish_order.py     # confirm the order CI will use
```

**Dependency-ordered publish.** Both companions depend on `tuika` by version, so
crates.io requires tuika live first. `publish.yml` derives the order from Cargo
metadata and skips versions already live. A companion dry-run can fail locally
when it requires a tuika version that is not yet on crates.io. That is expected,
not a broken release — CI validates it after tuika publishes.

If the tuika dry-run fails, fix the root cause and re-run. Do **not** open a
release PR with a known-broken publish path.

### 7. Commit and push

Stage explicitly — never `git add .`:

```bash
git add CHANGELOG.md Cargo.toml Cargo.lock
git add docs/demos/<scene>.gif examples/demo.rs   # if a demo was recorded or added
git commit -m "chore(release): prepare vX.Y.Z"
git push -u origin "$(git branch --show-current)"
```

Commits are signed — see [`AGENTS.md`](../../../AGENTS.md) § Signing. `git log
--format='%h %G? %s'` must show `G` (or `U`) for the release commit before it is
pushed.

### 8. Open the PR

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
  - [x] `cargo publish --dry-run -p tuika` (companions are validated in CI after
        tuika publishes)
  - [x] crates.io currently serves `A.B.C` → publishing `X.Y.Z`
  - [x] `Cargo.toml` + `Cargo.lock` agree on `X.Y.Z`
  - [x] highlight demo(s) re-recorded and embedded, pinned to `vX.Y.Z`
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

### 9. Monitor publishing after merge

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
cargo search tuika-mermaid --limit 1            # if it was part of the release
gh release view "vX.Y.Z"                        # tag + notes present
curl -sSI "https://docs.rs/tuika/X.Y.Z/tuika/"  # docs built
```

Declare **shipped** only when crates.io reports `X.Y.Z` for every crate the
release was meant to publish and docs.rs has built. If a workflow fails, inspect
logs (`gh run view <id> --log-failed`) and either re-run (transient — network or
registry propagation) or open a hotfix PR (packaging bug — see
[`knowledge/processes/release.md`](../../../knowledge/processes/release.md) § Hotfix
Releases).

## Common Pitfalls

- **Shallow clone.** Cloud sandboxes default to depth ≈ 50 and silently drop
  older commits from `git log`. Always `git fetch --unshallow` first.
- **Forgetting a companion's dependency requirement.** Bumping tuika without
  updating a companion's `tuika = { version = … }` publishes it against old
  tuika. The path dependency hides this locally.
- **An unpublishable companion.** If a companion requires a tuika version that
  never gets published, it becomes unresolvable for new users. Bump them
  together or not at all.
- **Yanking only the root.** Yanking tuika without yanking companion releases
  that require it leaves those releases unresolvable. Yank or supersede all
  affected versions.
- **Tag/Cargo drift.** A same-day patch release is almost always caused by
  version drift between `Cargo.toml` and what `cargo publish` actually sees. The
  dry-run catches it.
- **A stale root README repository pin.** Root guides and images live outside
  the crate and pin the release tag. Update every pinned URL with the version
  bump; the packaging test guards this mechanically.
- **A highlight demo that shows the old behavior.** Recording is part of
  preparing the release, not something to inherit from the gallery. If the
  scene existed before the change, re-record it, or the release note advertises
  a feature while showing its predecessor.
- **A `main`-pinned embed.** `raw.githubusercontent.com/.../main/...` is right
  for rustdoc, wrong for a changelog: the next re-recording silently rewrites
  what every past release appears to have shipped. Release notes pin `vX.Y.Z`.
- **A demo GIF left out of the release commit.** Staging only `CHANGELOG.md`,
  `Cargo.toml`, and `Cargo.lock` pushes a changelog whose embed has nothing
  behind it once the tag is cut.
- **Auto-merge.** Do not enable auto-merge on the release PR. A human must click
  the squash button so a real reviewer sees the changelog.

## Authentication

Required repo secret — set up once, see
[`knowledge/processes/release.md`](../../../knowledge/processes/release.md) § Authentication.

- `CARGO_REGISTRY_TOKEN` — crates.io publish scope.
