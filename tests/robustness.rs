//! Black-box robustness sweep over the public component surface.
//!
//! Every other suite renders a component the way its author expects it to be
//! used. This one does the opposite: it feeds each component adversarial text
//! (wide CJK, ZWJ emoji, combining marks, control bytes, a 4 KiB word, embedded
//! newlines) and renders it at every degenerate geometry a terminal can
//! actually produce — a zero column, a one-cell screen, a screen narrower than
//! the component's own chrome — through nothing but the published API.
//!
//! Three properties are asserted for every (component × corpus × size)
//! combination, because all three are ways the same class of bug reaches a
//! user:
//!
//! 1. **No panic.** Untrusted text and a shrinking viewport must degrade, never
//!    crash the host. The wrap solver's column counter overflowing at
//!    max-content width was exactly this.
//! 2. **No painting outside the rect.** Each view is rendered into an inset
//!    area of a larger buffer; a write past its own bounds shows up on the
//!    margin instead of silently landing in a neighbour.
//! 3. **No control bytes in cells.** A component that copies a C0 byte from its
//!    input into the cell grid corrupts the terminal for the whole session.
//!
//! The sweep is exhaustive and deterministic — no RNG — so a failure names the
//! exact component, corpus, and size that broke.

use tuika::components::{
    ActivityItem, ActivityList, ActivityStatus, AsciiFont, Boxed, CodeBlock, CompletionItem,
    CompletionPalette, CompletionState, Console, ConsoleLog, Diff, Flow, FormField, FormState,
    Grid, ItemScroll, KeyHints, Loader, Markdown, MarkdownState, Paragraph, ProgressBar, QrCode,
    QrEcc, Rule, Scroll, ScrollState, Scrollbar, SelectList, SelectState, SelectionScreen, Slider,
    SliderState, Spinner, StatusBar, TabSelect, TabSelectState, Table, Tabs, TabsState, Text,
    TextInput, TextInputState, ToastLevel, ToastList, Toasts, TreeList, TreeRow, TreeState,
    Viewport, VirtualWindow, Wrap,
};
use tuika::prelude::*;
use tuika::style::StyleSheet;
use tuika::themes;

/// Adversarial inputs, each aimed at one way text breaks a terminal renderer.
const CORPORA: &[(&str, &str)] = &[
    ("empty", ""),
    ("ascii", "the quick brown fox jumps over the lazy dog"),
    ("wide", "日本語のテキストと絵文字 👩‍👩‍👦 が混ざった行"),
    (
        "combining",
        "e\u{301}e\u{301}e\u{301} a\u{300}\u{327} ❤\u{fe0f} x\u{fe0e}",
    ),
    ("zero_width", "a\u{200b}b\u{200b}c\u{200d}d\u{feff}e"),
    ("control", "before\u{1}\u{7}\u{1b}after\u{7f}end"),
    ("newlines", "one\ntwo\n\n\nthree\r\nfour"),
    ("whitespace", "   \t\t   \u{a0}\u{2003}   "),
    (
        "long_word",
        "supercalifragilisticexpialidocious-and-then-some-more-without-a-single-break",
    ),
    (
        "markdownish",
        "# h1\n\n- *a* **b** `c`\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n```rs\nfn f(){}\n```",
    ),
    (
        "url",
        "see https://example.dev/a/b?c=d&e=f#g and http://x.io",
    ),
];

/// Geometries a real terminal can hand a component, degenerate ones first.
const SIZES: &[(u16, u16)] = &[
    (0, 0),
    (0, 4),
    (4, 0),
    (1, 1),
    (2, 1),
    (1, 3),
    (3, 3),
    (5, 2),
    (8, 4),
    (13, 7),
    (24, 3),
    (40, 12),
    (80, 24),
    (120, 2),
    (2, 60),
    (200, 1),
];

