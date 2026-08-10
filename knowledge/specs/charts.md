---
type: Architecture Specification
title: Adaptive charts
description: Defines the portable chart grammar and capability-adaptive rendering contract for tuika-charts.
---

# Adaptive charts

## Status

Accepted.

## Decision

Charts live in the separately published `tuika-charts` companion, not core.
The companion exposes one renderer-independent numeric grammar: line,
vertical-bar, filled-area, scatter, and stepped-line series; finite `(x, y)`
points; automatic or explicit domains; series colors; title; and legend.

A chart selects smooth RGBA rasterization when its `RenderCtx` provides terminal
graphics support and an `ImageLayer`. The real-terminal `Runner` detects and
supplies both automatically; custom hosts retain the explicit context seam. It
uses core's image machinery for placement and Kitty/iTerm2/Sixel lifecycle.
Otherwise it draws the same model into terminal cells with Unicode axes and
marks. The portable renderer uses
dense Unicode quadrant glyphs with a 2×2 subcell grid for connected line, step,
and area-edge geometry, while scatter retains Braille's 2×4 point placement.
Bars remain cell-shaped. Portable area fill and its edge share one quadrant mask
and color; a cell cannot encode a distinct edge, fill, and empty background at
once, while the shared mask preserves the exact boundary without gaps or fill
above it. The graphics renderer retains distinct dim fill and bright edge colors.
Quadrants deliberately trade some vertical resolution for continuous strokes
across terminal fonts; separated Braille dots are reserved for discrete marks.

The title and legend remain cells even in graphics mode, preserving readable
text and consistent semantics while only plot geometry becomes pixel-dense.
Non-finite points are ignored; no finite data renders a stable empty state.
Automatic x domains containing bars extend by half the smallest finite bar
interval on each side, keeping centered edge bars inside either renderer.
Explicit domains remain exact clipping bounds.


The repository owns a committed portable-renderer screenshot at
`docs/charts.png`. The companion README and `Chart` rustdoc embed the
repository-hosted asset, while the public chart guide uses it directly.
`scripts/gen-chart-demo.sh` records the real example, and the repository-wide
demo generator includes that script.

## Why

Charts need more rendering machinery than core should carry, but applications
must not author a graphics chart and a separate fallback. Restricting the public
grammar to features both renderers can preserve makes capability changes an
adaptive quality difference rather than a semantic one. Reusing the image
protocol abstraction avoids a second terminal capability detector or command
lifecycle.

## Non-goals

Categorical axes, interactions, tooltips, custom marks, stacking, smooth curves,
HTML/SVG configuration, and renderer-specific escape hatches are not part of
the portable grammar. They require a future design that preserves parity rather
than silently degrading one renderer.
