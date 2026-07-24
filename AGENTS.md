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
- tuika is pre-1.0, but it is a **published library**: every `pub` item is API. A
  breaking change is allowed in a minor release, and must be called out in the
  changelog rather than slipped in.
- Keep the dependency set small. Anything heavy — syntax grammars, image
  decoders, HTTP — belongs in the host or in `tuika-codeformatters`, behind a
  trait tuika defines.

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

This is a small Cargo **workspace**: the root package is the `tuika` library, and
`crates/tuika-codeformatters/` is tuika's tree-sitter `Highlighter`
implementation, published separately so tuika core stays grammar-free.
`cargo test`/`clippy` at the root cover both. For touched code:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
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
```

### Benchmarks

Component render benchmarks live in `benches/` (Criterion). When checking a
change for a regression, save a baseline first and compare against it:

```bash
cargo bench --bench markdown -- --save-baseline before
# ...make the change...
cargo bench --bench markdown -- --baseline before
```

Wall-clock numbers are too noisy on shared CI runners to gate on, so CI runs the
Criterion benches only on `main` (and via manual dispatch) and uploads the output
as an artifact; regression-checking is a local, baseline-to-baseline comparison.
New component benches go in the owning crate's `benches/` as another `[[bench]]`
with `harness = false`.

Alongside those, the `*_iai` benches count CPU instructions under
Valgrind/callgrind ([iai-callgrind](https://github.com/iai-callgrind/iai-callgrind)).
Instruction counts are deterministic and machine-independent for a fixed
toolchain + libc, so the numbers are committed to `*/benches/iai-baseline.json`
and the CI `iai` job **fails** on a regression past the baseline's tolerance —
this is a real gate, not an archive. Running them locally needs Valgrind and a
version-matched runner (`cargo install iai-callgrind-runner`):

```bash
rm -rf target/iai
cargo bench -p tuika --bench markdown_iai --bench scroll_iai \
  -p tuika-codeformatters --bench highlight_iai -- --save-summary=json
python3 benches/check_iai.py            # compare to the committed baseline
python3 benches/check_iai.py --update   # bless new counts (commit alongside the change)
```

When a change legitimately shifts counts (renderer change, dependency bump,
toolchain upgrade), regenerate the baseline with `--update` and commit it with
the code — like a snapshot test. To refresh it from CI's exact environment,
run the workflow manually (`workflow_dispatch`) and commit the uploaded
`iai-baseline` artifact.

### Testing

Layout and rendering are tested hermetically by rendering into an in-memory
ratatui `Buffer` and reading cells back — no real terminal. The consumer-facing
subset (`testing::{render, render_sizes, grid}`) is documented in the README;
tuika's own suite covers more:

- **Unit tests** — each module's own `#[cfg(test)] mod tests`: layout math,
  component rendering, interactive state (scroll/select/focus), keymap dispatch,
  compositor, easing, OSC encoders, and palette (every themed cell pinned to its
  `Theme` slot).
- **Cross-module integration** (`src/integration.rs`) — checks that span several
  modules and have no single owner: composing a real tree, and surviving
  tiny/degenerate screens where scroll and overlay resolution interact.
- **Property tests** (`src/proptests.rs`, `proptest`) — solver and overlay
  invariants for *any* input (children stay in bounds, flex fills exactly).
- **Golden snapshots** (`src/snapshots.rs`) — whole screens diffed against
  checked-in glyph grids; refresh with `UPDATE_SNAPSHOTS=1`. The grids are
  LF-only (`.gitattributes`), so a CRLF checkout cannot fail them.
- **Resize / degenerate sizes** — a size sweep from `0×0` up asserts no panic and
  no out-of-clip writes.
