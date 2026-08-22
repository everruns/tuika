# tuika — coding-agent guidance

`tuika` is a standalone, published terminal-UI toolkit: flexbox layout, anchored
overlays, focus, a keymap engine, and components over ratatui — including the
streaming `Markdown` renderer and `CodeBlock`. It is host-agnostic and knows
nothing about the applications that embed it. See `README.md` for the model.

This file is read on every turn by the agent itself when run inside this
repository, so keep it short, factual, and project-specific.

## Workflow

- Telegraph. Drop filler. Keep updates short and factual.
- Fix the root cause. If unsure, read more code; if still stuck, ask with short options.
- Unrecognized working-tree changes are probably from another agent or the user. Work with them. Stop only if they make the task unsafe.
- Start from latest `main` by default: `git fetch origin main`, then branch from or rebase onto `origin/main`.
- Keep changes small, PR-sized, testable, and runnable locally.
- For bug fixes, write or update a failing test before the fix when practical.
- Important decisions belong as concise comments near the relevant code, not in scratch docs.
- Never create GitHub issues for follow-ups discovered during internal or
  maintainer work. Keep them in the current PR's **Follow-ups**, durable
  knowledge, or local planning. Create an issue only when a maintainer
  explicitly asks; otherwise update an existing externally reported issue.
- tuika is pre-1.0, but it is a **published library**: every `pub` item is API. A
  breaking change is allowed in a minor release, and must be called out in the
  changelog rather than slipped in.
- Where a new public item goes is decided by
  [`knowledge/specs/api-surface.md`](knowledge/specs/api-surface.md), not by
  convenience: the crate root is the framework spine, `components` holds every
  `View`, `term` holds everything that talks to the terminal outside the cell
  grid, and `prelude` is the one-line import. One canonical path per item.
- Keep the dependency set small. Anything heavy — syntax grammars, diagram
  layout, image decoders, HTTP — belongs in the host or a companion crate
  (`tuika-codeformatters`, `tuika-mermaid`, `tuika-html`), behind a trait tuika
  defines.

## Knowledge and docs

- `knowledge/` is the repository's Open Knowledge Format (OKF) bundle and durable
  development memory. Read `knowledge/index.md` first, then only the concepts
  relevant to the task and their links.
- When a change alters durable behavior, intent, architecture, policy, constraints,
  terminology, or maintainer process, update the affected concepts in the same
  change. Update `knowledge/index.md` when concepts are added, removed, renamed, or
  reclassified; update `knowledge/log.md` for significant knowledge changes.
- Keep transient plans, task status, test output, and source-level details out of
  the bundle. Knowledge captures **why** and **what**, not exhaustive **how**.
- `README.md` is the public entry point (it is also the crates.io README);
  `docs/` contains standalone guidance for external users. Neither may link to
  internal `knowledge/` or `.agents/` material. See
  `knowledge/specs/documentation.md` for the documentation contract.

## Local dev and tests

This is a small Cargo **workspace**: the root package is the `tuika` library;
`crates/tuika-codeformatters/` is the tree-sitter `Highlighter`,
`crates/tuika-mermaid/` is the mmdflux `MarkdownBlockRenderer`, and
`crates/tuika-html/` is the html5ever `MarkdownBlockRenderer` (plus its own `Html`
view). All three are published separately so tuika core stays grammar-,
diagram-engine-, and HTML-parser-free.
Cargo defaults to the root package only, so pass `--workspace` to cover the
member too — an API change that breaks `tuika-codeformatters` is otherwise
invisible locally and fails in CI. For touched code:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

The MSRV is **1.88** (`rust-version` in `Cargo.toml`) and is enforced by a
dedicated CI job; `rust-toolchain.toml` pins a newer toolchain for day-to-day
development, so an MSRV break will not show up locally. If you reach for a newer
language feature, either avoid it or raise the MSRV deliberately.

Run any example to see a change in a real terminal (`q`/`Esc` quits):

```bash
cargo run --example gallery       # motion components + OSC 9;4 progress
cargo run --example markdown      # streaming markdown + highlighted code
cargo run --example image         # graphics protocols + alt-text fallback
cargo run --example split_footer  # split-footer screen mode over live scrollback
cargo run --example codex -- --split-footer   # the agent UI in that mode
```

