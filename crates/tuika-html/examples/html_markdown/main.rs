//! Markdown with raw HTML blocks in it, rendered through the boundary.
//!
//! ```bash
//! cargo run -p tuika-html --example html_markdown # a real terminal (q quits)
//! ```
//!
//! It runs as an actual app rather than printing a grid, because half of what
//! this crate does is *styling* — headings, inline code, links, the table's
//! bold header — and a plain-text dump throws every bit of that away.
//! `scripts/gen-html-demo.sh` records this same binary.

use tuika::components::Markdown;
use tuika::prelude::*;
use tuika::testing::{grid, render};
use tuika_html::HtmlRenderer;

#[path = "../support/mod.rs"]
mod support;

/// Shared with the demo generator so the recording cannot drift from the app.
const DOCUMENT: &str = include_str!("document.md");

fn scene() -> Element {
    // The renderer is `Copy` and holds no state, so the view can own one per
    // frame; a host with a configured `Limits` would keep one beside its state.
    element(Padded)
}

/// The document, inset from the terminal edges.
struct Padded;

impl View for Padded {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        available
    }

    fn render(&self, area: ratatui_core::layout::Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let html = HtmlRenderer::new();
        let inner = ratatui_core::layout::Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };
        Markdown::new(DOCUMENT)
            .block_renderer(&html)
            .render(inner, surface, ctx);
    }
}

fn main() -> std::io::Result<()> {
    let (theme, args) = support::theme_and_args()?;
    if args.iter().any(|a| a == "--dump") {
        let buffer = render(scene().as_ref(), 76, 22, &theme);
        for line in grid(&buffer).lines() {
            println!("{}", line.trim_end());
        }
        return Ok(());
    }

    let runner = Runner::new(RunnerConfig::default());
    runner.run(
        &theme,
        from_fn(
            &mut (),
            |(), _frame| scene(),
            |(), signal| match signal {
                Signal::Event(Event::Key(key))
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
                {
                    UpdateResult::Exit
                }
                _ => UpdateResult::Clean,
            },
        ),
    )
}
