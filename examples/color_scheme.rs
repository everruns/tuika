//! Follow the OS dark/light preference while the app runs. Run with
//! `cargo run --example color_scheme` (`q`/`esc` quits).
//!
//! The runner subscribes to the terminal's color-scheme reports (mode 2031 —
//! kitty, Ghostty, Contour, foot, recent WezTerm) and repaints with the light
//! or dark theme as the desktop flips. Toggle your OS appearance and watch the
//! screen follow. `l` and `d` set the scheme by hand, which is the same switch a
//! host can make itself; every other key is echoed, to show typing keeps working
//! while the reports arrive in the same input stream.

use std::io;

use tuika::live::Live;
use tuika::prelude::*;
use tuika::term::color_scheme::ColorScheme;
use tuika::themes;

struct State {
    scheme: Live<Option<ColorScheme>>,
    last_key: String,
}

fn main() -> io::Result<()> {
    let runner = Runner::new(RunnerConfig::default())
        .follow_color_scheme(themes::SOLARIZED_LIGHT, themes::SOLARIZED_DARK);
    let mut state = State {
        scheme: runner.color_scheme(),
        last_key: "—".into(),
    };

    // Painted until the terminal answers; one that never does keeps it.
    runner.run(
        &themes::SOLARIZED_DARK,
        from_fn(
            &mut state,
            |state, _frame| {
                let current = state.scheme.with(|scheme| *scheme);
                let (scheme, theme) = match current {
                    Some(ColorScheme::Dark) => ("dark", &themes::SOLARIZED_DARK),
                    Some(ColorScheme::Light) => ("light", &themes::SOLARIZED_LIGHT),
                    None => ("not reported yet", &themes::SOLARIZED_DARK),
                };
                // Plain text keeps the terminal's own foreground, which flips
                // with the OS on a terminal that reports the scheme; naming the
                // theme's text color keeps it readable on one that does not.
                let line =
                    |s: String| Text::new(vec![Line::styled(s, Style::default().fg(theme.text))]);
                let report = line(format!("Color scheme: {scheme}"));
                let typed = line(format!("Last key: {}", state.last_key));
                let hints = line("l light · d dark · q quit".into());
                view! {
                    col(padding = Padding::all(1), gap = 1) {
                        fixed(5) {
                            boxed(title = " color scheme ") {
                                col {
                                    fixed(1) { node(report) }
                                    fixed(1) { node(typed) }
                                }
                            }
                        }
                        grow(1) { spacer() }
                        fixed(1) { node(hints) }
                    }
                }
            },
            |state, signal| match signal {
                Signal::Event(Event::Key(key)) if key.plain() => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => UpdateResult::Exit,
                    KeyCode::Char('l') => {
                        state.scheme.set(Some(ColorScheme::Light));
                        UpdateResult::Dirty
                    }
                    KeyCode::Char('d') => {
                        state.scheme.set(Some(ColorScheme::Dark));
                        UpdateResult::Dirty
                    }
                    KeyCode::Char(c) => {
                        state.last_key = c.to_string();
                        UpdateResult::Dirty
                    }
                    _ => UpdateResult::Clean,
                },
                _ => UpdateResult::Clean,
            },
        ),
    )
}