### Benchmarks

Criterion targets measure wall clock and are advisory; the `*_iai` targets count
instructions against a committed baseline and are a CI gate. Procedure lives in
[`benches/README.md`](benches/README.md). The baseline snapshots are repository
CI inputs and are excluded from the published crates; benchmark source remains.

### Testing

Layout and rendering are tested hermetically by rendering into an in-memory
ratatui `Buffer` and reading cells back — no real terminal. The consumer-facing
subset (`testing::{render, render_sizes, grid}`) is documented in the README;
tuika's own suite covers more:

- **Unit tests** — each module's own `#[cfg(test)] mod tests`: layout math,
  component rendering, interactive state (scroll/select/focus), keymap dispatch,
  compositor, easing, OSC encoders, and palette (every themed cell pinned to its
  `Theme` slot).
- **Cross-module integration** (`src/tests/integration.rs`) — checks that span several
  modules and have no single owner: composing a real tree, and surviving
  tiny/degenerate screens where scroll and overlay resolution interact.
- **Property tests** (`src/tests/proptests.rs`, `proptest`) — solver and overlay
  invariants for *any* input (children stay in bounds, flex fills exactly).
- **Fuzz** (`src/tests/fuzz.rs`) — adversarial text (wide CJK, ZWJ emoji,
  combining marks, control bytes) and arbitrary event streams through the wrap
  solver, composed trees, the stateful components, and the parsers that read
  untrusted bytes; plus differential properties (streamed markdown vs one-shot,
  styled wrap vs plain). Raise the case count when hunting: `PROPTEST_CASES=5000`.
  CI runs the default count per PR and a deep run nightly
  (`.github/workflows/nightly-fuzz.yml`); commit any new `proptest-regressions/`
  seed with the fix.
- **Session stress** (`tests/stress_ui.rs`) — a whole session rather than one
  component: every `ScreenMode` driven through `Runner`/`AsyncRunner` over an
  in-memory screen that resizes *between* frames, mode changes mid-session,
  adversarial scrollback publishing, and shell/overlay/dock composition at
  degenerate sizes. Seeded and replayable; raise the event count when hunting
  with `TUIKA_STRESS_EVENTS=8000`.
- **Black-box sweep** (`tests/robustness.rs`) — every component × adversarial
  corpus × degenerate size through the published API only, asserting no panic,
  no paint outside the component's rect, and no control byte in a cell.
- **Golden snapshots** (`src/tests/snapshots.rs`) — whole screens diffed against
  checked-in glyph grids; refresh with `UPDATE_SNAPSHOTS=1`. The grids are
  LF-only (`.gitattributes`), so a CRLF checkout cannot fail them.
- **Resize / degenerate sizes** — a size sweep from `0×0` up asserts no panic and
  no out-of-clip writes.
- **PTY smoke** (`tests/pty_smoke.rs`) — drives examples under a pseudo-terminal
  and asserts the terminal-facing protocol: `gallery` for the alternate screen
  (enter/leave, cursor, keyboard reporting, native mouse default plus explicit
  capture lifecycle, OSC 9;4 progress, OSC 8 hyperlinks, truecolor and Braille
  cells through a reference terminal parser, resize survival, clean exit),
  `markdown` for a streaming OSC 8 link, and `split_footer`/`codex
  --split-footer` for the split-footer mode (no alt-screen, no mouse capture, the
  footer pinned to the bottom with published blocks above it, and its rows handed
  back on exit with the scrollback intact). The harness answers the
  cursor-position query an inline viewport is anchored with, from a vt100 model
  of the same stream. It launches *built* examples, so any runner that rebuilds
  the suite elsewhere — CI's coverage step, notably — has to build the examples
  into that same directory.
- **Packaging** (`tests/packaging.rs`) — drives `cargo package --list` so the
  published `.crate` never re-inflates with repo-only files or heavy assets.

## Docs layout