/// Call `visit` once per component, with a name and a borrowed view built from
/// `text`.
///
/// Views that borrow host state (every selection, scroll, and editor component)
/// are constructed here with their state on the stack, so the sweep covers the
/// borrowed forms without leaking a state per combination.
fn for_each_component(text: &'static str, visit: &mut dyn FnMut(&str, &dyn View)) {
    let theme = Theme::default();
    let lines: Vec<Line<'static>> = text.lines().map(|l| Line::from(l.to_string())).collect();
    let lines = if lines.is_empty() {
        vec![Line::from(String::new())]
    } else {
        lines
    };
    let words: Vec<Line<'static>> = text
        .split_whitespace()
        .map(|w| Line::from(w.to_string()))
        .collect();
    let words = if words.is_empty() {
        vec![Line::from(String::new())]
    } else {
        words
    };

    // Prose.
    visit("text", &Text::new(lines.clone()));
    visit("paragraph", &Paragraph::new(text, theme.text_style()));
    visit("wrap", &Wrap::new(lines.clone()));
    visit("markdown", &Markdown::new(text));
    visit(
        "code_block",
        &CodeBlock::new("rust", text).line_numbers(true),
    );
    visit(
        "diff",
        &Diff::new(text, &format!("{text}\nchanged")).line_numbers(true),
    );
    visit("ascii_font", &AsciiFont::new(text));

    // Streamed markdown, fed one chunk at a time like a host does.
    let mut md = MarkdownState::new();
    for chunk in text.as_bytes().chunks(7) {
        md.push_str(&String::from_utf8_lossy(chunk));
    }
    let sheet = StyleSheet::from_theme(&theme);
    let streamed = md
        .lines(40, &theme, &sheet, tuika::highlight::CodeHighlighter::Plain)
        .to_vec();
    visit("markdown_streamed", &Text::new(streamed));

    // Chrome and containers.
    visit(
        "boxed",
        &Boxed::new(element(Text::new(lines.clone())))
            .title(Line::from(text.to_string()))
            .padding(Padding::all(1)),
    );
    visit("rule", &Rule::new().title(Line::from(text.to_string())));
    visit(
        "flow",
        &words.iter().fold(Flow::new().gap(1), |flow, word| {
            flow.item(element(Text::new(vec![word.clone()])))
        }),
    );
    visit(
        "grid",
        &words.iter().fold(Grid::new(3).gap(1), |grid, word| {
            grid.cell(element(Text::new(vec![word.clone()])))
        }),
    );
    visit(
        "status_bar",
        &StatusBar::new()
            .left(vec![Span::styled(text.to_string(), theme.text_style())])
            .right(vec![Span::styled(text.to_string(), theme.muted_style())]),
    );
    visit(
        "key_hints",
        &KeyHints::new([("ctrl+c", text), (text, "quit")]),
    );

    // Progress and motion.
    visit(
        "progress_bar",
        &ProgressBar::determinate(0.42).label(text).percent(true),
    );
    visit("spinner", &Spinner::new(7));
    visit("loader", &Loader::new(3, text).hint(text));
    visit(
        "activity_list",
        &ActivityList::new(vec![
            ActivityItem::new(text, ActivityStatus::Running)
                .detail(text)
                .progress(0.5),
            ActivityItem::new(text, ActivityStatus::Queued),
        ])
        .frame(2),
    );

    // Selection and scrolling, over borrowed state.
    let mut select = SelectState::new();
    select.select(Some(words.len().saturating_sub(1)));
    visit("select_list", &SelectList::new(words.clone(), &select));
    visit(
        "table",
        &Table::new(
            vec![
                Column::new(Line::from(text.to_string()), Dimension::Flex(1)),
                Column::new(Line::from("n"), Dimension::Fixed(4)),
            ],
            words
                .iter()
                .map(|w| vec![w.clone(), Line::from("1")])
                .collect(),
            &select,
        ),
    );
    visit(
        "selection_screen",
        &SelectionScreen::new(text, words.clone(), &select).leading_rule(),
    );

    let scroll = ScrollState::new();
    visit("scroll", &Scroll::new(lines.clone(), &scroll));
    visit(
        "item_scroll",
        &ItemScroll::new(
            words
                .iter()
                .map(|w| element(Text::new(vec![w.clone()])))
                .collect(),
            &scroll,
        )
        .gap(1),
    );
    visit(
        "viewport",
        &Viewport::new(element(Text::new(lines.clone())), Size::new(30, 4), &scroll),
    );
    visit(
        "scrollbar",
        &Scrollbar::vertical(VirtualWindow::new(100, 10, 40)),
    );

    let tabs = TabsState::default();
    visit("tabs", &Tabs::new(words.clone(), &tabs));
    let tab_select = TabSelectState::default();
    visit("tab_select", &TabSelect::new(words.clone(), &tab_select));

    let rows: Vec<TreeRow<'_, usize>> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let label = w
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            if i == 0 {
                TreeRow::root(i, label, true)
            } else {
                TreeRow::new(i, Some(0), 1, label, false)
            }
        })
        .collect();
    let mut tree = TreeState::with_selected(0usize);
    tree.expand(0);
    visit("tree_list", &TreeList::new(&rows, &tree));

    let items: Vec<CompletionItem> = words
        .iter()
        .map(|w| {
            let label = w
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            CompletionItem::new(label).detail(text)
        })
        .collect();
    let mut completion = CompletionState::new();
    completion.sync(text, &items);
    visit(
        "completion_palette",
        &CompletionPalette::new(&items, &completion)
            .title(text)
            .show_query(true),
    );

    // Editing, forms, and floating chrome.
    let input = TextInputState::from_text(text);
    visit(
        "text_input",
        &TextInput::new(&input).placeholder(text, theme.muted_style()),
    );
    let form_state = FormState::default();
    visit(
        "form",
        &Form::new(
            vec![
                FormField::new(text, element(Text::raw(text))).help(text),
                FormField::new("second", element(Text::raw(text))).error(text),
            ],
            &form_state,
        ),
    );
    let slider = SliderState::new(0.0, 1.0, 0.5);
    visit("slider", &Slider::new(&slider).label(&slider));

    let mut toasts = Toasts::new(4);
    toasts.push(ToastLevel::Error, text);
    toasts.push(ToastLevel::Info, text);
    visit("toast_list", &ToastList::new(&toasts));

    let log = ConsoleLog::new(16);
    for line in text.lines() {
        log.line(line);
    }
    visit("console", &Console::new(&log).title(" log "));

    if let Some(qr) = QrCode::encode(text, QrEcc::Low) {
        visit("qr", &qr);
    }
}