- **PTY smoke** (`tests/pty_smoke.rs`) — drives the `gallery` example under a
  pseudo-terminal and asserts the terminal-facing protocol: alternate-screen
  enter/leave, cursor and mouse-capture lifecycle, OSC 9;4 progress, OSC 8
  hyperlinks, truecolor and Braille cells through a reference terminal parser,
  resize survival, and clean exit. It launches a *built* example, so any runner
  that rebuilds the suite elsewhere — CI's coverage step, notably — has to build
  the examples into that same directory.
- **Packaging** (`tests/packaging.rs`) — drives `cargo package --list` so the
  published `.crate` never re-inflates with repo-only files or heavy GIFs.

## Docs layout

- `docs/components.md` — the public component gallery (name, description, demo
  per component). Keep it **presentational only**: no build or regeneration
  instructions belong here (they live in this file).
- `docs/demos/*.gif` — the committed demo recordings referenced by
  `components.md` and, via `raw.githubusercontent.com` URLs, inline on the
  relevant type's rustdoc so they render on docs.rs — each component's `struct`
  doc, plus module-level types like `OverlaySpec`.
- `docs/hero.gif` — the README hero: a recording of a composite gallery screen.
  See [Hero screenshot](#hero-screenshot).

The demo, theme, and styling GIFs are GitHub-only assets: `Cargo.toml`'s
`exclude` keeps them (and the repository machinery — `knowledge/`, `.agents/`,
`.github/`, `scripts/`) out of the published `.crate`, and `tests/packaging.rs`
guards that split. Only `docs/hero.gif` and `docs/demos/image.svg`, which the
crates.io README embeds by relative path, ship in the crate.

## Component demos

One example is the single source of truth for the gallery:
[`examples/demo.rs`](examples/demo.rs). Its `DEMOS` registry declares every
scene (name, blurb, recording size, builder); the CLI, the tape generator, and
the integrity check all read it.

```bash
cargo run --example demo -- list             # list scenes
cargo run --example demo -- spinner          # interactive (q/esc quits)
cargo run --example demo -- spinner --dump   # print one frame as text
cargo run --example demo -- check            # verify the docs assets
```

### Regenerating the GIFs

[`scripts/gen-demos.sh`](scripts/gen-demos.sh) rebuilds every recording. It asks
the example to emit one VHS tape per scene into a temp dir — **tapes are
generated, not committed** — records each, and runs `check`. Requires
[VHS](https://github.com/charmbracelet/vhs) with `ttyd` and `ffmpeg` on `PATH`.

```bash
scripts/gen-demos.sh              # all scenes
scripts/gen-demos.sh spinner tabs # just these
```

Recordings are captured at ~2× pixel density and displayed at `width="880"`
(rustdoc uses its own `max-width`), so they stay crisp on HiDPI screens.

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
   the scene's recorded geometry, so what it prints is what the GIF will show —
   including anything `rows` is too small to fit.
3. Record it: `scripts/gen-demos.sh <name>`.
4. Reference `demos/<name>.gif` in `docs/components.md` and inline on the
   component's `struct` doc (via the `raw.githubusercontent.com/.../main/...`
   URL, so docs.rs resolves it).

## Hero screenshot

The README hero (`docs/hero.gif`) is a VHS recording of a composite "app" scene
that exercises most of the toolkit at once. The scene lives in
[`examples/screenshot.rs`](examples/screenshot.rs) as one `scene()` builder, and
[`scripts/gen-hero.sh`](scripts/gen-hero.sh) records it: it builds the example,
runs its `run` subcommand full-screen under VHS (window bar for chrome, theme
background matched to tuika's), and writes the GIF. Requires the same toolchain
as the demos — VHS with `ttyd` and `ffmpeg`.

```bash
scripts/gen-hero.sh
```

Regenerate it whenever the components' look changes so the hero stays truthful.

The example doubles as an **offline, recorder-free** path: the same `scene()`
also serializes to a crisp animated SVG (block glyphs painted as rects; static
cells shared across frames, only the moving region duplicated). Handy when VHS
isn't available, or for a vector asset.

```bash
cargo run --example screenshot            # write an SVG (default path)
cargo run --example screenshot -- out.svg # custom path
cargo run --example screenshot -- --dump  # print one frame as text
cargo run --example screenshot -- run     # animate it (what VHS records)
```

### The check invariant

`demo -- check` asserts every scene has a non-empty recording, no orphan GIF
lingers, every `demos/<name>.gif` referenced by the gallery markdown
(`components.md`, `features.md`) or a rustdoc embed (component docs plus
module-level docs like `overlay.rs`) maps to a real scene, and no scene is
clipped by its own frame — it re-renders each one with room to spare and fails on
any line the recorded height would cut off (`filling_demo` scenes are exempt). It
runs in the CI MSRV job and at the end of the generator, so gallery drift fails
CI instead of shipping a broken image to docs.rs.

## Image demo

The Images feature in `docs/features.md` (`docs/demos/image.svg`) can't be
recorded with VHS: VHS captures through `ttyd` + `xterm.js`, which doesn't
implement the Kitty graphics protocol, so a recording would only ever show the
text fallback — never the pixels. So, like the offline SVG path for the hero, the
demo is **generated from the real render** by
[`examples/image_demo.rs`](examples/image_demo.rs): the picture is the actual
RGBA `ImageData` the component transmits (embedded as a dependency-free PNG), and
the fallback panel is the exact placeholder string the component paints. Being an
`.svg`, it is outside the `.gif`-based `demo -- check` invariant, so it needs no
`DEMOS` scene.

```bash
cargo run --example image_demo            # writes docs/demos/image.svg
cargo run --example image_demo -- out.svg # custom path
```

## Cross-terminal checks

`.github/workflows/nightly-terminals.yml` runs the `gallery` example inside real
terminal emulators nightly. The in-repo tests prove tuika emits the right bytes;
the nightly checks how emulators *interpret* them. The tmux leg asserts via
[`scripts/assert-gallery.sh`](scripts/assert-gallery.sh); the GUI legs (kitty,
iTerm2, Windows Terminal) capture artifacts best-effort. Because the assertions
read the gallery's on-screen text, its box titles, Braille spinner, and footer
URL are load-bearing — changing them means updating the assert script.

## Git and commits

- Conventional Commits: `type(scope): description`.
- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.
- Use `chore` for updates to `knowledge/`, `AGENTS.md`, or CI metadata.
- Never add Claude/session/AI attribution links in commits, PRs, docs, or code comments.
- Stage files explicitly by name. Avoid broad `git add .` / `git add -A`.

Commit attribution must be a real human user. If git identity is missing or
agent-like, stop and ask before committing.

## PRs and CI

- Use `.github/pull_request_template.md`. Center the description on functional
  change and impact, not a code-location walkthrough (the diff shows that). Add a
  Before / After with proof — a recording or a `--dump` capture for visual
  changes — whenever behavior changes.
- PR titles must be Conventional Commits and under 70 characters.
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
  and [`knowledge/specs/shipping.md`](knowledge/specs/shipping.md).
- When asked for maintenance or release readiness, follow
  [`.agents/skills/maintenance/SKILL.md`](.agents/skills/maintenance/SKILL.md)
  and [`knowledge/specs/maintenance.md`](knowledge/specs/maintenance.md).
- When asked to release, cut a version, or publish to crates.io, follow
  [`.agents/skills/release/SKILL.md`](.agents/skills/release/SKILL.md) and
  [`knowledge/specs/release.md`](knowledge/specs/release.md). `tuika` and
  `tuika-codeformatters` are versioned independently and published in dependency
  order.

## Hosts

tuika was extracted from [yolop](https://github.com/everruns/yolop), whose
full-screen renderer is built on it, and yolop remains its largest consumer — but
it depends on tuika from crates.io like any other host and gets no special
treatment. A change that only makes sense for one host does not belong here;
give the host a seam (a trait, a state type, a callback) instead.
