//! Live mouse demo. Run with `cargo run --example mouse`.
//!
//! Drag across the text to select it — the selection highlights, and releasing
//! the button copies it to the system clipboard via OSC 52. Click the on-screen
//! buttons (`[ clear ]`, `[ quit ]`). Press `q`/`Esc` to quit.
//!
//! This exercises the whole `tuika::mouse` stack end to end: `translate_event`
//! feeding `SelectionState` / `ClickTracker` / `HitMap`, `highlight` +
//! `selected_text` over the rendered buffer, and `clipboard::write_clipboard`
//! (OSC 52). Chrome is drawn with plain ratatui widgets so the interaction rects
//! are exact — the star here is the mouse API, not layout.
//!
//! Shift-drag is intentionally *not* handled: most terminals use it to bypass
//! application mouse capture for their own native selection.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use tuika::term::backend::CrosstermBackend;
use tuika::term::terminal::{Terminal, TerminalOptions, Viewport};
use tuika::ui::Rect;
use tuika::ui::Style;

use tuika::Surface;
use tuika::host::AltScreen;
use tuika::mouse::{ClickTracker, HitMap, SelectionState, paint_selection, selected_text};
use tuika::prelude::*;
use tuika::style::BorderStyle;
use tuika::term::clipboard;

mod support;

const SAMPLE: &[&str] = &[
    "Drag across this text to select it.",
    "Release the button to copy the selection",
    "to your clipboard via OSC 52.",
    "Wide glyphs work too: 你好 world.",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    Clear,
    Quit,
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    enable_raw_mode()?;
    let mut alt = AltScreen::enter_with_mouse_capture()?;
    let mut term = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    let theme = cli.theme;
    let mut sel = SelectionState::new();
    let mut clicks = ClickTracker::new();
    let mut status = String::from("drag to select · click a button · q to quit");
    let sel_style = Style::default()
        .bg(theme.selection_bg)
        .fg(theme.selection_fg);
    // Set when a drag is released; the copy happens in the next draw, where the
    // freshly rendered frame buffer is available to read the selected text from.
    let mut pending_copy = false;

    loop {
        // Rects are computed inside the draw and reused for hit-testing below.
        let mut text_area = Rect::default();
        let mut clear_btn = Rect::default();
        let mut quit_btn = Rect::default();

        term.draw(|f| {
            let area = f.area();
            text_area = Rect::new(
                2,
                2,
                area.width.saturating_sub(4).max(1),
                SAMPLE.len() as u16,
            );
            let btn_y = text_area.bottom().saturating_add(1);
            clear_btn = Rect::new(2, btn_y, 9, 1);
            quit_btn = Rect::new(12, btn_y, 8, 1);

            {
                let full = f.area();
                let mut surface = Surface::new(f.buffer_mut(), full);
                surface.set_string(1, 0, " tuika mouse demo ", theme.accent_style());
                surface.draw_border(
                    Rect::new(1, 1, area.width.saturating_sub(2), SAMPLE.len() as u16 + 2),
                    BorderStyle::Plain.glyphs(),
                    Style::default().fg(theme.border),
                );
                for (i, line) in SAMPLE.iter().enumerate() {
                    surface.set_string(text_area.x, text_area.y + i as u16, line, Style::default());
                }
                surface.set_string(clear_btn.x, clear_btn.y, "[ clear ]", theme.muted_style());
                surface.set_string(quit_btn.x, quit_btn.y, "[ quit ]", theme.muted_style());
            }

            if sel.resolve(f.buffer_mut(), text_area) {
                pending_copy = true;
            }
            if let Some(range) = sel.range() {
                // Read the selected text off the freshly rendered frame *before*
                // highlighting (highlight only changes style, so order is cosmetic).
                if pending_copy {
                    let text = selected_text(f.buffer_mut(), text_area, range);
                    if let Ok(true) = clipboard::write(&mut io::stdout(), &text) {
                        status = format!("copied {} chars", text.chars().count());
                    }
                    pending_copy = false;
                }
                paint_selection(f.buffer_mut(), text_area, range, sel_style);
            } else {
                pending_copy = false;
            }

            {
                let full = f.area();
                let mut surface = Surface::new(f.buffer_mut(), full);
                surface.set_string(
                    1,
                    area.height.saturating_sub(1),
                    &status,
                    theme.muted_style(),
                );
            }
        })?;

        let mut hits: HitMap<Btn> = HitMap::new();
        hits.push(clear_btn, Btn::Clear);
        hits.push(quit_btn, Btn::Quit);

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let ct = event::read()?;
        if let CtEvent::Key(k) = &ct
            && k.kind != KeyEventKind::Release
            && matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break;
        }
        let Some(Event::Mouse(m)) = translate_event(ct) else {
            continue;
        };

        // Keep selection state consistent for every plain gesture.
        let selection_changed = m.plain() && sel.handle(&m);

        // A click that lands on a button wins over selection.
        if let Some(click) = clicks.handle(&m)
            && let Some(btn) = hits.hit(click.column, click.row)
        {
            match btn {
                Btn::Quit => break,
                Btn::Clear => {
                    sel.clear();
                    status = "cleared".into();
                }
            }
            continue;
        }

        // Releasing a drag arms the copy for the next frame.
        if selection_changed
            && matches!(m.kind, MouseKind::Up(MouseButton::Left))
            && sel.range().is_some()
        {
            pending_copy = true;
        }
    }

    let _ = term.clear();
    drop(term);
    alt.leave();
    disable_raw_mode()?;
    Ok(())
}
