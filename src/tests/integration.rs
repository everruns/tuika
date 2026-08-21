//! Cross-cutting integration tests for the `tuika` toolkit.
//!
//! Per-module unit tests live in each module's own `#[cfg(test)] mod tests`.
//! What remains here are the checks that span several modules at once and have
//! no single owner: composing a real tree, and surviving tiny/degenerate
//! screens where scroll and overlay resolution interact.

use crate::text::{Line, Span};

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
    use crate::style::Color;

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
    use crate::geometry::Rect;

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

/// A split footer end to end, with the pieces a host actually composes: a real
/// component tree painted into the reserved rows, markdown published into the
/// scrollback above it, and the rows handed back on exit. This spans `screen`,
/// `host::paint`, the layout solver, and the markdown renderer — no module owns
/// it alone.
#[test]
fn a_split_footer_composes_components_over_published_markdown() {
    use crate::geometry::Position;
    use crate::term::terminal::{Terminal, TerminalOptions};
    use crate::term::testbackend::TestBackend;
    use crate::term::traits::Backend;

    use crate::components::Markdown;
    use crate::screen::{ScreenMode, Scrollback, close_footer, pin_footer};

    let theme = Theme::default();
    let mut backend = TestBackend::new(28, 10);
    backend
        .set_cursor_position(Position::new(0, 0))
        .expect("place cursor");
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: ScreenMode::split_footer(3).viewport(),
        },
    )
    .expect("inline terminal");
    pin_footer(&mut terminal).expect("pin");

    // An answer arrives and is committed to the terminal's scrollback.
    let scrollback = Scrollback::new();
    scrollback.write(|_width| element(Markdown::new("# Answer\n\nIt was the **cache**.")));
    assert!(scrollback.flush(&mut terminal, &theme).expect("flush"));

    // The footer is a composed tree, not a string: a bordered status panel.
    let draw = |terminal: &mut Terminal<TestBackend>| {
        terminal
            .draw(|frame| {
                let area = frame.area();
                let tree = Flex::column()
                    .grow(
                        1,
                        element(Boxed::new(element(Text::raw("ready"))).title("codex")),
                    )
                    .fixed(
                        1,
                        element(StatusBar::new().left(vec![Span::raw("ctrl-c quit")])),
                    );
                crate::paint(frame.buffer_mut(), area, &Theme::default(), &tree, &[]);
            })
            .expect("draw footer");
    };
    draw(&mut terminal);

    let lines: Vec<String> = {
        let buffer = terminal.backend().buffer();
        (0..10).map(|y| row(buffer, y)).collect()
    };
    // Published markdown sits above the footer, rendered (heading styled, bold
    // inline) rather than as raw source.
    assert!(
        lines[..7].iter().any(|line| line.contains("Answer")),
        "the heading was published: {lines:?}"
    );
    assert!(
        lines[..7].iter().any(|line| line.contains("cache")),
        "the body was published: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("**")),
        "markdown was rendered, not printed as source: {lines:?}"
    );
    // The footer owns exactly the last three rows: border, then the status bar.
    assert!(lines[7].contains("codex"), "footer border row: {lines:?}");
    assert!(lines[9].starts_with("ctrl-c quit"), "status row: {lines:?}");

    close_footer(&mut terminal).expect("close");
    let after: Vec<String> = {
        let buffer = terminal.backend().buffer();
        (0..10).map(|y| row(buffer, y)).collect()
    };
    assert!(
        after[7..].iter().all(|line| line.is_empty()),
        "the footer's rows go back to the terminal: {after:?}"
    );
    assert!(
        after[..7].iter().any(|line| line.contains("cache")),
        "the published answer is the user's to keep: {after:?}"
    );
}

/// The path an overlay's input ownership actually takes: a scene declares the
/// owner, the registry records it, and the router hands the event to that
/// surface — for a paste as much as for a key.
///
/// This is the incident that motivated routing: a masked prompt was open, and a
/// paste landed in the composer behind it because pastes travelled a different
/// path from keys. Here both kinds reach the prompt, and the composer stays
/// empty.
#[test]
fn an_overlay_that_owns_input_receives_pastes_and_keys_alike() {
    use crate::components::{Dialog, SingleLineInputState};
    use crate::event::{Event, Key, KeyCode};
    use crate::focus::FocusRegistry;
    use crate::routing::{RouteStage, Router};
    use crate::scene::ScopedScene;

    let mut composer = SingleLineInputState::new();
    let mut prompt = SingleLineInputState::new();

    // One frame: the base tree plus a dialog that claims input ownership.
    let root = Text::raw("transcript");
    let scene = ScopedScene::new(&root)
        .dialog(Dialog::new("Token", element(Text::raw("paste it"))).focus_owner("prompt"));
    let mut focus = FocusRegistry::new();
    focus.begin_frame();
    focus.register("composer");
    scene.sync_focus(&mut focus);

    for event in [
        Event::Paste("sk-secret".to_string()),
        Event::Key(Key {
            code: KeyCode::Char('v'),
            ctrl: true,
            alt: false,
            shift: false,
        }),
        Event::Key(Key::new(KeyCode::Char('s'))),
    ] {
        let mut router = Router::new(&focus, &event);
        router.target("prompt", &mut prompt);
        router.target("composer", &mut composer);
        let delivery = router.finish();

        assert_eq!(delivery.target, Some("prompt"), "{event:?}");
        assert_eq!(delivery.stage, RouteStage::Target, "{event:?}");
        assert!(composer.text().is_empty(), "{event:?}");
    }

    assert_eq!(prompt.text(), "sk-secrets");

    // Closing the overlay hands input back to the base ring.
    focus.clear_owner();
    let typed = Event::Key(Key::new(KeyCode::Char('h')));
    let mut router = Router::new(&focus, &typed);
    router.target("prompt", &mut prompt);
    router.target("composer", &mut composer);
    assert!(router.finish().reached("composer"));
    assert_eq!(composer.text(), "h");
}
