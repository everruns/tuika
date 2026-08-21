---
type: Product Specification
title: Product Goal
description: Defines what tuika is for, the default-framework ambition it is positioned around, and the boundary it defends against host-specific and heavyweight concerns.
---

# Product Goal

## Purpose

tuika is a complete terminal-UI toolkit: a flexbox-style layout solver, anchored
overlays, focus and input ownership, a declarative keymap, an alternate-screen
host, a component set, and the cell grid and terminal loop underneath them. A
host should be able to describe a screen declaratively and get correct layout,
focus, and terminal lifecycle without writing a reconciler.

## Positioning

The ambition is to be **the default framework for building terminal
applications in Rust** — the answer a developer reaches for after deciding to
build a TUI, the way a web developer reaches for a framework rather than a
drawing API. Public material leads with that claim: the README states it as a
banner above the fold, and the crates.io description and keywords say
*application framework*, not *widget collection*.

The claim is earned by scope and by adoption evidence, never by overstatement:

- **Scope.** What a TUI application needs beyond drawing — layout, overlays,
  focus, keymap, theming, terminal lifecycle, screen modes, and a component set
  covering the modern expectations (streaming markdown, images, mouse and
  clipboard, native progress) — is in the box.
- **Complete, but not hostile.** tuika owns its stack down to the bytes and has
  no runtime dependency on ratatui, so it is an alternative rather than a layer.
  That is a statement about dependencies, not a claim to be better: ratatui is
  the reason a Rust TUI ecosystem exists, its widgets remain usable through the
  optional `ratatui` feature, and public material says so plainly rather than
  positioning against it.
- **Evidence over adjectives.** Public claims point at runnable examples and the
  [showcases](../../docs/showcases.md) — real applications on tuika — rather
  than asserting popularity the project does not yet have.

## Design goals

1. **Host-agnostic.** tuika knows nothing about the application embedding it. No
   type, feature, or default exists to serve one host.
2. **Composable over configurable.** Behavior is reached by composing views, not
   by accumulating knobs on a component.
3. **Dependency-light.** The published crate depends only on `crossterm`,
   `unicode-segmentation`, `unicode-width`, and `pulldown-cmark` — 32 crates in
   the default graph, of which `crossterm` is 27. Anything heavier belongs behind
   a trait the host implements — and a dependency tuika could *own* in a page of
   code, rather than delegate, is one it should: see
   [Dependency Discipline](../processes/maintenance.md#dependency-discipline).
4. **Interoperable, not exclusive.** Existing ratatui widgets compose through a
   cell-conversion boundary behind the optional `ratatui` feature; adopting tuika
   never means giving up the widgets already written for it.
5. **Testable without a terminal.** Rendering is observable as cells in memory,
   so behavior is asserted hermetically.

## The dependency boundary

tuika owns *presentation*; the host owns *acquisition*. This is the single line
that keeps the crate small, and every capability that crosses it does so through
a trait the host implements:

| Concern | tuika owns | Host owns |
| --- | --- | --- |
| Syntax highlighting | framing, background, gutter, wrapping (`CodeBlock`) | token spans, via `Highlighter` |
| Structured markdown blocks | width-aware placement, indentation, renderer ordering, and kind-specific fallbacks (`Markdown`) | parsing and terminal-native rendering, via `MarkdownBlockRenderer` |
| Images | protocol encoding, cell reservation, alt fallback | decoding bytes to RGBA, via `ImageResolver` |
| Live data | reading shared state at render time | producing it, and requesting redraws |
| Input | translation to tuika events, keymap dispatch | the event source and the command semantics |

The companion crates exist because of this rule: `tuika-codeformatters`
supplies tree-sitter grammars behind `Highlighter`, `tuika-mermaid` supplies
mmdflux parsing and layout behind `MarkdownBlockRenderer`, and `tuika-html`
supplies html5ever behind the same structured-block boundary. They are separately published
rather than optional tuika features so the core dependency tree cannot grow by
accident.

`tuika-html` also shows where the line falls *inside* one capability: the
presentational inline tags need no parser, so they are in the crate, while
block-level HTML needs a tree builder and is therefore a boundary. The test is the
dependency, not the topic.

## Non-goals

- **No reconciler or retained widget tree.** Views are rebuilt each frame;
  the cell-buffer diff is the only reconciliation.
- **No async runtime requirement.** The optional `async` feature adds an
  `AsyncRunner` for hosts already on Tokio; the default build has no runtime.
- **No data sources.** tuika neither spawns tasks nor performs I/O beyond the
  terminal.
- **No re-implementation of ratatui widgets.** tuika owns the cell grid, not a
  widget catalogue. Where ratatui has a widget tuika lacks, wrap it through the
  `ratatui` feature rather than cloning it.
- **No configuration format.** Themes, stylesheets, and keymaps are code-defined
  values; parsing a user's config file is the host's job.

## Versioning posture

tuika is pre-1.0 but published: every `pub` item is public API. Minor releases
may make deliberate breaking changes, which must be called out in the changelog;
patch releases may not. See [release.md](../processes/release.md).

## Public surface

- [`README.md`](../../README.md)
- [`docs/`](../../docs/)
