---
type: Product Specification
title: Product Goal
description: Defines what tuika is for and the boundary it defends against host-specific and heavyweight concerns.
---

# Product Goal

## Purpose

tuika supplies the pieces `ratatui` deliberately leaves to the application — a
flexbox-style layout solver, anchored overlays, focus and input ownership, a
declarative keymap, an alternate-screen host, and a component set — while
leaving `ratatui` in charge of the cell buffer and its diff against the
terminal. A host should be able to describe a screen declaratively and get
correct layout, focus, and terminal lifecycle without writing a reconciler.

## Design goals

1. **Host-agnostic.** tuika knows nothing about the application embedding it. No
   type, feature, or default exists to serve one host.
2. **Composable over configurable.** Behavior is reached by composing views, not
   by accumulating knobs on a component.
3. **Dependency-light.** The published crate depends only on `ratatui-core`,
   `ratatui-crossterm`, `crossterm`, `textwrap`, `unicode-segmentation`,
   `unicode-width`, and `pulldown-cmark`. Anything heavier belongs behind a
   trait the host implements.
4. **Interoperable, not exclusive.** Existing ratatui widgets compose through a
   raw-`Buffer` seam; adopting tuika never means giving up ratatui.
5. **Testable without a terminal.** Rendering is observable as cells in memory,
   so behavior is asserted hermetically.

## The dependency boundary

tuika owns *presentation*; the host owns *acquisition*. This is the single line
that keeps the crate small, and every capability that crosses it does so through
a trait the host implements:

| Concern | tuika owns | Host owns |
| --- | --- | --- |
| Syntax highlighting | framing, background, gutter, wrapping (`CodeBlock`) | token spans, via `Highlighter` |
| Images | protocol encoding, cell reservation, alt fallback | decoding bytes to RGBA, via `ImageResolver` |
| Live data | reading shared state at render time | producing it, and requesting redraws |
| Input | translation to tuika events, keymap dispatch | the event source and the command semantics |

`tuika-codeformatters` exists because of this rule: a ready-made tree-sitter
`Highlighter` is genuinely useful, and the grammar crates it needs are exactly
what tuika core must not carry. It is a separate published crate rather than an
optional feature so tuika's dependency tree cannot grow by accident.

## Non-goals

- **No reconciler or retained widget tree.** Views are rebuilt each frame;
  ratatui's buffer diff is the only reconciliation.
- **No async runtime requirement.** The optional `async` feature adds an
  `AsyncRunner` for hosts already on Tokio; the default build has no runtime.
- **No data sources.** tuika neither spawns tasks nor performs I/O beyond the
  terminal.
- **No re-implementation of ratatui widgets.** Where ratatui has a widget, wrap
  it rather than clone it.
- **No configuration format.** Themes, stylesheets, and keymaps are code-defined
  values; parsing a user's config file is the host's job.

## Versioning posture

tuika is pre-1.0 but published: every `pub` item is public API. Minor releases
may make deliberate breaking changes, which must be called out in the changelog;
patch releases may not. See [release.md](./release.md).

## Public surface

- [`README.md`](../../README.md)
- [`docs/`](../../docs/)
