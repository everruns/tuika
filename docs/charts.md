---
title: Charts
description: Define one terminal chart that adapts from portable Unicode cells to smooth terminal graphics.
sidebar:
  order: 9
---

# Charts

[`tuika-charts`](https://crates.io/crates/tuika-charts) defines one chart model
for two terminal rendering paths. A graphics-capable terminal gets a smooth RGBA
plot; every other terminal gets a dense Unicode plot. Applications do not keep
two chart implementations or ask users to choose a mode.

| Portable cells | Terminal graphics |
| --- | --- |
| <img src="charts/line-cells.png" alt="Line chart rendered with terminal cells" width="560"> | <img src="charts/line-graphics.png" alt="The same line chart rendered with terminal graphics" width="560"> |

## Adaptive by default

`Chart` is a normal `View`. When an application runs through tuika's `Runner`,
the framework detects the terminal and owns the image lifecycle:

- Kitty, Ghostty, and WezTerm receive raw RGBA through the Kitty graphics
  protocol.
- iTerm2 receives a PNG through its inline-image protocol.
- a Sixel-capable custom host can select Sixel explicitly.
- all other environments render directly into terminal cells.

The selection changes fidelity, not meaning. Both paths share the same title,
legend, series, colors, numeric domains, clipping, empty state, and handling of
non-finite points. The graphics plot is an image, while its title and legend
remain cells for crisp, consistent text.

```rust,ignore
let chart = Chart::new()
    .title("Requests")
    .series(Series::line("api", points));

// Return `chart` from Application::view. Runner supplies graphics when the
// terminal supports them and otherwise leaves Chart on its cell path.
Runner::new(RunnerConfig::default())
    .run(&Theme::default(), &mut app)?;
```

Direct calls to `paint` use cells by default, making in-memory tests stable.
Custom terminal hosts can supply `ImageSupport` and `ImageLayer` through
`RenderCtx::with_image_graphics`, then emit and clear that layer after each cell
frame.

## Series gallery

Every pair below is generated from the same runnable example at the same size.
The left capture disables graphics signals; the right capture runs through
Ghostty and exercises the actual Kitty image output.

### Line

Use a line for connected samples and trends.

| Cells | Graphics |
| --- | --- |
| <img src="charts/line-cells.png" alt="Cell line chart" width="560"> | <img src="charts/line-graphics.png" alt="Graphics line chart" width="560"> |

### Area

An area connects samples and fills the space down to the plot baseline. The
cell renderer shares one quadrant boundary between edge and fill, preventing
gaps or fill above the line; graphics mode can keep a bright edge over a dimmer
fill.

| Cells | Graphics |
| --- | --- |
| <img src="charts/area-cells.png" alt="Cell area chart" width="560"> | <img src="charts/area-graphics.png" alt="Graphics area chart" width="560"> |

### Bar

Bars are vertical marks centered on numeric x coordinates. Automatic x domains
reserve half a bar interval beyond the first and last values so edge bars remain
inside the plot.

| Cells | Graphics |
| --- | --- |
| <img src="charts/bar-cells.png" alt="Cell bar chart" width="560"> | <img src="charts/bar-graphics.png" alt="Graphics bar chart" width="560"> |

### Scatter

Scatter series keep samples independent. The portable path uses Braille's 2×4
subcell grid; graphics mode uses pixel marks.

| Cells | Graphics |
| --- | --- |
| <img src="charts/scatter-cells.png" alt="Cell scatter chart" width="560"> | <img src="charts/scatter-graphics.png" alt="Graphics scatter chart" width="560"> |

### Step

Step series draw horizontal-then-vertical transitions, useful for replicas,
states, thresholds, and other values held until the next sample.

| Cells | Graphics |
| --- | --- |
| <img src="charts/step-cells.png" alt="Cell step chart" width="560"> | <img src="charts/step-graphics.png" alt="Graphics step chart" width="560"> |

## Portable grammar

The shared grammar deliberately includes only features both paths can preserve:

- finite numeric `(x, y)` points;
- line, area, bar, scatter, and horizontal-then-vertical step series;
- multiple named and colored series;
- automatic or explicit numeric x/y domains;
- an optional title and legend.

Interactions, tooltips, categorical axes, smooth curves, stacked data,
HTML/SVG marks, and renderer-specific styling are outside this portable model.
That boundary prevents a chart from silently losing meaning when it moves to a
terminal with different capabilities.

```rust
use tuika_charts::{Chart, Domain, Point, Series};

let chart = Chart::new()
    .title("API traffic")
    .x_domain(Domain::new(0.0, 6.0).unwrap())
    .series(Series::line("requests", [
        Point::new(0.0, 12.0),
        Point::new(1.0, 18.0),
        Point::new(2.0, 16.0),
    ]))
    .series(Series::area("volume", [
        Point::new(0.0, 9.0),
        Point::new(1.0, 12.0),
        Point::new(2.0, 11.0),
    ]))
    .series(Series::scatter("errors", [
        Point::new(0.0, 2.0),
        Point::new(1.0, 1.0),
        Point::new(2.0, 4.0),
    ]));
```

Non-finite points are ignored. A chart with no finite data renders `No chart
data`. Explicit domains are used verbatim and therefore control clipping
directly.

Run the adaptive gallery with `q` or `Esc` to quit:

```bash
cargo run -p tuika-charts --example charts
```