- `logo.svg`, `logo-dark.svg`, and `logo-mono.svg` — the hand-authored vector
  sources for tuika's primary, dark-surface, and one-color marks. The primary is
  embedded in the README through a release-tag-pinned absolute URL, so none of
  the rendered copies need to ship in the crate.
  Their 1024 px PNG exports (`logo*.png`) are GitHub-only; regenerate them with
  `scripts/gen-logo-assets.sh`.
- `docs/components.md` — the public component-gallery index, covering **every**
  component and linking into focused family pages under `docs/components/`.
  Each family page owns the descriptions and demos, including those published
  in a companion crate. Keep the gallery **presentational only**: no build or
  regeneration instructions belong there (they live in this file). The README
  links to entries there rather than explaining a component itself; see
  `knowledge/specs/documentation.md`.
- `docs/layout.md` — the public layout guide: wrapping, flex-item sizing,
  Flow/Grid selection, measurement requests, and migration notes.
- `docs/markdown.md` — the markdown guide: streaming, GFM tables, the
  highlighter boundary, link policy, and images in one page. It reuses the gallery's
  `DEMOS` recordings, so it is inside the `demo -- check` reference invariant.
- `docs/demos/*.{gif,png}` — the committed demo recordings referenced by the
  component family pages and, via `raw.githubusercontent.com` URLs, inline on the
  relevant type's rustdoc so they render on docs.rs — each component's `struct`
  doc, plus module-level types like `OverlaySpec`.
