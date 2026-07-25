# Changelog

Notable changes to `tuika`. This file starts with the entry below; releases
before it are described in their [GitHub
Releases](https://github.com/everruns/tuika/releases).

## [Unreleased]

### Highlights

**The crate root is now a decision instead of an accumulation.** `tuika::` had
grown to 30 flat public modules plus 167 names re-exported to the root, so
almost every type had two equally valid paths and neither was canonical. Four
levels now each have one job, and where a new item goes is a rule rather than a
preference:

| Path | Holds |
| --- | --- |
| `tuika::` | the framework spine — `View`, `Element`, `RenderCtx`, layout, events, `Theme`, `Surface`, the host seam |
| `tuika::components` | every widget |
| `tuika::term` | everything out-of-band: `clipboard`, `hyperlink`, `progress`, `pointer`, `image`, `capabilities`, `palette` |
| `tuika::prelude` | the spine and the components in one glob import |

- **New**: `tuika::prelude` — `use tuika::prelude::*;` replaces most import
  blocks outright, which is the intended migration for application code.
- No behavior change: this release moves and renames public items only. Every
  test, snapshot, and benchmark passes unchanged apart from its imports, and
  `tests/public_api.rs` now pins the layout from outside the crate, the way a
  host sees it.

**A theme can be inherited from the terminal.** An application can adopt the
palette the user already configured instead of imposing its own — opt-in and
host-initiated, so nothing changes for an app that does not ask.

- **New**: `themes::TERMINAL` (also `Theme::terminal()`, and a `terminal` entry
  in `themes::PRESETS`) — a `const Theme` whose every slot is `Color::Reset` or a
  `Color::Indexed` ANSI slot, so the terminal resolves the palette. No query, no
  timeout, no failure mode.
- **New**: `tuika::term::palette` — `TerminalPalette` with `parse`/`query`, plus
  `QUERY_FOREGROUND`, `QUERY_BACKGROUND`, and `query_sequence()`. Asks the
  terminal for its colors with the xterm queries (OSC 10 / 11 / 4), fenced by the
  Device Attributes request so an unsupported query costs a round-trip rather
  than a timeout.
- **New**: `Theme::from_terminal(&TerminalPalette)` derives a full theme from the
  reply — reported foreground and background verbatim, in-between tones blended
  and contrast-guarded, hues from the ANSI palette.
- **New**: `Capabilities::query_with_palette(timeout)` answers "what can this
  terminal do" and "what colors is it using" in one round-trip.
- **New example**: `cargo run --example inherit` (and `-- --probe` to print what
  your terminal answers without taking over the screen).

Additive only — nothing moved or was renamed by this change.

### Breaking Changes

Most application code migrates by replacing its `use tuika::{…};` block with
`use tuika::prelude::*;`. The tables below cover what the prelude does not
carry.

**Modules moved**

- **Out-of-band escapes are one family**: they shared a shape but had three
  unrelated names, so a reader had to learn each separately.
  - Before: `tuika::clipboard`, `tuika::hyperlink`, `tuika::native`, `tuika::capabilities`
  - After: `tuika::term::{clipboard, hyperlink, progress, pointer, capabilities}`
- **Images split along the cell boundary**: the protocol half talks to the
  terminal, the view half is a component like any other.
  - Before: `tuika::image::{Image, ImageData, ImageLayer, ImageSupport}`
  - After: `tuika::components::Image` and `tuika::term::image::{ImageData, ImageLayer, ImageSupport}`
- **Markdown is a component**, and lives where the other components live.
  - Before: `tuika::markdown`
  - After: `tuika::components::markdown`
- **One runner module**: a `cfg` is an implementation detail, not a second entry
  in the module list.
  - Before: `tuika::async_runner::{AsyncRunner, Signal}`
  - After: `tuika::runner::{Runner, RunnerConfig, AsyncRunner, Signal}`
- **Ratatui interop is named for the seam, not for its only type.**
  - Before: `tuika::ratatui_view::RatatuiView`
  - After: `tuika::interop::RatatuiView`

**Items renamed**

- **The OSC encoders share one shape** — a pure `encode`, a thin writer.
  - Before: `osc52`, `write_clipboard`, `osc8`, `osc8_with`, `encode_pointer_shape`, `write_pointer_shape`
  - After: `term::clipboard::{encode, write}`, `term::hyperlink::{encode, encode_with}`, `term::pointer::{encode, write}`
- **`tuika::highlight` was a module and a function at once.** The module is the
  `Highlighter` seam; the function paints a selection.
  - Before: `tuika::highlight(buffer, area, range, style)`
  - After: `tuika::mouse::paint_selection(buffer, area, range, style)`
- **Hand-prefixed names get their module back.** The prefixes existed only to
  disambiguate a flat root.
  - Before: `markdown_to_lines`, `markdown_to_linked_lines`, `diff_rows`, `qr_encode`, `wrap_lines`
  - After: `components::markdown::{to_lines, to_linked_lines}`, `components::diff::rows`, `components::qr::encode`, `components::text::wrap_lines`
  - Before: `ASCII_FONT_HEIGHT`, `CONSOLE_DEFAULT_CAPACITY`, `TOAST_DEFAULT_TTL`
  - After: `components::ascii_font::FONT_HEIGHT`, `components::console::DEFAULT_CAPACITY`, `components::toast::DEFAULT_TTL`
- **`Overlay` sits beside `OverlaySpec`**, since resolving a spec to a rect and
  pairing that rect with a view are two halves of one pipeline.
  - Before: `tuika::host::Overlay`
  - After: `tuika::overlay::Overlay` (still re-exported as `tuika::Overlay`)

**Root re-exports removed**

Components and the per-module surface (`anim`, `focus`, `framebuffer`,
`highlight`, `keymap`, `live`, `mouse`, `probe`, `themes`, `width`, and the
styling extras) are no longer flattened to `tuika::`. Reach them through
`tuika::prelude::*` or their module path.

### What's Changed

* refactor: give the crate root, components, and term one job each
* refactor(term): group the out-of-band escapes under one module
* refactor(components): move markdown and the image view in with the components
* refactor: fold `async_runner` into `runner` and rename `ratatui_view` to `interop`
* refactor(tests): move the crate's test scaffolding under `src/tests`
* refactor(markdown): split the module along its parse/flatten passes
* docs(example): follow the stream in the markdown example until the reader scrolls back
* test: pin the public module layout from outside the crate (`tests/public_api.rs`)
* docs: add `knowledge/specs/api-surface.md` and a crate-layout section to the README
* feat(themes): inherit the terminal's palette
