//! A latency dashboard in the style of a matplotlib figure: percentile lines
//! over a shaded p50–p95 band, a dashed SLO threshold, an incident marker,
//! grid lines and axis titles, beside per-service p95 bars. Made-up data.
//!
//! ```bash
//! cargo run -p tuika-charts --example latency_dashboard
//! cargo run -p tuika-charts --example latency_dashboard -- --dump
//! ```

use std::io;

use tuika::prelude::*;
use tuika_charts::{Axis, Chart, Domain, Point, Reference, Series, Stack};

// A Catppuccin Macchiato-like palette.
const BASE: Color = Color::Rgb(36, 39, 58);
const SURFACE: Color = Color::Rgb(54, 58, 79);
const TEXT: Color = Color::Rgb(202, 211, 245);
const SUBTEXT: Color = Color::Rgb(165, 173, 203);
const OVERLAY: Color = Color::Rgb(110, 115, 141);
const BLUE: Color = Color::Rgb(138, 173, 244);
const MAUVE: Color = Color::Rgb(198, 160, 246);
const PEACH: Color = Color::Rgb(245, 169, 127);
const RED: Color = Color::Rgb(237, 135, 150);
const GREEN: Color = Color::Rgb(166, 218, 149);
const YELLOW: Color = Color::Rgb(238, 212, 159);
// Mauve at ~18% over the base: the band between p50 and p95.
const BAND: Color = Color::Rgb(62, 59, 90);

fn theme() -> Theme {
    let mut theme = tuika::themes::DRACULA;
    theme.background = BASE;
    theme.surface = SURFACE;
    theme.text = TEXT;
    theme.muted = SUBTEXT;
    theme.dim = OVERLAY;
    theme.accent = TEXT;
    theme.accent_alt = MAUVE;
    theme.border = OVERLAY;
    theme.border_focused = YELLOW;
    theme
}

const P50: [f64; 31] = [
    42., 46., 45., 46., 49., 49., 48., 49., 49., 48., 47., 46., 45., 43., 42., 41., 40., 37., 40.,
    39., 40., 40., 38., 41., 43., 44., 44., 45., 46., 48., 50.,
];
const P95: [f64; 31] = [
    82., 91., 94., 91., 95., 96., 97., 98., 103., 97., 97., 96., 88., 83., 84., 83., 82., 72., 84.,
    74., 80., 80., 80., 85., 88., 86., 89., 90., 94., 97., 100.,
];
const P99: [f64; 31] = [
    122., 137., 141., 132., 147., 142., 148., 147., 155., 145., 145., 147., 138., 124., 129., 120.,
    121., 106., 205., 106., 117., 121., 124., 123., 135., 123., 132., 130., 137., 140., 155.,
];
const DAY_MARK: f64 = 19.0;
const SLO: f64 = 250.0;

fn days(values: &[f64]) -> Vec<Point> {
    values
        .iter()
        .enumerate()
        .map(|(i, &y)| Point::new(i as f64 + 1.0, y))
        .collect()
}

fn latency_chart() -> Chart {
    Chart::new()
        .title("Request latency, October")
        .x_domain(Domain::new(0.0, 32.0).unwrap())
        .y_domain(Domain::new(0.0, 300.0).unwrap())
        .y_axis(Axis::new().ticks(6).grid().title("ms"))
        .x_axis(Axis::new().ticks(8).grid().format(|v| {
            if v <= 1.0 {
                "Oct".to_string()
            } else {
                format!("{v:02.0}")
            }
        }))
        .reference(Reference::value(SLO).label("SLO").color(RED).dashed())
        .reference(
            Reference::position(DAY_MARK)
                .label("incident")
                .color(YELLOW),
        )
        .series(
            Series::band("p50–p95", days(&P50), days(&P95))
                .color(BAND)
                .legend(false),
        )
        .series(Series::line("p50", days(&P50)).color(BLUE))
        .series(Series::line("p95", days(&P95)).color(MAUVE))
        .series(Series::line("p99", days(&P99)).color(PEACH))
}

// Bottom-to-top, like matplotlib's barh: the first category is the lowest row.
const SERVICES: [(&str, f64, Color); 5] = [
    ("queue", 88., PEACH),
    ("cache", 12., GREEN),
    ("db", 210., RED),
    ("auth", 64., MAUVE),
    ("api", 118., BLUE),
];

fn service_chart() -> Chart {
    // One series per bar so each service gets its own color; stacking keeps
    // them from splitting the row into five grouped slots.
    let mut chart = Chart::new()
        .title("By service")
        .legend(false)
        .horizontal()
        .stack(Stack::Normal)
        .y_domain(Domain::new(0.0, 230.0).unwrap())
        .x_axis(Axis::new().categories(SERVICES.iter().map(|s| s.0)))
        .y_axis(Axis::new().grid().title("p95 ms"));
    for (i, (name, value, color)) in SERVICES.iter().enumerate() {
        chart = chart.series(
            Series::bar(*name, [Point::new(i as f64, *value)])
                .color(*color)
                .labels(),
        );
    }
    chart
}

fn dashboard() -> Element {
    let bg = Style::default().bg(BASE);
    view! {
        col(padding = Padding::all(1), gap = 0, background = bg) {
            grow(1) {
                row(gap = 4) {
                    grow(3) { node(latency_chart()) }
                    grow(2) { node(service_chart()) }
                }
            }
        }
    }
}

struct App;

impl Application for App {
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
        dashboard()
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--dump") {
        let theme = theme();
        let buffer = tuika::testing::render(dashboard().as_ref(), 132, 34, &theme);
        println!("{}", tuika::testing::grid(&buffer));
        return Ok(());
    }
    Runner::new(RunnerConfig::default()).run(&theme(), &mut App)
}
