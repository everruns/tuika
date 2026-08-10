# tuika-charts

Adaptive terminal charts for [tuika](https://github.com/everruns/tuika).

![Portable chart renderer](https://raw.githubusercontent.com/everruns/tuika/main/docs/charts.png)
One chart model renders as smooth pixels through Kitty, iTerm2, or Sixel when
available, and as a dense Unicode cell chart everywhere else. Lines, steps, and
area edges use solid 2×2 quadrant glyphs; scatter points use Braille subcells;
bars use solid cell marks; and area fills use the same quadrant grid so they stop
at the exact subcell boundary without gaps or overshoot. Both paths support the
same deliberately small grammar: numeric x/y data; line, bar, area, scatter, and
step series; fixed or automatic domains; colors; title; and legend.

```rust
use tuika_charts::{Chart, Point, Series};

let chart = Chart::new()
    .title("Requests")
    .series(Series::area("baseline", [
        Point::new(0.0, 10.0),
        Point::new(1.0, 14.0),
        Point::new(2.0, 13.0),
    ]))
    .series(Series::line("api", [
        Point::new(0.0, 12.0),
        Point::new(1.0, 18.0),
        Point::new(2.0, 15.0),
    ]));
```

Tuika's `Runner` detects graphics support and manages the per-frame image layer,
so `Chart` adapts without renderer setup. Direct/custom hosts retain the explicit
`RenderCtx::with_image_graphics` seam. See the
[chart guide](https://github.com/everruns/tuika/blob/main/docs/charts.md) and runnable example:

```bash
cargo run -p tuika-charts --example charts
```

The data model borrows the useful common denominator from declarative chart
grammars such as TanStack Charts, while intentionally excluding interactions,
HTML layout, custom marks, and renderer-specific options that cannot behave the
same way in both terminal renderers.
