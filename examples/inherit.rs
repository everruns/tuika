//! Inherit the terminal's own colors. Run with `cargo run --example inherit`
//! (`q`/`esc` quits).
//!
//! Two ways to make a tuika app look like it belongs in the terminal the user
//! configured, and this example shows both plus the fallback between them:
//!
//! - Ask the terminal for its palette ([`Capabilities::query_with_palette`]) and
//!   derive a full theme from the answer ([`Theme::from_terminal`]). Real 24-bit
//!   colors, so the in-between tones — panels, rules, selection bands — can be
//!   blended and contrast-checked.
//! - Fall back to [`themes::TERMINAL`], which needs no query at all: every slot
//!   is an ANSI index or `Reset`, so the terminal resolves it.
//!
//! The probe is the interesting part. It must run **in raw mode, before the
//! event loop reads stdin** — the replies arrive on stdin, and an event loop
//! already running would swallow them as keystrokes. That is the whole reason
//! this example enters raw mode itself, probes, leaves raw mode, and only then
//! starts the runner.
//!
//! `--probe` does just the query and prints what came back, without taking over
//! the screen — the fastest way to see what your terminal actually answers:
//!
//! ```text
//! cargo run --example inherit -- --probe
//! ```

use std::io;
use std::ops::ControlFlow;
use std::time::Duration;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use tuika::components::{Flex, Rule, Text};
use tuika::term::capabilities::Capabilities;
use tuika::{Event, KeyCode, Padding, Runner, RunnerConfig, Theme, themes, view};

/// How long to wait for the terminal to answer. A real tty replies to the
/// Device Attributes fence immediately, so this is only reached on a terminal
/// that ignores the query entirely.
const PROBE_TIMEOUT: Duration = Duration::from_millis(100);

/// Where the running theme came from, so the UI can say so.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The terminal answered; every color below is derived from its palette.
    Probed,
    /// The terminal said nothing, so the ANSI-slot theme inherits implicitly.
    AnsiSlots,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Probed => "probed — derived from the colors this terminal reported",
            Source::AnsiSlots => "themes::TERMINAL — the terminal answered nothing, ANSI slots",
        }
    }
}

/// Enter raw mode, ask the terminal for its palette, and leave raw mode.
///
/// The raw-mode enable and disable are paired here rather than left to the
/// runner: a probe that returns early must not strand the terminal.
fn probe() -> io::Result<(Theme, Source)> {
    crossterm::terminal::enable_raw_mode()?;
    let (_caps, palette) = Capabilities::query_with_palette(PROBE_TIMEOUT);
    crossterm::terminal::disable_raw_mode()?;

    Ok(match Theme::from_terminal(&palette) {
        Some(theme) => (theme, Source::Probed),
        None => (themes::TERMINAL, Source::AnsiSlots),
    })
}

/// `#rrggbb` for a derived color, or the ANSI slot's own name when the theme is
/// the one the terminal resolves for itself.
fn describe(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "terminal default".to_string(),
        Color::Indexed(i) => format!("ansi {i}"),
        other => format!("{other:?}"),
    }
}

/// The theme slots worth showing, in the order the UI lists them.
fn slots(theme: &Theme) -> Vec<(&'static str, Color)> {
    vec![
        ("background", theme.background),
        ("surface", theme.surface),
        ("text", theme.text),
        ("muted", theme.muted),
        ("dim", theme.dim),
        ("accent", theme.accent),
        ("accent_alt", theme.accent_alt),
        ("border", theme.border),
        ("selection_bg", theme.selection_bg),
        ("code.keyword", theme.code.keyword),
        ("code.string", theme.code.string),
        ("code.comment", theme.code.comment),
    ]
}

fn build(theme: &Theme, source: Source) -> tuika::Element {
    // Each row paints its own color, so the swatch is the color itself rather
    // than a description of it.
    let rows: Vec<tuika::Element> = slots(theme)
        .into_iter()
        .map(|(name, color)| {
            let line = Line::from(vec![
                Span::styled("████", Style::default().fg(color)),
                Span::styled(
                    format!(" {name:<13} {}", describe(color)),
                    Style::default().fg(theme.text),
                ),
            ]);
            tuika::element(Text::new(vec![line]))
        })
        .collect();

    let mut palette = Flex::column();
    for row in rows {
        palette = palette.fixed(1, row);
    }

    view! {
        col(padding = Padding::all(1), gap = 1,
            background = Style::default().bg(theme.background)) {
            fixed(3) {
                boxed(title = Line::from(Span::styled(" inherited theme ", theme.accent_style()))) {
                    text(source.label().to_string())
                }
            }
            fixed(14) {
                boxed(title = Line::from(Span::styled(" slots ", theme.accent_style()))) {
                    node(palette)
                }
            }
            fixed(1) { node(Rule::new()) }
            fixed(1) {
                text("q / esc to quit".to_string())
            }
        }
    }
}

fn main() -> io::Result<()> {
    let probe_only = std::env::args().any(|a| a == "--probe");
    let (theme, source) = probe()?;

    if probe_only {
        // Plain text, no alternate screen: usable over a pipe and in a test.
        println!("source: {}", source.label());
        for (name, color) in slots(&theme) {
            println!("{name}: {}", describe(color));
        }
        return Ok(());
    }

    let runner = Runner::new(RunnerConfig::default());
    runner.run(
        &theme,
        |_frame| build(&theme, source),
        |event| match event {
            Event::Key(key) if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) => {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        },
    )
}
