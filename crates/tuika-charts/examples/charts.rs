//! Adaptive chart gallery. Kitty/iTerm2/Sixel terminals get smooth pixel
//! geometry; other terminals get the same charts as Unicode cell plots.

use std::io;

use tuika::prelude::*;
use tuika::testing::{grid, render};
use tuika_charts::{Chart, Point, Series};

#[derive(Clone, Copy)]
enum ChartKind {
    Line,
    Area,
    Bar,
    Scatter,
    Step,
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
    }
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
        _ => None,
    }
}

fn main() -> io::Result<()> {
    let theme = Theme::default();
    let isolated = selected_chart();
    let mut gallery = ChartGallery { isolated };
    if std::env::args().any(|arg| arg == "--dump") {
        println!(
            "{}",
            grid(&render(gallery_view(isolated).as_ref(), 96, 28, &theme))
        );
        return Ok(());
    }

    Runner::new(RunnerConfig::default()).run_app(&theme, &mut gallery)
}
