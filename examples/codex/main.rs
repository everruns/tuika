//! A **replica of the OpenAI Codex CLI's interface**, built on tuika. Run with
//! `cargo run --example codex` (type a prompt and press enter · `/` for
//! commands · `@` for files · `⌃C` to quit).
//!
//! This is **not** the Codex CLI and is not affiliated with or endorsed by
//! OpenAI; no part of Codex is used or included. It imitates the *shape* of a
//! coding-agent TUI as closely as the toolkit allows — a transcript of bulleted
//! events and bordered panels above a composer, a slash-command popup, an
//! escalation prompt for a command the sandbox will not run unattended, a
//! `Working (12s • Esc to interrupt)` row, and a context meter in the footer —
//! because that shape is the hardest thing to assemble from single components,
//! and building it is what surfaced the gaps `ItemScroll` and the composer
//! token seams now fill.
//!
//! Everything behind the interface is fake: `agent.rs` replays a scripted turn
//! chosen from the prompt's keywords, revealing it frame by frame, so there is
//! no model, no network, and no shell.
//!
//! What it exercises, and why it is a useful example: streaming
//! [`MarkdownState`](tuika::MarkdownState) answers that only re-parse their
//! in-flight tail, a [`Scroll`](tuika::Scroll) transcript that sticks to the
//! bottom until the user pages away, [`TextInputState`](tuika::TextInputState)
//! with the terminal caret placed through a [`RectProbe`](tuika::RectProbe),
//! [`SelectState`](tuika::SelectState) pickers layered over the composer, and a
//! focus model where the same key means different things depending on which
//! surface is up.
//!
//! ```text
//! cargo run --example codex                 # interactive
//! cargo run --example codex -- --dump       # print canned frames as text
//! cargo run --example codex -- --dump slash # just one of them
//! ```
//!
//! Try: `why does the snapshot test fail?`, `clean up the build artifacts`
//! (raises the approval prompt), `explain this repo`, or `/init`.

mod agent;
mod app;
mod history;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::event;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};

use tuika::probe::RectProbe;
use tuika::{StyleSheet, TerminalSession, paint, translate_event};

use crate::app::{App, Flow};

/// Frame budget. Also the clock the `Working (12s …)` timer counts in, so the
/// scripted turn and the elapsed display stay in step without a wall clock.
pub const FRAME_MS: u64 = 60;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("A replica of the Codex CLI's interface, built on tuika.");
            println!("Not the Codex CLI; not affiliated with OpenAI. The agent is scripted:");
            println!("no model, no network, no shell.");
            println!();
            println!("usage: cargo run --example codex [-- --dump [scene]]");
            println!("scenes: welcome, turn, approval, slash, mention, status");
            Ok(())
        }
        Some("--dump") => ui::dump(args.get(1).map(String::as_str)),
        _ => run(),
    }
}

fn run() -> io::Result<()> {
    let mut app = App::new();
    let theme = app::codex_theme();
    let sheet = StyleSheet::from_theme(&theme);
    let probe = RectProbe::new();

    let _session = TerminalSession::enter()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let root = ui::build(&mut app, area, &theme, &sheet, &probe);
            paint(f.buffer_mut(), area, &theme, root.as_ref(), &[]);
            // The composer's rect is only known after layout; the probe reports
            // where it landed, and the real terminal caret goes there.
            if let Some(pos) = app.cursor(probe.rect()) {
                f.set_cursor_position(pos);
            }
        })?;

        if event::poll(Duration::from_millis(FRAME_MS))?
            && let Some(ev) = translate_event(event::read()?)
            && app.handle(&ev) == Flow::Quit
        {
            break;
        }
        app.tick();
    }

    let _ = terminal.clear();
    drop(terminal);
    Ok(())
}
