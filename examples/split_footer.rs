//! A split-footer screen mode: a live footer over real terminal scrollback.
//!
//! The renderer owns only the bottom rows. Everything above them stays the
//! terminal's own scrollback — the shell prompt that started this example is
//! still up there, the wheel and mouse selection still work, and the published
//! output can be scrolled back to and copied like any other command's.
//!
//! A background worker publishes into that scrollback through a `Scrollback`
//! handle instead of `println!`, which would land wherever the footer left the
//! cursor.
//!
//! ```text
//! cargo run --example split_footer            # the terminal keeps the mouse
//! cargo run --example split_footer -- --mouse # the footer captures it instead
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tuika::prelude::*;

mod support;

/// Rows the footer reserves. Everything above is the terminal's.
const FOOTER_ROWS: u16 = 5;

struct Status {
    published: u64,
    paused: bool,
}

/// The footer: what stays on screen and repaints every frame.
fn footer(status: &Status, frame: u64, theme: &Theme) -> Element {
    let state = if status.paused { "paused" } else { "watching" };
    let head = Line::from(vec![
        Span::styled(
            format!("{} ", Spinner::new(frame).glyph()),
            Style::default().fg(theme.accent),
        ),
        Span::styled(
            format!("{state} "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("· {} lines published", status.published),
            Style::default().fg(theme.muted),
        ),
    ]);

    element(
        Flex::column()
            .child(
                Dimension::Flex(1),
                element(
                    Boxed::new(element(Text::new(vec![
                        head,
                        Line::from(Span::styled(
                            "output above this box is ordinary terminal scrollback",
                            Style::default().fg(theme.dim),
                        )),
                    ])))
                    .title(" split footer "),
                ),
            )
            .child(
                Dimension::Fixed(1),
                element(KeyHints::new([
                    ("space", "publish now"),
                    ("p", "pause"),
                    ("q", "quit"),
                ])),
            ),
    )
}

/// One published block: a timestamped log line for the scrollback.
fn publish(scrollback: &Scrollback, seq: u64, theme: &Theme) {
    let accent = theme.accent;
    let muted = theme.muted;
    scrollback.write(move |_width| {
        element(Text::new(vec![Line::from(vec![
            Span::styled(format!("[{seq:04}] "), Style::default().fg(muted)),
            Span::styled("build", Style::default().fg(accent)),
            Span::raw(format!(" finished in {} ms", 40 + seq * 3 % 90)),
        ])]))
    });
}

fn main() -> std::io::Result<()> {
    let cli = support::Cli::parse()?;
    // Opt-in mouse capture: off by default so the terminal keeps the wheel and
    // drag-selection over the scrollback this mode preserves.
    let capture_mouse = cli.args.iter().any(|arg| arg == "--mouse");
    if cli.args.iter().any(|arg| arg != "--mouse") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run --example split_footer [-- --mouse] [--theme NAME]",
        ));
    }
    let mode = ScreenMode::split_footer(FOOTER_ROWS);
    let mode = if capture_mouse {
        mode.with_mouse_capture()
    } else {
        mode
    };

    let theme = cli.theme;
    let runner = Runner::new(RunnerConfig {
        tick_rate: Duration::from_millis(80),
        screen_mode: mode,
    });
    let scrollback = runner.scrollback();
    let status = Live::with_redraw(
        Status {
            published: 0,
            paused: false,
        },
        runner.redraw_handle(),
    );

    // A worker publishing on its own schedule: the handle is `Send`, and each
    // block is committed whole by the runner on its next iteration.
    let stop = Arc::new(AtomicBool::new(false));
    let seq = Arc::new(AtomicU64::new(1));
    let worker = thread::spawn({
        let (stop, seq) = (Arc::clone(&stop), Arc::clone(&seq));
        let (scrollback, status) = (scrollback.clone(), status.clone());
        move || {
            while !stop.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(700));
                if status.with(|status| status.paused) {
                    continue;
                }
                publish(&scrollback, seq.fetch_add(1, Ordering::AcqRel), &theme);
                status.update(|status| status.published += 1);
            }
        }
    });

    let view_status = status.clone();
    let mut state = ();
    let result = runner.run(
        &theme,
        from_fn(
            &mut state,
            move |(), frame| {
                element(LiveView::new(view_status.clone(), move |status| {
                    footer(status, frame, &theme)
                }))
            },
            |(), signal| match signal {
                Signal::Event(Event::Key(key)) if key.plain() => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => UpdateResult::Exit,
                    KeyCode::Char('p') => {
                        status.update(|status| status.paused = !status.paused);
                        UpdateResult::Clean
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        publish(&scrollback, seq.fetch_add(1, Ordering::AcqRel), &theme);
                        status.update(|status| status.published += 1);
                        UpdateResult::Clean
                    }
                    _ => UpdateResult::Clean,
                },
                _ => UpdateResult::Clean,
            },
        ),
    );

    stop.store(true, Ordering::Release);
    let _ = worker.join();
    result
}
