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
The companion exposes one renderer-independent grammar: line, bar, filled-area,
scatter, stepped-line, and donut series; finite `(x, y)` points; automatic or
explicit domains; series colors; title; legend; numeric or categorical axis tick
labels; stacking and bar grouping; horizontal orientation; sample markers, value
labels, and a focused-position readout.

Every series resolves to renderer-independent geometry — paths, bands, bar
rectangles, arcs — in data coordinates before either renderer runs. Stacking,
group slotting, percent scaling, and category placement are decided once there.
Computing them inside a renderer instead would mean computing them twice, and
two implementations of the same rule are two chances for the paths to disagree.
Domains are then sized from that geometry, which is why bars and areas reach the
zero baseline without a separate rule: each carries its baseline as its own low
edge.

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

The title, legend, and axis tick labels remain cells even in graphics mode,
preserving readable text and consistent semantics while only plot geometry
becomes pixel-dense. Because the labels sit outside the image, both renderers
plot into the *same* cells: the graphics path carries no internal margin of its
own, or a label would point at a different value than the pixel beside it.

Bars occupy a slice of the band their position owns, and are half-open across
the category axis: a band's gutter can be narrower than a cell, and two bars
rounded to touching columns read as one wide bar, so the far column is dropped
to keep the gap the grouping intended. Horizontal orientation exists because a
terminal gives a category a whole row of width that way, where a vertical bar
leaves it one column. A rule is drawn along zero whenever the value domain
straddles it.

Annotations may never cover the readings they annotate: a value label is tried
in a few positions around its mark and dropped when none is clear. The focused
position is host state rather than chart state, so the chart stays a pure view
and one mechanism serves a keyboard cursor, a replay, or a fixed annotation.

The donut is the only polar shape in the grammar, and only as a ring with
optional centre text: a filled pie's wedge boundaries and radial labels both
degrade badly at cell resolution, while a thick ring does not. Slices are named
by the legend. Both renderers paint the same arcs.

Graphics images are transparent where nothing is drawn rather than filled with
the theme background, because the terminal composites them over the cell grid:
an opaque background would hide the cell-drawn text beneath it, which is what
lets value labels and a donut's centre text stay cells in graphics mode exactly
as tick labels outside the image already do.

Both axes are labelled by default, because an unlabelled plot states a shape
without stating a scale. Ticks are placed on round values — a power of ten times
1, 2, 2.5, or 5 — and label precision is derived from the tick step rather than
its magnitude. Chrome yields to data when space runs short: y labels claim a
gutter but are dropped whole rather than take half the width, x labels reuse the
margin row the plot already reserved, and colliding x labels are thinned left to
right so a narrow chart loses labels instead of legibility. `Axis::hidden`
restores the unlabelled geometry for sparklines and dense dashboards.

Non-finite points are ignored; no finite data renders a stable empty state.
Automatic domains are sized from resolved geometry, so a bar position keeps the
full band it owns and an edge bar keeps its gutter rather than sitting flush
against the frame. Bars and areas therefore reach zero on their own: each
encodes its value in a filled span rather than in the position of its tip, and a
domain starting at the data minimum would draw a bar of 1 beside a bar of 5 as a
sliver beside a full column, stating a ratio the data does not contain. Line,
step, and scatter series are read as positions and keep the tighter domain that
resolves their variation. Explicit domains remain exact clipping bounds.


The repository owns a paired gallery under `docs/charts/`: one portable-cell and
one terminal-graphics screenshot for each supported series kind and layout. The
companion README and `Chart` rustdoc embed the repository-hosted assets, while
the public chart guide uses them directly. `scripts/gen-chart-demo.sh` records
the real example twice at identical dimensions. Its graphics pass builds the
pinned Everruns VHS fork with Ghostty support and applies the small repository
patch that composites Ghostty's Kitty placements into VHS frames. The
repository-wide demo generator includes that script.

## Why

Charts need more rendering machinery than core should carry, but applications
must not author a graphics chart and a separate fallback. Restricting the public
grammar to features both renderers can preserve makes capability changes an
adaptive quality difference rather than a semantic one. Reusing the image
protocol abstraction avoids a second terminal capability detector or command
lifecycle.

## Non-goals

Smooth curves, filled pie and radar geometry, gradients, icon marks, grid lines,
HTML/SVG configuration, and renderer-specific escape hatches are not part of the
portable grammar. Radar is excluded on the same ground the pie is: diagonal webs
and around-the-circle labels look right in graphics and wrong in cells, and a
feature that only survives one renderer is the divergence this contract exists
to prevent. The rest require a future design that preserves parity rather than
silently degrading one renderer.
