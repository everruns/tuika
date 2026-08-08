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
The companion exposes one renderer-independent numeric grammar: line and
vertical-bar series, finite `(x, y)` points, automatic or explicit domains,
series colors, title, and legend.

A chart selects smooth RGBA rasterization only when the host provides both
terminal graphics support and an `ImageLayer`. It uses core's image machinery
for placement and Kitty/iTerm2/Sixel lifecycle. Otherwise it draws the same
model into terminal cells with Unicode axes and marks. Supplying only one half
of the graphics configuration falls back to cells.

The title and legend remain cells even in graphics mode, preserving readable
text and consistent semantics while only plot geometry becomes pixel-dense.
Non-finite points are ignored; no finite data renders a stable empty state.


The companion owns a committed portable-renderer screenshot beside its example.
Its README embeds the relative asset so it ships in the crate; the public chart
guide and `Chart` rustdoc reuse it. `scripts/gen-chart-demo.sh` records the real
example, and the repository-wide demo generator includes that script.

## Why

Charts need more rendering machinery than core should carry, but applications
must not author a graphics chart and a separate fallback. Restricting the public
grammar to features both renderers can preserve makes capability changes an
adaptive quality difference rather than a semantic one. Reusing the image
protocol abstraction avoids a second terminal capability detector or command
lifecycle.

## Non-goals

Categorical axes, interactions, tooltips, custom marks, stacking, curves,
HTML/SVG configuration, and renderer-specific escape hatches are not part of
the portable grammar. They require a future design that preserves parity rather
than silently degrading one renderer.