- `docs/hero.gif` — the README hero: a recording of a composite gallery screen.
  See [Hero screenshot](#hero-screenshot).
- `docs/split-footer.gif` — the split-footer screen mode. Beside the hero rather
  than in `docs/demos/`, because it records a whole terminal session instead of a
  registry scene. See [Split-footer demo](#split-footer-demo).
- `docs/showcases.md` + `docs/showcases/*.gif` — applications built on tuika, one
  recording each. See [Showcase demos](#showcase-demos).
- A recording of a whole runnable example lives **beside that example**, so its
  directory stays self-contained: `examples/codex/codex.gif` (a replica of the
  Codex CLI's UI — label it as one wherever it is embedded, see
  `knowledge/specs/documentation.md`), embedded in `docs/showcases.md` while the
  README's runnable-examples table links to the example source, and regenerated
  by [`scripts/gen-codex-demo.sh`](scripts/gen-codex-demo.sh), which drives the
  real `codex` binary under VHS so the recording cannot drift from the example.
  The in-repo application-shell showcase follows the same rule at
  `examples/workbench_demo/workbench-demo.gif`, regenerated by
  `scripts/gen-workbench-demo.sh` from the real `workbench_demo` binary in its
  original warm copper-and-plum palette.
  These are outside the `demo -- check` invariant, which covers single-component
  scenes.
  The `tuika-mermaid` integration recording follows the same pattern at
  `crates/tuika-mermaid/examples/mermaid_markdown/mermaid.gif`; regenerate it
  with `scripts/gen-mermaid-demo.sh`.
  `tuika-charts` has paired portable-cell and terminal-graphics screenshots for
  every series kind under `docs/charts/`, embedded by its README, the public
  chart guide, and `Chart` rustdoc. Regenerate them with
  `scripts/gen-chart-demo.sh`. The graphics pass builds the pinned Everruns VHS
  fork under `target/` with its Ghostty renderer and requires Go 1.26,
  pkg-config, and Zig 0.15.2 in addition to VHS's normal dependencies.

  `tuika-html` has two, one per example — the markdown boundary
  (`examples/html_markdown/html.png`) and the standalone `Html` component
  (`examples/html_view/html_view.png`) — both regenerated with
  `scripts/gen-html-demo.sh`. It is a *screenshot* rather than a GIF because the
  scene is settled, the same rule the component gallery applies — and the
  example runs as a real app rather than printing a grid, because a plain-text
  dump would throw away the styling that is half of what the crate does.

The root package's public `docs/` tree and alternate logo/demo assets are
repository-only: `Cargo.toml`'s `exclude` keeps them (and the generated `site/`
bundle plus repository machinery — `knowledge/`, `.agents/`, `.github/`,
`scripts/`) out of tuika's published `.crate`, and `tests/packaging.rs` guards
that split. The root README's guide links and logo use release-tag-pinned
absolute URLs; the hero and image demo track `main` so regenerated visuals show
up immediately. The split-footer recording lives only in the focused guide, so
no `docs/` file ships in tuika.

Every published member owns the same rule, and how its README embeds a recording
decides the answer: `tuika-mermaid`'s small recording ships, because its README
reaches it by relative path and the crates.io page would break without it, while
`tuika-codeformatters` excludes `docs/*.gif`, because its README embeds by
absolute URL and no crate consumer can reach the packaged copy. Regenerate its
language gallery with `scripts/gen-language-demo.sh`.
`tests/packaging.rs` covers all four crates.

## Component demos

One example is the single source of truth for the gallery:
[`examples/demo.rs`](examples/demo.rs). Its `DEMOS` registry declares every
scene (name, blurb, recording size, builder); the CLI, the tape generator, and
the integrity check all read it.

For a complete repository-wide refresh, including the hero, theme and styling
galleries, the split-footer recording, the generated image SVG, companion-crate
recordings, the Codex example, and the external showcases:

```bash
scripts/gen-all-demos.sh
scripts/gen-all-demos.sh --skip-showcases # complete local-only refresh
```

The umbrella script is the canonical inventory of committed demo generators.
Showcases are included by default because “all demos” includes external hosts;
the opt-out must be explicit.

```bash
cargo run --example demo -- list             # list scenes
cargo run --example demo -- spinner          # interactive (q/esc quits)
cargo run --example demo -- spinner --dump   # print one frame as text
cargo run --example demo -- check            # verify the docs assets
```

### Regenerating the assets

[`scripts/gen-demos.sh`](scripts/gen-demos.sh) rebuilds every recording. It asks
the example to emit one VHS tape per scene into a temp dir — **tapes are
generated, not committed** — records each, and runs `check`. Requires
[VHS](https://github.com/charmbracelet/vhs) with `ttyd` and `ffmpeg` on `PATH`.

```bash
scripts/gen-demos.sh              # all scenes
scripts/gen-demos.sh spinner tabs # just these
```

Recordings are captured at exactly 1760 px and displayed at `width="880"`
(rustdoc uses its own `max-width`). The integer 2× scale keeps terminal glyphs
crisp instead of asking the browser to resample fractional source pixels.
Motion scenes are GIFs; settled scenes are full-color PNG screenshots so text
antialiasing is not reduced to GIF's palette. Repository-owned captures use
`solarized-dark`; the per-theme comparison gallery remains one capture per
bundled theme, external showcases retain their host application's palette, and
the Workbench showcase retains the original palette it demonstrates.

A tape is sized in *pixels*; how many rows and columns that buys is up to the
emulator's font metrics, so the harness pins each scene to `RECORD_COLS × rows`
and paints the surplus in the theme background. The registry, not the recording
host, decides what a scene's frame is — which is what makes `--dump` and the
clipping check below trustworthy.

The theme and stylesheet galleries have their own generators —
[`scripts/gen-theme-demos.sh`](scripts/gen-theme-demos.sh) (one recording per
bundled theme, from `tuika::themes::PRESETS`) and
[`scripts/gen-styling-demos.sh`](scripts/gen-styling-demos.sh) (one per
`StyleSheet` variant, plus a live-cycling capture).

### Adding a component demo

1. Add a `scene_*` builder and a `DEMOS` entry in `examples/demo.rs` (set
   `rows` to the frame height and `animated` for motion scenes). Use
   `filling_demo` instead of `demo` only for a scene that runs past the bottom of
   its frame on purpose — a viewport or a log tail.
2. Confirm it renders: `cargo run --example demo -- <name> --dump`. The dump uses
   the scene's recorded geometry, so what it prints is what the asset will show —
   including anything `rows` is too small to fit.
3. Record it: `scripts/gen-demos.sh <name>`.
4. Reference `demos/<name>.gif` for an animated scene or `demos/<name>.png` for
   a settled scene in the relevant `docs/components/*.md` family page and inline on the
   component's `struct` doc (via the `raw.githubusercontent.com/.../main/...`
   URL, so docs.rs resolves it).

## Hero screenshot

The README hero (`docs/hero.gif`) is a VHS recording of a composite "app" scene
that exercises most of the toolkit at once. The scene lives in
[`examples/screenshot.rs`](examples/screenshot.rs) as one `scene()` builder, and
[`scripts/gen-hero.sh`](scripts/gen-hero.sh) records it: it builds the example,
runs it full-screen under VHS with `--theme solarized-dark` (window bar for
chrome, terminal background matched to the scene), and writes the GIF. Requires
the same toolchain as the demos — VHS with `ttyd` and `ffmpeg`.

```bash
scripts/gen-hero.sh
```

Regenerate it whenever the components' look changes so the hero stays truthful.

The example doubles as an **offline, recorder-free** path: the same `scene()`
also serializes to a crisp animated SVG (block glyphs painted as rects; static
cells shared across frames, only the moving region duplicated). Handy when VHS
isn't available, or for a vector asset.

```bash
cargo run --example screenshot                         # animate in a terminal
cargo run --example screenshot -- out.svg              # write an SVG
cargo run --example screenshot -- docs/hero.svg        # write the doc asset
cargo run --example screenshot -- --theme gruvbox-dark # themed terminal
cargo run --example screenshot -- --dump               # print one frame as text
```

### The check invariant

`demo -- check` asserts every scene has a non-empty recording in its declared
format, no orphan or stale-format asset lingers, every referenced demo asset
in the gallery markdown (`components.md`, `components/*.md`, `features.md`,
`markdown.md`) or a rustdoc embed (component docs plus
module-level docs like `overlay.rs`) maps to a real scene, and no scene is
clipped by its own frame — it re-renders each one with room to spare and fails on
any line the recorded height would cut off (`filling_demo` scenes are exempt). It
runs in the CI MSRV job and at the end of the generator, so gallery drift fails
CI instead of shipping a broken image to docs.rs.

## Showcase demos

`docs/showcases.md` lists applications built on tuika, one recording each in
`docs/showcases/`. These record *other projects*, so there is no in-repo scene:
[`scripts/gen-showcase-demos.sh`](scripts/gen-showcase-demos.sh) clones each host
into a cache directory (`TUIKA_SHOWCASE_CACHE`, default
`~/.cache/tuika-showcases`), builds it, and records it under VHS. Checkouts and
build artifacts are cached because a full yolop build is slow.

```bash
scripts/gen-showcase-demos.sh          # both scenes
scripts/gen-showcase-demos.sh llmsim   # just this one
```

Both scenes are driven by a local [LLMSim](https://github.com/chaliy/llmsim), so
neither needs a provider key and neither reaches a model: yolop talks to a
*scripted* simulator (a fixed tool call plus a fixed answer, so the transcript is
identical on every run), and the dashboard scene records the simulator itself
under a traffic loop the script drives from outside the recording.

Showcase GIFs live outside `docs/demos/`, so they are outside the `demo -- check`
invariant — an unreferenced one is not a CI failure. When adding a host, add the
scene function and its `case` arm to the generator, then a section to
`docs/showcases.md` and a bullet to the README's *Used in* list.

Recordings are captured at the same pixel density as the component demos (~19 px
cells against the displayed `width="880"`, over 2×), on the smallest cell grid the
host's UI actually needs — a showcase sits beside the demos, so a softer one looks
broken. Push the frame much past that and VHS stops keeping up: the capture loses
frames, and the GIF then plays back *faster* than the session really ran. After
regenerating, check the duration against the tape, not just the sharpness:

```bash
ffprobe -v error -show_entries format=duration -of csv=p=0 docs/showcases/yolop.gif
```

## Split-footer demo

`docs/split-footer.gif` (embedded in `docs/features.md` and `ScreenMode`'s
rustdoc) shows a whole terminal, not a component: the footer *and*
the scrollback above it, which no `Buffer` holds — so it cannot come from the
`DEMOS` registry. It is recorded from the real session by
[`scripts/gen-split-footer-demo.sh`](scripts/gen-split-footer-demo.sh), which
drives the built `split_footer` example under VHS at the gallery's density
(66×14 cells, `FontSize 40`), then sends `q` and keeps recording: the last
seconds are the point of the mode — the published blocks stay, the footer's rows
are handed back.

```bash
scripts/gen-split-footer-demo.sh
```

[`examples/split_footer_demo.rs`](examples/split_footer_demo.rs) is the
**recorder-free** path to the same scene, the way `examples/screenshot.rs` is
for the hero: it runs the built example under a pseudo-terminal — the same
harness `tests/pty_smoke.rs` asserts against — samples the grid through vt100,
and writes an animated SVG. Nothing it produces is committed; use it when
VHS/ttyd is unavailable, or for the text dump.

```bash
cargo run --example split_footer_demo                 # run in a real terminal
cargo run --example split_footer_demo -- out.svg      # write an SVG
cargo run --example split_footer_demo -- --dump       # the frames as text
```

Being a whole-terminal recording rather than a registry scene, it is outside the
`demo -- check` invariant.

## Image demo

The Images feature in `docs/features.md` (`docs/demos/image.svg`) can't be
recorded with VHS: VHS captures through `ttyd` + `xterm.js`, which doesn't
implement the Kitty graphics protocol, so a recording would only ever show the
text fallback — never the pixels. So, like the offline SVG path for the hero, the
demo is **generated from the real render** by
[`examples/image_demo.rs`](examples/image_demo.rs): the picture is the actual
RGBA `ImageData` the component transmits (embedded as a dependency-free PNG), and
the fallback panel is the exact placeholder string the component paints. As a
standalone generated asset, it is outside the registry-based `demo -- check`
invariant, so it needs no `DEMOS` scene.

```bash
cargo run --example image_demo                         # run in a real terminal
cargo run --example image_demo -- out.svg              # write an SVG
cargo run --example image_demo -- docs/demos/image.svg --theme solarized-dark # write the doc asset
```

## Cross-terminal checks

`.github/workflows/nightly-terminals.yml` runs the `gallery` example inside real
terminal emulators nightly. The in-repo tests prove tuika emits the right bytes;
the nightly checks how emulators *interpret* them. The tmux leg asserts via
[`scripts/assert-gallery.sh`](scripts/assert-gallery.sh); the GUI legs (kitty,
iTerm2, Windows Terminal) capture artifacts best-effort. Because the assertions
read the gallery's on-screen text, its box titles, Braille spinner, and footer
URL are load-bearing — changing them means updating the assert script.

## Secrets

- Shared repository secrets live in [Doppler](https://doppler.com). Nothing
  secret belongs in the repository, in a `.env` file, in a workflow literal, or
  pasted into a command line. A maintainer's existing SSH signing key stays in
  their normal OS/Git setup; it is a personal identity, not a repository secret.
- Project **`everruns-dev`**, config **`dev`**.
- Read secrets at the point of use instead of copying values around:

```bash
doppler run --project everruns-dev --config dev -- <command>
doppler run --only-secrets COMMIT_SIGNING_KEY_B64 --project everruns-dev --config dev -- <command>
```

- A preauthenticated Doppler CLI is sufficient; `DOPPLER_TOKEN` need not also
  be present. If a task actually needs Doppler and neither authentication path
  works, stop and ask — do not fall back to an unmanaged credential.
- Never print a secret to stdout, a log, or a commit. Write key material to a
  private path with `0600` permissions and delete it when done.

## Git and commits

- Conventional Commits: `type(scope): description`.
- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.
- Use `chore` for updates to `knowledge/`, `AGENTS.md`, or CI metadata.
- Never add Claude/session/AI attribution links in commits, PRs, docs, or code comments.
- Stage files explicitly by name. Avoid broad `git add .` / `git add -A`.

Commit attribution must be a real human user. If git identity is missing or
agent-like, stop and ask before committing.

### Signing

**Every commit must be signed and must show as Verified on GitHub.** This is a
hard requirement, not a preference: an unsigned commit on any branch is a defect
to fix before pushing. Rewrites count too — `git rebase`, `git cherry-pick`, and
`git commit --amend` all drop signatures unless told otherwise
(`git rebase --gpg-sign`).

Use the real human identity already configured in Git first. Both SSH and
OpenPGP signatures are acceptable when GitHub recognizes the signing key. A
typical SSH setup is:

```bash
git config --local gpg.format ssh
git config --local user.signingkey ~/.ssh/id_ed25519.pub
git config --local commit.gpgsign true
git config --local tag.gpgsign true
```

Use the maintainer's actual configured public-key path; do not assume the
example filename. Do not replace a working SSH setup merely because Doppler is
available.

When no usable personal signing key is configured, Doppler holds the backup
OpenPGP identity as `COMMIT_SIGNING_KEY_B64`, with its fingerprint in
`COMMIT_SIGNING_KEY_ID`. Import it into a throwaway keyring and point Git at it:

```bash
export GNUPGHOME="$(mktemp -d)"   # throwaway keyring; never ~/.gnupg
doppler run --only-secrets COMMIT_SIGNING_KEY_B64 --project everruns-dev --config dev \
  -- sh -c 'printf %s "$COMMIT_SIGNING_KEY_B64" | base64 -d | gpg --batch --import'

git config --local gpg.format openpgp
git config --local user.signingkey FA3D613308B45D42D2D437FF6B554BC31F96585D
git config --local commit.gpgsign true
git config --local tag.gpgsign true
```

The fingerprint above is the current `COMMIT_SIGNING_KEY_ID`; read it from
Doppler if the key is rotated. `GNUPGHOME` must stay exported for every OpenPGP
commit or rewrite in the session.

Confirm before pushing — `%G?` must be `G` (or `U`) for every commit, never `N`.
For SSH signatures, configure `gpg.ssh.allowedSignersFile` locally when needed
so Git can verify the same public key GitHub recognizes:

```bash
git log --format='%h %G? %s' origin/main..HEAD
```

## PRs and CI

- **A pull request is required only for external contributions.** Maintainers
  with push access land work directly on `main` once CI is green. Open a PR
  anyway when a change is risky, wants a second opinion, or needs the CI matrix
  before it lands — but routine maintainer work does not need one.
- Direct-to-`main` work carries the same bar as a PR: signed commits, green CI,
  and the artifacts in [Shipping](knowledge/processes/shipping.md) kept in sync.
- **A change lands on `main` as one commit**, whichever path it took: a PR is
  squash-merged, and direct work is squashed locally before it is pushed. The
  work-in-progress history of a branch — fixups, re-recordings, review
  corrections — is not `main`'s history. Squash before signing, since a rewrite
  drops signatures.
- Use `.github/pull_request_template.md`. Center the description on functional
  change and impact, not a code-location walkthrough (the diff shows that). Add a
  Before / After with proof — a recording or a `--dump` capture for visual
  changes — whenever behavior changes.
- PR titles must be Conventional Commits and under 70 characters — a squashed
  commit inherits the title, so it is the commit subject that lands.
- Use **Squash and Merge**.
- GitHub Actions is the CI source of truth.
- Never merge red CI.
- Before merge, prefer rebasing onto latest `origin/main`.

Use `gh` directly for GitHub commands.

## Shipping, maintenance, and releases

- "Ship" means implement, test the changed feature with an automated test that
  exercises it, gather evidence, perform a security review, open a mergeable PR,
  address every review comment, and merge only after CI is green.
- When asked to ship, follow [`.agents/skills/ship/SKILL.md`](.agents/skills/ship/SKILL.md)
  and [`knowledge/processes/shipping.md`](knowledge/processes/shipping.md).
- When asked for maintenance or release readiness, follow
  [`.agents/skills/maintenance/SKILL.md`](.agents/skills/maintenance/SKILL.md)
  and [`knowledge/processes/maintenance.md`](knowledge/processes/maintenance.md).
- When asked to release, cut a version, or publish to crates.io, follow
  [`.agents/skills/release/SKILL.md`](.agents/skills/release/SKILL.md) and
  [`knowledge/processes/release.md`](knowledge/processes/release.md). `tuika` and
  the companion crates are versioned independently and published in dependency
  order.

## Hosts

tuika was extracted from [yolop](https://github.com/everruns/yolop), whose
full-screen renderer is built on it, and yolop remains its largest consumer — but
it depends on tuika from crates.io like any other host and gets no special
treatment. A change that only makes sense for one host does not belong here;
give the host a boundary (a trait, a state type, a callback) instead.
