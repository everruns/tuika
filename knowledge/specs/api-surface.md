---
type: Architecture Specification
title: Public API surface
description: Defines what the crate root, components, term, and the prelude each own, and the rules that decide where a new public item goes.
---

# Public API surface

## Why

tuika is a published library, so its module tree is API: every path a host types
is a compatibility promise. Before this policy existed the tree had grown by
accretion — thirty public modules as flat peers, plus 167 names re-exported to
the crate root — so almost every type had two equally valid paths (`tuika::Flex`
and `tuika::components::Flex`) and neither was canonical.

That is not a cosmetic problem. A flat root forces names to disambiguate
themselves by hand (`ASCII_FONT_HEIGHT`, `CONSOLE_DEFAULT_CAPACITY`,
`qr_encode`), lets unrelated concepts collide across namespaces (a `highlight`
module and a `highlight` function both at `tuika::`), and gives a reader no way
to guess where anything lives. The fix is to decide what each level is *for*.

## What

Four levels, each with one job:

| Level | Owns | Test for belonging |
| --- | --- | --- |
| crate root | the framework spine: `View`/`Element`/`ScopedElement`/`RenderCtx`, `Application`, owned or scoped scene composition, layout and responsive dock geometry, events, `Theme`/`StyleSheet`/`StyleRole`/`StyleResolver`, `Surface`, host seam, runners | a host touches it on essentially every frame |
| `components` | every widget | it implements `View` |
| `term` | escapes outside the cell grid: clipboard, hyperlink, progress, pointer, image, capabilities | it talks to the terminal, not to the buffer |
| `screen` | which part of the terminal a frame owns, and publishing above a split footer | it decides or manipulates the reserved region |
| `prelude` | the spine plus all components, in one glob | an application wants it without thinking |

Everything else stays behind its own module path: `themes::by_name`,
`probe::RectProbe`, `width::str_cols`, `framebuffer::FrameBuffer`,
`view::DrawView`, `mouse::paint_selection`. That is not a demotion — it is the
point. A short path is a claim about frequency, so it is worth something only
when it is true.

## Design

### One canonical path per item

A type has exactly one place it lives and one path that reaches it. The prelude
is the deliberate exception, because a glob import is understood to be an
ergonomic alias rather than a second address.

This is why components are re-exported flat from `components` rather than
through per-component modules: `components::Scroll` is the path to write. Where
a component's module is public (see below) the type is technically reachable
through it as well, and rustdoc renders it there — that is a rendering detail,
not a second address.

### A module goes public only when the flat namespace fails it

Inside `components`, a component's own module is private by default. It becomes
public exactly when it owns something a flat namespace cannot name well — a
constant or a free function whose meaning depends on which component it belongs
to. `toast::DEFAULT_TTL`, `markdown::to_lines`, `diff::rows`, `qr::encode`, and
`text::wrap_lines` earn their module; a component that owns only its own type
does not.

The alternative — flattening everything and hand-prefixing the collisions —
produces names like `TOAST_DEFAULT_TTL`, where the prefix is doing the work a
module path would do for free.

### A startup decision can still earn the root

`ScreenMode` and `Scrollback` sit at the crate root even though a host names the
first once, at startup: the mode is a field of `RunnerConfig`, which is already
there, and `TerminalSession` — also a once-per-run type — set the precedent. The
rest of `screen` stays behind its module path, because `pin_footer`,
`close_footer`, and `publish_block` are for hosts that drive their own loop, and
an explicit `screen::` documents that call better than a bare name would.

### Feature-gating is not a reason to split a module

`Runner` and `AsyncRunner` are one concept — a run loop — differing only in which
runtime the host already has. They share `runner`, with the async half behind
`#[cfg(feature = "async")]`. A `cfg` in the tree is an implementation detail; it
should not show up as a second entry in the module list a reader scans.

### Test scaffolding is not API

Unit tests live beside the code they cover. Everything with no single owning
module — cross-module integration, property tests, golden snapshots, shared
helpers — lives under `src/tests/`, compiled only under `#[cfg(test)]`, so the
crate root reads as public surface and nothing else. `testing` is the separate,
genuinely public helper module for *consumers*; see
[testing.md](../processes/testing.md).

## Constraints

- Every `pub` item is API. Moving or renaming one is a breaking change, allowed
  in a minor release pre-1.0, and must be called out in `CHANGELOG.md` with a
  before/after migration line — never slipped in.
- `ui` re-exports the narrow backend value vocabulary custom `View`
  implementations must name (`Rect`, colors/styles, and text lines/spans), so a
  host need not add a direct backend dependency merely to implement tuika's
  trait. Backend operations and widgets remain private.
- `View::measure` and `View::render` receive the same `RenderCtx`. Any public
  helper that measures a component tree (`Flex::solve`, item-height helpers,
  split-footer publication) must require or already own that context; it may not
  synthesize default styling behind the host's back.
- The prelude may grow, but adding to it is a judgement about frequency, not a
  convenience: a name that lands there is one a host should not have to think
  about. Terminal escapes, pixel canvases, probes, and width measurement are
  deliberately outside it.
- A glob prelude can collide with another crate's names (ratatui's `Text` and
  tuika's `Text` are the obvious pair). The prelude documents this rather than
  renaming around it, because the fix — import the two by path — is local to the
  host.

## Related

- [architecture.md](./architecture.md)
- [out-of-band.md](./out-of-band.md)
- [documentation.md](./documentation.md)
