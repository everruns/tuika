# Charts

[`tuika-charts`](https://crates.io/crates/tuika-charts) is tuika's lightweight
chart companion. It adapts rendering to the terminal without changing the chart
or its data:

<img src="../crates/tuika-charts/examples/charts.png" alt="API traffic chart rendered with Unicode cells" width="880">

The committed screenshot shows the portable renderer; run the same example in
a graphics-capable terminal to see its plot switch to smooth pixels.

- **Kitty, iTerm2, or Sixel:** smooth rasterized lines and filled bars are sent
  through tuika's graphics layer.
- **Every other terminal:** a Ratatui-inspired Unicode plot draws axes, lines,
  bars, and the legend directly into cells.

The renderer is the adaptive implementation detail. Titles, series, domains,
colors, clipping, and legends have the same meaning in both paths.

## Grammar

The first release intentionally supports only the common portable subset:

- numeric `(x, y)` points;
- line and vertical bar series;
- multiple named series;
- automatic or explicit x/y domains;
- per-series colors;
- an optional title and legend.

Interactions, tooltips, HTML/SVG marks, curves, stacked data, categorical axes,
and renderer-specific configuration are outside the shared grammar. Keeping the
model smaller than either renderer prevents charts from silently losing meaning
when graphics support changes.

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
    .series(Series::bar("errors", [
        Point::new(0.0, 2.0),
        Point::new(1.0, 1.0),
        Point::new(2.0, 4.0),
    ]));
```

`Chart` is a normal `View`. With no graphics configuration it always uses cells,
which also makes tests and `--dump` output deterministic.

## Enable adaptive graphics

Use the same lifecycle as [`Image`](features.md#images): detect support once,
keep one `ImageLayer`, pass both to the chart, then emit and clear the layer after
the cell frame is flushed.

```rust,ignore
let support = ImageSupport::detect();
let layer = ImageLayer::new();

let chart = Chart::new()
    .series(Series::line("requests", points))
    .support(support)
    .in_layer(&layer);

terminal.draw(|frame| paint(/* ... chart ... */))?;
layer.emit(&mut std::io::stdout())?;
layer.clear();
```

A capability without a layer, or a layer without a capability, safely selects
the cell renderer. Non-finite points are ignored; a chart with no finite points
renders `No chart data` rather than panicking.

Run the example with `q`/`Esc` to quit:

```bash
cargo run -p tuika-charts --example charts
cargo run -p tuika-charts --example charts -- --dump
```
