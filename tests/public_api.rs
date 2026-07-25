//! The public module layout, exercised as a host sees it.
//!
//! Every path here is API, so this file is the executable half of
//! `knowledge/specs/api-surface.md`: an integration test compiles against the
//! crate from outside, which means a path that moves, a re-export that is
//! dropped, or a prelude that stops carrying a name fails here rather than in a
//! host's build. The rendering assertions are what keep it from degrading into
//! a list of `use` statements — the imported types have to actually work.

use tuika::prelude::*;
use tuika::testing::{grid, render};

/// The prelude is the documented one-line import, so it has to be enough on its
/// own to build and render a real tree.
#[test]
fn the_prelude_alone_can_build_and_render_a_screen() {
    let theme = Theme::default();
    let screen = element(
        Flex::column()
            .fixed(1, element(Text::raw("title")))
            .fixed(1, element(Rule::new()))
            .grow(1, element(Text::raw("body"))),
    );
    assert_eq!(
        grid(&render(screen.as_ref(), 5, 3, &theme)),
        "title\n─────\nbody "
    );
}

/// `view!` expands to `$crate::components::…` paths. Only an out-of-crate
/// expansion proves those paths resolve for a host.
#[test]
fn the_view_macro_expands_against_the_component_paths() {
    let theme = Theme::default();
    let screen = view! {
        col {
            fixed(1) { text("one") }
            fixed(1) { text("two") }
        }
    };
    assert_eq!(grid(&render(screen.as_ref(), 3, 2, &theme)), "one\ntwo");
}

/// Components stay reachable by their flat `components::` path even where the
/// owning module is public.
#[test]
fn components_are_reachable_flat() {
    use tuika::components::{AsciiFont, Boxed, Console, Diff, Image, Markdown, QrCode, Toasts};

    let theme = Theme::default();
    let boxed = element(Boxed::new(element(tuika::components::Text::raw("x"))));
    assert_eq!(
        grid(&render(boxed.as_ref(), 5, 3, &theme)),
        "╭───╮\n│ x │\n╰───╯"
    );

    // Named only to pin the paths; constructing each is the component's own
    // test's job.
    let _ = AsciiFont::new("hi");
    let _ = Console::new;
    let _ = Diff::new;
    let _ = Image::new;
    let _ = Markdown::new("# hi");
    let _ = QrCode::encode("hi", QrEcc::Low);
    let _ = Toasts::new;
}

/// A component module is public exactly when it owns a constant or free
/// function the flat namespace cannot name well.
#[test]
fn component_modules_own_their_constants_and_free_functions() {
    use tuika::components::{ascii_font, console, diff, markdown, qr, text, toast};

    // Read through their modules rather than as hand-prefixed root aliases. The
    // values are each component's own contract, so this only pins that they are
    // reachable and sane.
    let defaults = [
        u64::from(ascii_font::FONT_HEIGHT),
        console::DEFAULT_CAPACITY as u64,
        toast::DEFAULT_TTL,
    ];
    assert!(defaults.iter().all(|value| *value > 0));

    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);
    let plain = CodeHighlighter::Plain;
    assert!(!markdown::to_lines("# hi", 20, &theme, &sheet, plain).is_empty());
    assert!(
        !markdown::to_linked_lines("[a](https://example.com)", 20, &theme, &sheet, plain)
            .0
            .is_empty()
    );
    assert!(!diff::rows("a\n", "b\n").is_empty());
    assert!(qr::encode(b"hi", tuika::components::QrEcc::Low).is_some());
    assert_eq!(text::wrap_lines(&[], 10).len(), 0);
}

/// Everything out-of-band lives under `term`, and each escape module exposes the
/// same pure-`encode` shape.
#[test]
fn term_groups_the_out_of_band_escapes() {
    use tuika::term::{capabilities, clipboard, hyperlink, image, pointer, progress};

    assert_eq!(clipboard::encode("hi").unwrap(), "\x1b]52;c;aGk=\x1b\\");
    assert!(hyperlink::encode("https://example.com", "link").contains("\x1b]8;;"));
    assert_eq!(
        pointer::encode(pointer::PointerShape::Pointer),
        "\x1b]22;pointer\x1b\\"
    );
    assert_eq!(
        progress::encode(progress::ProgressState::Normal, 50),
        "\x1b]9;4;1;50\x07"
    );

    let mut sink: Vec<u8> = Vec::new();
    assert!(clipboard::write(&mut sink, "hi").expect("write"));
    assert!(pointer::write(&mut sink, pointer::PointerShape::Default).is_ok());

    // The image protocol half is here; the view half is a component.
    assert!(image::ImageData::from_rgba(1, 1, vec![0, 0, 0, 255]).is_some());
    let _ = image::ImageLayer::new();
    let _ = image::ImageSupport::detect_from(None, None, None, None);
    let _ = capabilities::Capabilities::from_env();
}

/// The paths that used to collide at the crate root now name one thing each.
#[test]
fn the_former_root_collisions_resolve() {
    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;
    use ratatui_core::style::{Color, Style};
    use tuika::mouse::{SelectionRange, paint_selection};

    // `tuika::highlight` is the Highlighter seam, and nothing else.
    let _: &dyn tuika::highlight::Highlighter = &PlainHighlighter;

    // Painting a selection is a mouse concern, and says so.
    let area = Rect::new(0, 0, 4, 1);
    let mut buffer = Buffer::empty(area);
    buffer.set_string(0, 0, "abcd", Style::default());
    let selection = SelectionRange {
        start: (0, 0),
        end: (1, 0),
    };
    paint_selection(
        &mut buffer,
        area,
        selection,
        Style::default().bg(Color::Blue),
    );
    assert_eq!(buffer[(0, 0)].style().bg, Some(Color::Blue));
    assert_eq!(buffer[(3, 0)].style().bg, Some(Color::Reset));

    // `Overlay` sits beside the spec that resolves its rect.
    let dialog = tuika::components::Text::raw("hi");
    let rect = OverlaySpec::centered(50, 50).min_size(2, 1).resolve(area);
    let overlays = [Overlay {
        area: rect,
        view: &dialog,
        clear: true,
    }];
    let mut screen = Buffer::empty(Rect::new(0, 0, 8, 3));
    paint(
        &mut screen,
        Rect::new(0, 0, 8, 3),
        &Theme::default(),
        &tuika::components::Text::raw("base"),
        &overlays,
    );
    assert!(grid(&screen).contains("hi"));
}

/// The remaining modules keep their own path on purpose; this pins them so a
/// later "helpful" flattening is a test failure rather than a silent addition.
#[test]
fn the_non_prelude_surface_keeps_its_module_path() {
    assert_eq!(tuika::width::str_cols("ab"), 2);
    assert!(tuika::themes::by_name("gruvbox-dark").is_some());
    let _ = tuika::probe::RectProbe::new();
    let _ = tuika::framebuffer::FrameBuffer::new(1, 1);
    let _ = tuika::interop::RatatuiView::sized(Size::new(1, 1), |_area, _buffer: &mut _| {});
    let _ = tuika::runner::Runner::new(RunnerConfig::default());
    // `AltScreen::enter` touches the real terminal, so only its path is pinned.
    let _: fn() -> std::io::Result<tuika::host::AltScreen> = tuika::host::AltScreen::enter;
}
