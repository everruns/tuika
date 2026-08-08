//! Adaptive chart gallery. Kitty/iTerm2/Sixel terminals get smooth pixel
//! geometry; other terminals get the same charts as Unicode cell plots.

use std::io::{self, Write};
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use tuika::prelude::*;
use tuika::term::image::{ImageLayer, ImageSupport};
use tuika::testing::{grid, render};
use tuika_charts::{Chart, Point, Series};

struct ChartGallery {
    support: ImageSupport,
    layer: Option<ImageLayer>,
}

impl ChartGallery {
    fn new(support: ImageSupport, layer: Option<&ImageLayer>) -> Self {
        Self {
            support,
            layer: layer.cloned(),
        }
    }

    fn configure(&self, chart: Chart) -> Chart {
        let chart = chart.support(self.support);
        match &self.layer {
            Some(layer) => chart.in_layer(layer),
            None => chart,
        }
    }

    fn charts(&self) -> [Chart; 4] {
        [
            self.configure(
                Chart::new()
                    .title("Requests · line + area")
                    .series(Series::area(
                        "baseline",
                        points(&[9., 12., 11., 15., 18., 17., 21.]),
                    ))
                    .series(Series::line(
                        "requests",
                        points(&[12., 18., 16., 25., 33., 31., 42.]),
                    )),
            ),
            self.configure(
                Chart::new()
                    .title("Errors · bars")
                    .series(Series::bar("errors", points(&[2., 1., 4., 2., 5., 3., 2.]))),
            ),
            self.configure(
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
                    )),
            ),
            self.configure(
                Chart::new()
                    .title("Deploy state · step")
                    .series(Series::step(
                        "replicas",
                        points(&[2., 2., 4., 4., 3., 5., 5.]),
                    )),
            ),
        ]
    }
}

impl View for ChartGallery {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        available
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let left_width = area.width / 2;
        let top_height = area.height / 2;
        let areas = [
            Rect::new(area.x, area.y, left_width, top_height),
            Rect::new(
                area.x + left_width,
                area.y,
                area.width - left_width,
                top_height,
            ),
            Rect::new(
                area.x,
                area.y + top_height,
                left_width,
                area.height - top_height,
            ),
            Rect::new(
                area.x + left_width,
                area.y + top_height,
                area.width - left_width,
                area.height - top_height,
            ),
        ];
        for (chart, chart_area) in self.charts().into_iter().zip(areas) {
            chart.render(chart_area, &mut surface.child(chart_area), ctx);
        }
    }
}

fn points(values: &[f64]) -> impl Iterator<Item = Point> + '_ {
    values
        .iter()
        .enumerate()
        .map(|(x, &y)| Point::new(x as f64, y))
}

fn main() -> io::Result<()> {
    let theme = Theme::default();
    if std::env::args().any(|arg| arg == "--dump") {
        println!(
            "{}",
            grid(&render(
                &ChartGallery::new(ImageSupport::None, None),
                96,
                28,
                &theme
            ))
        );
        return Ok(());
    }

    let support = ImageSupport::detect();
    let layer = ImageLayer::new();
    let _session = TerminalSession::enter()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            paint(
                frame.buffer_mut(),
                area,
                &theme,
                &ChartGallery::new(support, Some(&layer)),
                &[],
            );
        })?;
        layer.emit(&mut io::stdout())?;
        io::stdout().flush()?;
        layer.clear();
        if event::poll(Duration::from_millis(100))?
            && let Some(Event::Key(key)) = translate_event(event::read()?)
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break;
        }
    }
    Ok(())
}
