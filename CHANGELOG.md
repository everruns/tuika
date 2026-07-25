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
| `tuika::term` | everything out-of-band: `clipboard`, `hyperlink`, `progress`, `pointer`, `image`, `capabilities` |
| `tuika::prelude` | the spine and the components in one glob import |

- **New**: `tuika::prelude` — `use tuika::prelude::*;` replaces most import
  blocks outright, which is the intended migration for application code.
- No behavior change: this release moves and renames public items only. Every
  test, snapshot, and benchmark passes unchanged apart from its imports.

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
* docs: add `knowledge/specs/api-surface.md` and a crate-layout section to the README
* fix(docs): resolve a merge conflict committed to `knowledge/log.md`
