//! Cross-cutting integration tests for the `tuika` toolkit.
//!
//! Per-module unit tests live in each module's own `#[cfg(test)] mod tests`.
//! What remains here are the checks that span several modules at once and have
//! no single owner: composing a real tree, and surviving tiny/degenerate
//! screens where scroll and overlay resolution interact.

use ratatui_core::text::{Line, Span};

use crate::components::{Boxed, Flex, Scroll, ScrollState, StatusBar, Text};
use crate::overlay::OverlaySpec;
use crate::style::Theme;
use crate::surface::Surface;
use crate::tests::support::{buffer, row};
use crate::view::{RenderCtx, View, element};

/// An inherited theme has to reach the *cells*, not just the `Theme` struct —
/// which means going through a real component tree and reading the buffer back.
///
/// Two paths, and the assertions differ because the promises differ. A theme
/// derived from a terminal's reply pins cells to concrete 24-bit values; the
/// zero-I/O preset pins them to `Reset` and ANSI indices, because its whole
/// point is that the *terminal* resolves them.
#[test]
fn an_inherited_theme_reaches_the_painted_cells() {
    use ratatui_core::style::Color;

    use crate::term::palette::TerminalPalette;
    use crate::themes;

    // A theme derived from what a terminal actually reported, alongside the
    // preset that needs no report at all.
    let palette = TerminalPalette::parse(
        b"\x1b]11;rgb:1c1c/1e1e/2626\x1b\\\x1b]10;rgb:d5d5/d0d0/c8c8\x1b\\\
          \x1b]4;12;rgb:5c5c/9f9f/ecec\x1b\\\x1b[?62c",
    );
    let derived = Theme::from_terminal(&palette).expect("a complete theme");

    // A bordered panel pins the screen fill and the border…
    let panel = element(Boxed::new(element(Text::raw("body"))));
    let buf = crate::testing::render(panel.as_ref(), 12, 3, &derived);
    assert_eq!(
        buf[(5, 1)].bg,
        Color::Rgb(0x1c, 0x1e, 0x26),
        "the screen fill is the reported background, verbatim"
    );
    assert_eq!(buf[(0, 0)].fg, derived.border);
    assert_ne!(
        derived.border, derived.text,
        "the border is a derived tone, not one of the two reported colors"
    );

    let buf = crate::testing::render(panel.as_ref(), 12, 3, &themes::TERMINAL);
    assert_eq!(
        buf[(5, 1)].bg,
        Color::Reset,
        "the terminal's own background shows through"
    );
    assert_eq!(
        buf[(0, 0)].fg,
        Color::Indexed(8),
        "the border is an ANSI slot the terminal resolves"
    );

    // …and a console pins text, surface, and accent. It borrows its log, so it
    // renders directly rather than through `element`.
    let log = crate::components::ConsoleLog::new(8);
    log.line("entry");
    let console = crate::components::Console::new(&log).title("t");
    let paint_console = |theme: &Theme| {
        let mut buf = buffer(12, 3);
        let area = buf.area;
        let ctx = RenderCtx::new(theme);
        let mut surface = Surface::new(&mut buf, area);
        console.render(area, &mut surface, &ctx);
        buf
    };

    let buf = paint_console(&derived);
    assert_eq!(
        buf[(0, 2)].fg,
        Color::Rgb(0xd5, 0xd0, 0xc8),
        "body text is the reported foreground, verbatim"
    );
    assert_eq!(
        buf[(0, 0)].fg,
        Color::Rgb(0x5c, 0x9f, 0xec),
        "the accent is the bright blue it reported"
    );
    assert_eq!(buf[(0, 2)].bg, derived.surface);
    assert_ne!(
        derived.surface, derived.background,
        "the raised fill is derived, and distinct from the base"
    );

    let buf = paint_console(&themes::TERMINAL);
    assert_eq!(
        buf[(0, 2)].fg,
        Color::Reset,
        "body text is the terminal's own foreground"
    );
    assert_eq!(buf[(0, 0)].fg, Color::Indexed(4), "so is the accent");
}

#[test]
fn scroll_and_overlay_survive_tiny_screens() {
    use ratatui_core::layout::Rect;

    // A tall scroll region rendered into a 1×1 viewport.
    let lines: Vec<Line<'static>> = (0..50).map(|i| Line::from(format!("l{i}"))).collect();
    let mut st = ScrollState::new();
    st.clamp(50, 1);
    let theme = Theme::default();
    let ctx = RenderCtx::new(&theme);
    let mut buf = buffer(1, 1);
    let area = buf.area;
    let mut surface = Surface::new(&mut buf, area);
    Scroll::new(lines, &st).render(area, &mut surface, &ctx);

    // Overlay resolution on tiny/zero screens stays within bounds.
    for (w, h) in [(0u16, 0u16), (1, 1), (2, 3), (5, 5)] {
        let screen = Rect::new(0, 0, w, h);
        let rect = OverlaySpec::centered(80, 80).min_size(4, 4).resolve(screen);
        assert!(rect.right() <= screen.right(), "overlay exceeds {screen:?}");
        assert!(
            rect.bottom() <= screen.bottom(),
            "overlay exceeds {screen:?}"
        );
    }
}

#[test]
fn nested_flex_tree_lays_out_status_and_body() {
    let theme = Theme::default();
    let mut buf = buffer(24, 6);
    let area = buf.area;

    let body = Boxed::new(element(Text::raw("hi"))).title("Body");
    let status = StatusBar::new().left(vec![Span::raw("model: sim")]);
    let tree = Flex::column()
        .grow(1, element(body))
        .fixed(1, element(status));

    let ctx = RenderCtx::new(&theme);
    let mut surface = Surface::new(&mut buf, area);
    tree.render(area, &mut surface, &ctx);

    // Status bar occupies the last row.
    assert!(row(&buf, 5).starts_with("model: sim"));
    // Body box drew a rounded border on the top row with its title.
    assert!(row(&buf, 0).contains("Body"));
    assert_eq!(buf[(0, 0)].symbol(), "╭");
}
