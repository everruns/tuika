# tuika-charts

Adaptive terminal charts for [tuika](https://github.com/everruns/tuika): define
the data once, then let the terminal choose the best renderer it supports.

| Portable cells | Terminal graphics |
| --- | --- |
| Quadrants, Braille, and solid cells work in every terminal. | Smooth RGBA plots use Kitty, iTerm2, or Sixel when available. |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-cells.png" alt="Line chart rendered with terminal cells" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-graphics.png" alt="Line chart rendered with terminal graphics" width="560"> |

There is no second chart definition and no renderer switch in application code.
Tuika's `Runner` detects graphics support, supplies the image layer, and falls
back to cells when the terminal cannot display images. Titles, data, domains,
colors, clipping, and legends keep the same meaning in both modes.

## Quick start

```rust
use tuika_charts::{Chart, Point, Series};

let chart = Chart::new()
    .title("API traffic")
    .series(Series::area("volume", [
        Point::new(0.0, 10.0),
        Point::new(1.0, 14.0),
        Point::new(2.0, 13.0),
    ]))
    .series(Series::line("requests", [
        Point::new(0.0, 12.0),
        Point::new(1.0, 18.0),
        Point::new(2.0, 15.0),
    ]));
```

Both axes are labelled by default. `Axis::categories` names positions instead of
measuring them, `Axis::format` supplies units, and `Axis::hidden` drops the
labels for a sparkline. `Chart::stack` combines bar and area series,
`Chart::horizontal` gives each category a full row of width for its name, and
`Chart::focus` is the terminal's answer to a hover tooltip:

```rust
use tuika_charts::{Axis, Chart, Point, Series, Stack};

let chart = Chart::new()
    .x_axis(Axis::new().categories(["Jan", "Feb", "Mar"]))
    .stack(Stack::Normal)
    .focus(1.0)
    .series(Series::bar("desktop", [
        Point::new(0.0, 18.0),
        Point::new(1.0, 30.0),
        Point::new(2.0, 23.0),
    ]));
```

`Chart` is a normal tuika `View`. Return it from `Application::view` and run the
application normally. To try the complete gallery (`q` or `Esc` quits):

```bash
cargo run -p tuika-charts --example charts
```

## Every series, both ways

The paired captures below use the same example, data, dimensions, and theme.
Only terminal graphics capability changes.

### Line

Connected samples for trends and time series.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-cells.png" alt="Cell line chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-graphics.png" alt="Graphics line chart" width="560"> |

### Area

A connected series filled down to the plot baseline.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/area-cells.png" alt="Cell area chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/area-graphics.png" alt="Graphics area chart" width="560"> |

### Bar

Vertical bars centered on their numeric x coordinates.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/bar-cells.png" alt="Cell bar chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/bar-graphics.png" alt="Graphics bar chart" width="560"> |

### Scatter

Independent observations: Braille subcells in portable mode and pixel marks in
graphics mode.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/scatter-cells.png" alt="Cell scatter chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/scatter-graphics.png" alt="Graphics scatter chart" width="560"> |

### Step

Horizontal-then-vertical transitions for state and level changes.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/step-cells.png" alt="Cell step chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/step-graphics.png" alt="Graphics step chart" width="560"> |

### Grouped and stacked

Bar series share a category band by default; `Chart::stack` sums them instead.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/grouped-cells.png" alt="Cell grouped bar chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/grouped-graphics.png" alt="Graphics grouped bar chart" width="560"> |

### Horizontal

Categories down the side, where a name gets a whole row of width.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/horizontal-cells.png" alt="Cell horizontal bar chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/horizontal-graphics.png" alt="Graphics horizontal bar chart" width="560"> |

### Donut

A ring with centre text — the one polar shape that stays readable in cells.

| Cells | Graphics |
| --- | --- |
| <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/donut-cells.png" alt="Cell donut chart" width="560"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/donut-graphics.png" alt="Graphics donut chart" width="560"> |

## How adaptation works

| Capability | Rendering path |
| --- | --- |
| Kitty, Ghostty, WezTerm | Raw RGBA through the Kitty graphics protocol |
| iTerm2 | PNG through the iTerm2 inline-image protocol |
| Sixel host configuration | Palette-quantized Sixel image |
| Anything else | Unicode terminal cells |

The graphics path rasterizes plot geometry; the title, legend, and axis tick
labels remain cells, so text stays sharp, theme-consistent, and identical
between the two paths. The portable path uses 2×2 quadrant
subcells for connected geometry and area boundaries, 2×4 Braille subcells for
scatter points, and solid cells for bars.

Direct/custom hosts can opt into graphics with
`RenderCtx::with_image_graphics`. Plain `paint` calls use cells by default,
which keeps snapshots and tests deterministic.

See the full [chart guide](https://github.com/everruns/tuika/blob/main/docs/charts.md)
for domains, fallback behavior, and the intentionally portable grammar.

## Radar charts

Radar series interpret each point's `y` value as a percentage (`0..=100`) and
use x-axis categories as spoke labels:

```rust
let chart = Chart::new()
    .x_axis(Axis::new().categories(["Strength", "Speed", "Skill"]))
    .series(Series::radar(
        "Profile",
        [Point::new(0.0, 80.0), Point::new(1.0, 65.0), Point::new(2.0, 90.0)],
    ));
```

Run `cargo run -p tuika-charts --example radar` for an interactive profile
picker whose list selection updates the chart.
