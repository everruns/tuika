//! Adaptive chart gallery. Kitty/iTerm2/Sixel terminals get smooth pixel
//! geometry; other terminals get the same charts as Unicode cell plots.

use std::io;

use tuika::prelude::*;
use tuika::testing::{grid, render};
use tuika_charts::{Axis, Chart, Point, Series, Stack};

mod support;

#[derive(Clone, Copy)]
enum ChartKind {
    Line,
    Area,
    Bar,
    Scatter,
    Step,
    Grouped,
    Stacked,
    Horizontal,
    Donut,
    Focus,
}

struct ChartGallery {
    isolated: Option<ChartKind>,
}

impl Application for ChartGallery {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        match signal {
            Signal::Event(Event::Key(key))
                if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
            {
                UpdateResult::Exit
            }
            _ => UpdateResult::Clean,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        gallery_view(self.isolated)
    }
}

fn gallery_view(isolated: Option<ChartKind>) -> Element {
    if let Some(kind) = isolated {
        return view! {
            col(padding = Padding::all(1)) {
                grow(1) { node(chart(kind)) }
            }
        };
    }

    view! {
        col(padding = Padding::all(1)) {
            grow(1) {
                row {
                    grow(1) { node(requests_chart()) }
                    grow(1) { node(errors_chart()) }
                }
            }
            grow(1) {
                row {
                    grow(1) { node(latency_chart()) }
                    grow(1) { node(deploy_chart()) }
                }
            }
            grow(1) {
                row {
                    grow(1) { node(grouped_chart()) }
                    grow(1) { node(horizontal_chart()) }
                }
            }
        }
    }
}

fn chart(kind: ChartKind) -> Chart {
    match kind {
        ChartKind::Line => line_chart(),
        ChartKind::Area => area_chart(),
        ChartKind::Bar => errors_chart(),
        ChartKind::Scatter => latency_chart(),
        ChartKind::Step => deploy_chart(),
        ChartKind::Grouped => grouped_chart(),
        ChartKind::Stacked => stacked_chart(),
        ChartKind::Horizontal => horizontal_chart(),
        ChartKind::Donut => donut_chart(),
        ChartKind::Focus => focus_chart(),
    }
}

const MONTHS: [&str; 6] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun"];
const DESKTOP: &[f64] = &[18., 30., 23., 27., 20., 34.];
const MOBILE: &[f64] = &[12., 20., 15., 17., 14., 22.];

fn months() -> Axis {
    Axis::new().categories(MONTHS)
}

fn grouped_chart() -> Chart {
    Chart::new()
        .title("Visits · grouped bars")
        .x_axis(months())
        .series(Series::bar("desktop", points(DESKTOP)))
        .series(Series::bar("mobile", points(MOBILE)))
}

fn stacked_chart() -> Chart {
    Chart::new()
        .title("Visits · stacked")
        .x_axis(months())
        .stack(Stack::Normal)
        .series(Series::area("desktop", points(DESKTOP)))
        .series(Series::area("mobile", points(MOBILE)))
}

fn horizontal_chart() -> Chart {
    Chart::new()
        .title("Browsers · horizontal")
        .horizontal()
        .x_axis(Axis::new().categories(["Chrome", "Safari", "Firefox", "Edge", "Other"]))
        .series(Series::bar("share", points(&[42., 26., 15., 10., 7.])).labels())
}

fn donut_chart() -> Chart {
    Chart::new()
        .title("Traffic · donut")
        .center("1.2k")
        .series(Series::donut("desktop", [Point::new(0., 55.)]))
        .series(Series::donut("mobile", [Point::new(0., 30.)]))
        .series(Series::donut("other", [Point::new(0., 15.)]))
}

fn focus_chart() -> Chart {
    Chart::new()
        .title("Visits · focus readout")
        .x_axis(months())
        .focus(3.0)
        .series(Series::line("desktop", points(DESKTOP)).markers())
        .series(Series::line("mobile", points(MOBILE)).markers())
}

fn line_chart() -> Chart {
    Chart::new().title("Requests · line").series(Series::line(
        "requests",
        points(&[12., 18., 16., 25., 33., 31., 42.]),
    ))
}

fn area_chart() -> Chart {
    Chart::new().title("Volume · area").series(Series::area(
        "volume",
        points(&[8., 14., 12., 20., 28., 25., 36.]),
    ))
}

fn requests_chart() -> Chart {
    const REQUESTS: &[f64] = &[12., 18., 16., 25., 33., 31., 42.];
    Chart::new()
        .title("Requests · line + area")
        .series(Series::area("volume", points(REQUESTS)))
        .series(Series::line("requests", points(REQUESTS)))
}

fn errors_chart() -> Chart {
    Chart::new()
        .title("Errors · bars")
        .series(Series::bar("errors", points(&[2., 1., 4., 2., 5., 3., 2.])))
}

fn latency_chart() -> Chart {
    Chart::new()
        .title("Latency · scatter")
        .y_axis(Axis::new().format(|value| format!("{value:.0}ms")))
        .series(Series::scatter(
            "p95",
            [
                Point::new(0.2, 18.0),
                Point::new(0.9, 27.0),
                Point::new(1.8, 22.0),
                Point::new(2.5, 35.0),
                Point::new(3.7, 29.0),
                Point::new(4.4, 41.0),
                Point::new(5.8, 37.0),
            ],
        ))
}

fn deploy_chart() -> Chart {
    Chart::new()
        .title("Deploy state · step")
        .series(Series::step(
            "replicas",
            points(&[2., 2., 4., 4., 3., 5., 5.]),
        ))
}

fn points(values: &[f64]) -> impl Iterator<Item = Point> + '_ {
    values
        .iter()
        .enumerate()
        .map(|(x, &y)| Point::new(x as f64, y))
}

fn selected_chart() -> Option<ChartKind> {
    match std::env::var("TUIKA_CHART_DEMO").ok()?.as_str() {
        "line" => Some(ChartKind::Line),
        "area" => Some(ChartKind::Area),
        "bar" => Some(ChartKind::Bar),
        "scatter" => Some(ChartKind::Scatter),
        "step" => Some(ChartKind::Step),
        "grouped" => Some(ChartKind::Grouped),
        "stacked" => Some(ChartKind::Stacked),
        "horizontal" => Some(ChartKind::Horizontal),
        "donut" => Some(ChartKind::Donut),
        "focus" => Some(ChartKind::Focus),
        _ => None,
    }
}

fn main() -> io::Result<()> {
    let (theme, args) = support::theme_and_args()?;
    let isolated = selected_chart();
    let mut gallery = ChartGallery { isolated };
    if args.iter().any(|arg| arg == "--dump") {
        println!(
            "{}",
            grid(&render(gallery_view(isolated).as_ref(), 96, 42, &theme))
        );
        return Ok(());
    }

    Runner::new(RunnerConfig::default()).run(&theme, &mut gallery)
}
