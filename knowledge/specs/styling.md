---
type: Product Specification
title: Theming and styling
description: Defines the two-layer look model — themes as color tokens, stylesheets as semantic rules — and why component code never hard-codes a color.
---

# Theming and styling

## Why

A terminal application's look has to be changeable in two different ways, and
conflating them makes both hard. Users want to swap a *palette* — gruvbox,
solarized, a light variant — without any component knowing which palette is
active. Application authors want to change what a *kind of thing* looks like —
"links are green and bold here" — without editing every component that can draw
a link.

One flat set of colors solves the first and not the second. A per-component
style parameter solves the second and not the first, and pushes the same
decision to dozens of call sites.

## What

Two layers, resolved at render time:

- A **`Theme`** is the token layer: a plain `Copy` struct of named colors
  (`background`, `accent`, `border`, the markdown/code palette). It is passed
  through the render context, and **no component hard-codes a color**, so
  swapping the theme handed to `paint` restyles the whole tree at once. Bundled
  palettes are `const Theme` values reachable as structs, by named constructor,
  or by string (`theme_by_name`, for a `--theme` flag or config value);
  `themes::PRESETS` enumerates them for a picker.
- A **`StyleSheet`** is the rule layer: a mapping from a semantic *role*
  (heading, link, inline code, list bullet, a panel's border and fill) onto a
  `StyleBundle` of color plus text attributes. Overriding one role restyles
  every element with that role, including markdown parts and bare URLs.

`StyleSheet::from_theme` reproduces exactly the look components had before
stylesheets existed, so adopting a sheet is a no-op until a role is overridden.
Adoption must stay free; a default sheet that changes appearance would make the
feature a migration rather than an addition.

Generic status roles (`success`, `warning`, `danger`, `info`) are additive
`Theme` helpers derived from the existing code palette. They deliberately do
not add required fields to `Theme`: downstream themes commonly use public
struct literals, so adding fields would turn a semantic addition into a
source-breaking migration.

## Design

### Bundles are partial overlays

A `StyleBundle` contributes only the attributes it sets. A bundle with no color
(the default emphasis rule, which adds italic) leaves the surrounding text's
color alone. This is what makes struct-update syntax the natural authoring form:
name the roles you care about, and everything unnamed keeps tracking the theme.

### One policy per tree

A host installs a single sheet for the whole render rather than passing styles
down through view constructors. Styling is a cross-cutting policy; threading it
through the tree would put a style parameter on every component and make "change
all links" an N-call-site edit — precisely the problem the layer exists to
remove.

### The gallery is the proof

The theme and stylesheet galleries (`docs/themes.md`, `docs/styling.md`) are
recordings of *one shared scene* rendered under each palette and each sheet, so
the comparison is honest rather than a hand-staged screenshot per variant. The
theme list comes from `themes::PRESETS` and the variant list from the styling
example, so a new palette or role cannot be documented into existence without
existing in code. Regenerating them is `scripts/gen-theme-demos.sh` and
`scripts/gen-styling-demos.sh`.

## Constraints

- Palette regressions are caught by pinning themed cells to their `Theme` slot in
  the unit tests: a component that reaches for a literal color, or the wrong
  slot, fails a test rather than merely looking wrong.
- Themes are values, not configuration. Parsing a user's theme file is the
  host's job; tuika only accepts the resulting struct or a bundled name.

## Non-goals

- No cascade, inheritance, or selectors — roles are flat and resolved directly.
- No runtime stylesheet *format*; a sheet is Rust data.
- No per-component style overrides layered on top of the sheet, which would
  reintroduce the scattered-decision problem.

## Public surface

- [`docs/styling.md`](../../docs/styling.md)
- [`docs/themes.md`](../../docs/themes.md)
