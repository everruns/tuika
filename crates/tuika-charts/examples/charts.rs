//! Adaptive chart demo. Kitty/iTerm2/Sixel terminals get smooth pixel geometry;
//! other terminals get the same chart as a Unicode cell plot.

use std::io::{self, Write};
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event;
use ratatui::{Terminal, TerminalOptions, Viewport};
use tuika::prelude::*;
use tuika::term::image::{ImageLayer, ImageSupport};
use tuika::testing::{grid, render};
use tuika_charts::{Chart, Point, Series};

fn chart(support: ImageSupport, layer: Option<&ImageLayer>) -> Chart {
    let chart = Chart::new()
        .title("API traffic")
        .series(Series::line(
            "requests",
            [12.0, 18.0, 16.0, 25.0, 33.0, 31.0, 42.0]
                .into_iter()
                .enumerate()
                .map(|(x, y)| Point::new(x as f64, y)),
        ))
        .series(Series::bar(
            "errors",
            [2.0, 1.0, 4.0, 2.0, 5.0, 3.0, 2.0]
                .into_iter()
                .enumerate()
                .map(|(x, y)| Point::new(x as f64, y)),
        ))
        .support(support);
    match layer {
        Some(layer) => chart.in_layer(layer),
        None => chart,
    }
}

fn main() -> io::Result<()> {
    let theme = Theme::default();
    if std::env::args().any(|arg| arg == "--dump") {
        println!(
            "{}",
            grid(&render(&chart(ImageSupport::None, None), 72, 22, &theme))
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
                &chart(support, Some(&layer)),
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
