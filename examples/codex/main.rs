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
//! token boundaries now fill.
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
//! ```
//!
//! Try: `why does the snapshot test fail?`, `clean up the build artifacts`
//! (raises the approval prompt), `explain this repo`, or `/init`.

mod agent;
mod app;
mod history;
#[path = "../support/mod.rs"]
mod support;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::event;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};

use tuika::components::Flex;
use tuika::probe::RectProbe;
use tuika::screen::{ScreenMode, close_footer, pin_footer, publish_block};
use tuika::{
    Dimension, Element, Padding, RenderCtx, StyleSheet, TerminalSession, Theme, element, paint,
    translate_event,
};

use crate::app::{App, Flow};
use crate::history::Cell;

/// Rows the split-footer mode reserves: enough for the working row, the
/// composer at a couple of lines, a completion popup, and the status footer.
/// The region is fixed, so it is sized for the tallest of those states.
const FOOTER_ROWS: u16 = tuika::screen::DEFAULT_FOOTER_HEIGHT;

/// Frame budget. Also the clock the `Working (12s …)` timer counts in, so the
/// scripted turn and the elapsed display stay in step without a wall clock.
pub const FRAME_MS: u64 = 60;

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let args = cli.args;
    let theme = if cli.theme_name.is_some() {
        cli.theme
    } else {
        app::codex_theme()
    };
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("A replica of the Codex CLI's interface, built on tuika.");
            println!("Not the Codex CLI; not affiliated with OpenAI. The agent is scripted:");
            println!("no model, no network, no shell.");
            println!();
            println!("usage: cargo run --example codex [-- --split-footer] [--theme NAME]");
            Ok(())
        }
        Some("--dump") => ui::dump(args.get(1).map(String::as_str), &theme),
        Some("--split-footer") => run_split(theme),
        _ => run(theme),
    }
}

/// The same app over a split footer: the composer owns the bottom rows, and
/// finished transcript entries are published into the terminal's scrollback.
///
/// The loop is the full-screen one plus two steps — publish what settled, and
/// keep the footer pinned — which is the whole difference between the two modes
/// for a host that drives its own loop.
fn run_split(theme: Theme) -> io::Result<()> {
    let mut app = App::new();
    let sheet = StyleSheet::from_theme(&theme);
    let probe = RectProbe::new();
    let mode = ScreenMode::split_footer(FOOTER_ROWS);

    let _session = TerminalSession::enter_with(mode)?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: mode.viewport(),
        },
    )?;

    loop {
        // Hand over everything that will not change again. A cell holds a
        // streaming-markdown cache and so is not `Send`; `publish_block` takes
        // it straight from this loop rather than through the `Scrollback` queue.
        let width = terminal.get_frame().area().width;
        let ctx = RenderCtx::new(&theme).with_sheet(sheet);
        for mut cell in app.drain_settled() {
            let view = published(&mut cell, width, &theme, &sheet);
            publish_block(&mut terminal, view.as_ref(), &ctx)?;
        }

        // Resolve a resize before re-pinning, so the footer is measured against
        // the screen it is about to be drawn on.
        terminal.autoresize()?;
        pin_footer(&mut terminal)?;
        terminal.draw(|f| {
            let area = f.area();
            let root = ui::build(&mut app, area, &theme, &sheet, &probe);
            paint(f.buffer_mut(), area, &theme, root.as_ref(), &[]);
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

    // Give the reserved rows back; everything published above them is the
    // user's session now.
    let _ = close_footer(&mut terminal);
    drop(terminal);
    Ok(())
}

/// One transcript entry as it goes into the scrollback: the same view the
/// full-screen transcript renders, in the same gutter, with the blank row that
/// separates entries there folded into the block.
fn published(cell: &mut Cell, width: u16, theme: &Theme, sheet: &StyleSheet) -> Element {
    let inner = width.saturating_sub(ui::GUTTER * 2);
    element(
        Flex::column()
            .padding(Padding {
                left: ui::GUTTER,
                right: ui::GUTTER,
                top: 1,
                bottom: 0,
            })
            .child(Dimension::Auto, cell.view(inner, theme, sheet)),
    )
}

fn run(theme: Theme) -> io::Result<()> {
    let mut app = App::new();
    let sheet = StyleSheet::from_theme(&theme);
    let probe = RectProbe::new();

    let _session = TerminalSession::enter_with(ScreenMode::Alternate.with_mouse_capture())?;
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
