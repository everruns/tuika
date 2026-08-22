//! Overlay compositing + input routing. Run with
//! `cargo run --example overlay` (o open · enter/esc close · q quit).
//!
//! Demonstrates a [`SceneOverlay`] following a laid-out trigger through a
//! [`RectProbe`](tuika::probe::RectProbe). The preferred below-target placement
//! flips above when the terminal edge leaves more room there. While the popover
//! is open it owns input — the same key means different things by overlay state.

use std::io;
use std::time::Duration;

use crossterm::event::{self};
use tuika::term::backend::CrosstermBackend;
use tuika::term::terminal::{Terminal, TerminalOptions, Viewport};
use tuika::ui::{Line, Span};

use tuika::prelude::*;

mod support;
use tuika::probe::RectProbe;

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let mut open = false;
    let mut confirmed = 0u32;
    let trigger = RectProbe::new();

    let _session = TerminalSession::enter()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    let theme = cli.theme;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let hint = if open {
                "popover open — enter = confirm · esc = dismiss"
            } else {
                "o open popover · q quit"
            };
            let trigger_view = trigger.wrap(Text::new(vec![Line::from(Span::styled(
                "[ o Open actions ]",
                theme.accent_style(),
            ))]));
            let base: Element = view! {
                col(padding = Padding::all(1), gap = 1) {
                    node(Text::new(vec![Line::from(Span::styled(
                        "tuika overlay demo",
                        theme.accent_style(),
                    ))]))
                    fixed(4) {
                        boxed(title = Line::from(" base layer ")) {
                            node(Text::raw(format!(
                                "confirmed {confirmed} time(s) — base content stays visible around the panel"
                            )))
                        }
                    }
                    grow(1) { spacer() }
                    fixed(1) {
                        row {
                            grow(1) { spacer() }
                            fixed(18) { node(trigger_view) }
                        }
                    }
                    fixed(1) {
                        node(Text::new(vec![Line::from(Span::styled(hint, theme.muted_style()))]))
                    }
                }
            };

            let mut scene = Scene::new(base);
            if open {
                let popover = view! {
                    boxed(
                        title = Line::from(Span::styled(" actions ", theme.accent_style())),
                        padding = Padding::all(1)
                    ) {
                        col(gap = 1) {
                            node(Text::raw("Proceed with the action?"))
                            node(Text::new(vec![Line::from(Span::styled(
                                "enter = yes · esc = no",
                                theme.muted_style(),
                            ))]))
                        }
                    }
                };
                let spec = OverlaySpec {
                    width: tuika::overlay::Extent::Cells(38),
                    height: tuika::overlay::Extent::Cells(7),
                    ..OverlaySpec::centered(0, 0).margin(1)
                };
                scene = scene.overlay(
                    SceneOverlay::new(popover, spec)
                        .target(
                            &trigger,
                            TargetPlacement::below().align(TargetAlign::End).gap(1),
                        )
                        .focus_owner("actions"),
                );
            }
            paint_scene(f.buffer_mut(), area, &theme, &scene);
        })?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Some(Event::Key(k)) = translate_event(event::read()?) else {
            continue;
        };
        if !k.plain() {
            continue;
        }
        match (open, k.code) {
            (false, KeyCode::Char('q')) => break,
            (false, KeyCode::Char('o')) => open = true,
            (true, KeyCode::Enter) => {
                confirmed += 1;
                open = false;
            }
            (true, KeyCode::Esc) => open = false,
            _ => {}
        }
    }

    let _ = terminal.clear();
    drop(terminal);
    Ok(())
}