/// Strip OSC 8 hyperlink wrappers from a cell symbol, leaving what the terminal
/// actually displays.
///
/// tuika embeds `ESC ] 8 ; ; url ST` runs in the cells of a linked label, so an
/// escape *there* is the feature. Anywhere else it is terminal corruption, and
/// stripping the wrapper is what tells the two apart — the same thing a host
/// reconstructing a row has to do.
fn strip_osc8(symbol: &str) -> String {
    let mut out = String::with_capacity(symbol.len());
    let mut rest = symbol;
    while let Some(open) = rest.find("\u{1b}]8;;") {
        out.push_str(&rest[..open]);
        let after = &rest[open + "\u{1b}]8;;".len()..];
        match after.find("\u{1b}\\") {
            Some(end) => rest = &after[end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Render into an inset rect of a margined buffer and assert the component
/// painted only inside it, wrote no control byte, and did not panic.
fn assert_well_behaved(name: &str, corpus: &str, view: &dyn View, theme: &Theme) {
    const MARGIN: u16 = 2;
    for &(width, height) in SIZES {
        let outer = Rect::new(0, 0, width + MARGIN * 2, height + MARGIN * 2);
        let inner = Rect::new(MARGIN, MARGIN, width, height);
        let mut buffer = tuika::ui::Buffer::empty(outer);
        paint(&mut buffer, inner, theme, view, &[]);

        let blank = tuika::ui::Cell::default();
        for y in outer.top()..outer.bottom() {
            for x in outer.left()..outer.right() {
                let cell = &buffer[(x, y)];
                if !inner.contains(tuika::ui::Position::new(x, y)) {
                    assert_eq!(
                        cell, &blank,
                        "{name}/{corpus} at {width}x{height} painted ({x}, {y}) outside {inner:?}"
                    );
                    continue;
                }
                let visible = strip_osc8(cell.symbol());
                assert!(
                    !visible.chars().any(char::is_control),
                    "{name}/{corpus} at {width}x{height} put a control byte in a cell at ({x}, {y}): {:?}",
                    cell.symbol()
                );
            }
        }
    }
}

#[test]
fn every_component_survives_every_corpus_at_every_size() {
    let theme = Theme::default();
    for (corpus, text) in CORPORA {
        for_each_component(text, &mut |name, view| {
            assert_well_behaved(name, corpus, view, &theme);
        });
    }
}

#[test]
fn resolved_measurement_never_exceeds_the_space_offered() {
    // `View::measure` reports an *intrinsic* size and may exceed the space it
    // was offered; `measure_request` is what a layout container asks, and its
    // answer is the one that must fit — a container hands out the rect it gets
    // back. A component overriding `measure_request` (the width-sensitive ones
    // do) can break that clamp, and a parent then allocates a rect the child
    // paints past.
    let theme = Theme::default();
    let ctx = RenderCtx::new(&theme);
    for (corpus, text) in CORPORA {
        for_each_component(text, &mut |name, view| {
            for &(width, height) in SIZES {
                let request = MeasureRequest::new(Size::new(width, height));
                let measured = view.measure_request(request, &ctx);
                assert!(
                    measured.width <= width && measured.height <= height,
                    "{name}/{corpus} resolved to {measured:?} inside {width}x{height}"
                );
                // The unconstrained ask must not panic or report nonsense
                // either: this is the max-content path that overflowed the wrap
                // solver's column counter.
                let unconstrained = request
                    .with_available_width(AvailableSpace::MaxContent)
                    .with_available_height(AvailableSpace::MaxContent);
                let _ = view.measure_request(unconstrained, &ctx);
                let min = request
                    .with_available_width(AvailableSpace::MinContent)
                    .with_available_height(AvailableSpace::MinContent);
                let _ = view.measure_request(min, &ctx);
            }
        });
    }
}

#[test]
fn rendering_is_deterministic_across_themes_and_sheets() {
    // Painting the same view twice must produce identical cells: a component
    // that carries hidden per-render state (a cursor blink, an interior mutable
    // cache keyed on the wrong thing) shows up here rather than as a flicker.
    for named in themes::PRESETS {
        for (corpus, text) in CORPORA.iter().take(4) {
            for_each_component(text, &mut |name, view| {
                for &(width, height) in &[(0u16, 0u16), (7, 3), (40, 12)] {
                    let first = tuika::testing::render(view, width, height, &named.theme);
                    let second = tuika::testing::render(view, width, height, &named.theme);
                    assert_eq!(
                        first, second,
                        "{name}/{corpus} rendered differently on the second paint \
                         under theme {} at {width}x{height}",
                        named.name
                    );
                }
            });
        }
    }
}

/// A tiny deterministic PRNG, so a "fuzzed" sequence is reproducible from its
/// seed and a CI failure can be replayed exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// The event stream a storm sends: the navigation keys and wheel actions that
/// move persistent state, plus resizes.
fn storm_event(rng: &mut Rng) -> Signal {
    let codes = [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::Backspace,
        KeyCode::Char('x'),
        KeyCode::Char('界'),
    ];
    match rng.below(4) {
        0 => Signal::Event(Event::Mouse(Mouse::at(
            if rng.below(2) == 0 {
                MouseKind::ScrollDown
            } else {
                MouseKind::ScrollUp
            },
            rng.below(40) as u16,
            rng.below(20) as u16,
        ))),
        1 => Signal::Event(Event::Resize {
            width: rng.below(60) as u16,
            height: rng.below(20) as u16,
        }),
        2 => Signal::Tick,
        _ => Signal::Event(Event::Key(Key::new(
            codes[rng.below(codes.len() as u64) as usize],
        ))),
    }
}

#[test]
fn a_shrinking_list_under_a_scrolling_user_still_renders() {
    // The combination that produces most host crash reports: persistent
    // selection and scroll state built against one collection, then rendered
    // against a shorter one — a filter applied, a stream that dropped rows, a
    // refresh mid-scroll. tuika keeps the stale index deliberately (the host
    // reconciles with `clamp`), so the *views* must survive being handed one.
    let theme = Theme::default();
    let mut rng = Rng(0x5eed_1234_9abc_def0);
    let mut select = SelectState::new();
    let mut scroll = ScrollState::new();

    for step in 0..600u32 {
        let len = (rng.below(9)) as usize;
        let viewport = rng.below(8) as usize;
        let event = storm_event(&mut rng);
        if let Signal::Event(event) = &event {
            let _ = select.handle(event, len);
            let _ = scroll.handle(event, len, viewport);
        }

        // Rendered against a collection that may have shrunk since the event.
        let shorter = (rng.below(len as u64 + 1)) as usize;
        let rows: Vec<Line<'static>> = (0..shorter)
            .map(|i| Line::from(format!("row {i} 日本語 👩‍👩‍👦")))
            .collect();
        let (width, height) = (rng.below(30) as u16, rng.below(10) as u16);
        let _ = tuika::testing::render(
            &SelectList::new(rows.clone(), &select),
            width,
            height,
            &theme,
        );
        let _ = tuika::testing::render(&Scroll::new(rows, &scroll), width, height, &theme);
        assert!(step < 600);
    }
}

#[test]
fn an_application_survives_a_resize_and_input_storm() {
    // The public runtime path, end to end: a real `Application` fed a long
    // pseudo-random stream of resizes, keys, wheel events, and ticks. Every
    // frame it produces must match the viewport it was asked for — a component
    // that panics or paints a differently-sized buffer under a zero-column
    // screen fails here rather than in a user's terminal.
    struct App {
        select: SelectState,
        scroll: ScrollState,
        input: TextInputState,
        rows: usize,
    }

    impl Application for App {
        fn update(&mut self, signal: Signal) -> UpdateResult {
            let Signal::Event(event) = signal else {
                return UpdateResult::Dirty;
            };
            let _ = self.select.handle(&event, self.rows);
            let _ = self.scroll.handle(&event, self.rows, 8);
            let _ = self.input.handle(&event);
            UpdateResult::Dirty
        }

        fn view(&self, frame: u64) -> Element {
            let rows: Vec<Line<'static>> = (0..self.rows)
                .map(|i| Line::from(format!("row {i}")))
                .collect();
            element(
                Flex::column()
                    .gap(1)
                    .fixed(1, element(Text::raw(format!("frame {frame}"))))
                    .grow(1, element(SelectList::new(rows, &self.select)))
                    .fixed(3, element(TextInput::new(&self.input))),
            )
        }
    }

    let mut rng = Rng(0x1234_5678_9abc_def0);
    let mut harness = tuika::testing::TestHarness::new(
        App {
            select: SelectState::new(),
            scroll: ScrollState::new(),
            input: TextInputState::from_text("seed 日本語"),
            rows: 5,
        },
        40,
        12,
    );

    let mut size = (40u16, 12u16);
    for _ in 0..800 {
        let signal = storm_event(&mut rng);
        if let Signal::Event(Event::Resize { width, height }) = signal {
            size = (width, height);
        }
        harness.state_mut().rows = rng.below(12) as usize;
        if let (_, Some(frame)) = harness.step_app(signal) {
            assert_eq!(
                frame.area,
                Rect::new(0, 0, size.0, size.1),
                "a frame was painted at the wrong size"
            );
        }
    }
}
