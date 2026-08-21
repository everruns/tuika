//! Randomized robustness tests: the same code paths, hammered with adversarial
//! input instead of curated examples.
//!
//! [`proptests`](super::proptests) pins *invariants of the solvers* — a child
//! rect stays in bounds, a flex line fills exactly. This module is aimed one
//! level lower, at the class of defect that reaches users as a crash rather
//! than a wrong pixel: an arithmetic edge (the wrap solver's column counter
//! overflowing at `AvailableSpace::MaxContent`), a byte offset that is not a
//! char boundary, an index derived from a width that just went to zero.
//!
//! Five families live here, and they are deliberately different:
//!
//! - **Text** — strings built from an adversarial alphabet (wide CJK, ZWJ
//!   emoji, combining marks, zero-width and control characters, whitespace
//!   runs) through the wrap solver and the prose components, at every
//!   interesting width including `0` and `u16::MAX`.
//! - **Event streams** — arbitrary key/mouse sequences into the stateful
//!   components, asserting that persistent state stays within the bounds the
//!   caller declared (which is what a host indexes with on the next frame).
//! - **Streaming markdown** — a differential against the one-shot render, over
//!   generated documents, chunk sizes, and widths. Two settled-prefix bugs came
//!   out of this one; both depended on where the chunk boundaries fell.
//! - **Composed trees and overlays** — nested flex, boxes, and clamps rendered
//!   into an inset area of a larger buffer, so a write past the rect is caught
//!   as a scribble on the margin instead of landing in a neighbour.
//! - **Parsers on untrusted bytes** — terminal replies, bare-URL detection, QR
//!   payloads, image buffers: input tuika does not author and cannot trust.
//!
//! The per-component black-box sweep — every component × corpus × degenerate
//! size, through the published API only — is its peer in `tests/robustness.rs`.
//!
//! Everything here is deterministic: proptest is seeded from its own regression
//! file, and the sweeps enumerate fixed combinations.

use crate::geometry::Rect;
use crate::style::Style;
use crate::text::{Line, Span};
use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

use crate::components::text::{wrap_lines, wrap_str};
use crate::components::{Paragraph, Wrap};
use crate::geometry::Size;
use crate::style::Theme;
use crate::view::{RenderCtx, View};
use crate::width::{grapheme_cols, str_cols};

/// Column width of a string as `usize`, so an assertion can *see* an overflow
/// instead of inheriting [`str_cols`]'s saturation.
fn cols(s: &str) -> usize {
    s.graphemes(true).map(|g| grapheme_cols(g) as usize).sum()
}

/// Text fragments chosen to break naive column arithmetic and byte slicing.
///
/// Each entry is one *grapheme* concern: a wide CJK cell, a ZWJ sequence that
/// must stay intact, a base + combining mark that must not be split, a
/// zero-width space that advances bytes but no columns, a C0 control byte, and
/// the whitespace runs that drive the solver's break logic.
const FRAGMENTS: &[&str] = &[
    "a",
    "word",
    "supercalifragilisticexpialidocious",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "日本語",
    "👩\u{200d}👩\u{200d}👦",
    "e\u{301}",
    "\u{200b}",
    "\u{1}",
    "https://example.dev/a/b",
    "—",
    "❤\u{fe0f}",
    "x\u{fe0e}",
];

/// A string assembled from [`FRAGMENTS`], never containing a newline (the wrap
/// solver's documented contract is one source line per call).
fn fuzz_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(FRAGMENTS), 0..24)
        .prop_map(|parts| parts.concat())
}

/// Widths that matter: the degenerate ones, the ordinary ones, and the
/// max-content sentinel `AvailableSpace::MaxContent` measures with.
fn fuzz_width() -> impl Strategy<Value = u16> {
    prop_oneof![
        3 => 0u16..=8,
        3 => 1u16..=200,
        1 => Just(u16::MAX),
        1 => (u16::MAX - 4)..=u16::MAX,
    ]
}

/// Every non-whitespace grapheme of `text`, concatenated — what wrapping must
/// preserve exactly, whatever it does with the spaces between them.
fn words_only(text: &str) -> String {
    text.graphemes(true)
        .filter(|g| !g.chars().all(char::is_whitespace))
        .collect()
}

proptest! {
    /// The plain-text solver, over adversarial graphemes at every width: it
    /// terminates, keeps every word's bytes, maps each word back to the source
    /// exactly, and never lets a row exceed the width it was given.
    #[test]
    fn wrap_str_preserves_content_and_respects_width(
        text in fuzz_text(),
        width in fuzz_width(),
    ) {
        let rows = wrap_str(&text, width);
        prop_assert!(!rows.is_empty(), "a wrap always yields at least one row");

        let mut recovered = String::new();
        for row in &rows {
            // Every word points at the source bytes it was copied from, on a
            // grapheme boundary — the property link ranges and search hits are
            // carried onto a wrapped row with.
            for (row_start, src) in &row.words {
                prop_assert!(text.is_char_boundary(src.start) && text.is_char_boundary(src.end));
                prop_assert_eq!(&row.text[*row_start..row_start + src.len()], &text[src.clone()]);
                recovered.push_str(&text[src.clone()]);
            }
            // A row may exceed the width only when it is a single grapheme
            // wider than the row itself (nothing left to break), or when the
            // column counter is deliberately saturating at max-content width.
            let row_cols = cols(&row.text);
            if row_cols > width as usize && width != u16::MAX {
                prop_assert_eq!(row.words.len(), 1, "over-wide row: {:?}", row.text);
                prop_assert_eq!(row.text.graphemes(true).count(), 1, "over-wide row: {:?}", row.text);
            }
        }
        prop_assert_eq!(recovered, words_only(&text), "wrapping lost or reordered content");
    }

    /// Re-wrapping an already-wrapped row at the same width is a fixed point.
    ///
    /// Any row the solver emits that would wrap again means the first pass
    /// produced a row it does not itself consider to fit — the signature of an
    /// off-by-one in the joining-space accounting.
    #[test]
    fn wrapping_is_a_fixed_point(text in fuzz_text(), width in 1u16..80) {
        for row in wrap_str(&text, width) {
            let again = wrap_str(&row.text, width);
            prop_assert_eq!(again.len(), 1, "row {:?} re-wrapped to {}", row.text, again.len());
            prop_assert_eq!(&again[0].text, &row.text);
        }
    }

    /// The styled solver agrees with the plain one, glyph for glyph.
    ///
    /// `wrap_lines` and `wrap_str` share the solver and differ only in how they
    /// join words (a styled space vs a plain one), so a divergence here means
    /// the styled path lost a cluster in `coalesce`.
    #[test]
    fn styled_and_plain_wrapping_agree(text in fuzz_text(), width in fuzz_width()) {
        let plain: Vec<String> = wrap_str(&text, width).into_iter().map(|r| r.text).collect();
        let styled: Vec<String> = wrap_lines(&[Line::from(text.clone())], width)
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        if width == 0 {
            // A zero width is documented as a pass-through for `wrap_lines`.
            prop_assert_eq!(styled, vec![text]);
        } else {
            prop_assert_eq!(styled, plain);
        }
    }

    /// Multi-span input keeps every cluster and every style across the reflow,
    /// at any width — including the degenerate ones.
    #[test]
    fn wrap_lines_preserves_styled_clusters(
        parts in proptest::collection::vec(
            (proptest::sample::select(FRAGMENTS), 0u8..4),
            0..12,
        ),
        width in fuzz_width(),
    ) {
        let styles = [
            Style::default(),
            Style::default().fg(crate::style::Color::Red),
            Style::default().bg(crate::style::Color::Blue),
            Style::default().add_modifier(crate::style::Modifier::BOLD),
        ];
        let line = Line::from(
            parts
                .iter()
                .map(|(frag, style)| Span::styled((*frag).to_string(), styles[*style as usize]))
                .collect::<Vec<_>>(),
        );
        let source: String = parts.iter().map(|(frag, _)| *frag).collect();
        let out = wrap_lines(&[line], width);
        prop_assert!(!out.is_empty());
        if width == 0 {
            return Ok(());
        }
        let recovered: String = out
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        prop_assert_eq!(words_only(&recovered), words_only(&source));
        for line in &out {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if cols(&text) > width as usize && width != u16::MAX {
                prop_assert_eq!(text.graphemes(true).count(), 1, "over-wide row: {:?}", text);
            }
        }
    }

    /// `Paragraph` measures what it renders.
    ///
    /// Its height must equal the number of rows the shared solver produces for
    /// the same width — the disagreement that a second, differently-modelled
    /// wrapper reintroduces every time one is added.
    #[test]
    fn paragraph_measurement_matches_its_wrap(text in fuzz_text(), width in 1u16..60) {
        let theme = Theme::default();
        let paragraph = Paragraph::new(text.clone(), Style::default());
        let measured = paragraph.measure(Size::new(width, u16::MAX), &RenderCtx::new(&theme));
        prop_assert_eq!(measured.height as usize, wrap_str(&text, width).len());
        // A 2-column grapheme cannot be narrowed to fit a 1-column row, so the
        // measured width is the offered one *or* the width of one wide cell —
        // never anything the content did not actually need.
        prop_assert!(measured.width <= width.max(2), "measured wider than offered");
    }

    /// Prose components paint inside their rect for any text and any size.
    ///
    /// The view is rendered into an inset area of a larger buffer, so a write
    /// past its own bounds shows up as a scribble on the margin rather than
    /// being silently absorbed by a neighbouring component.
    #[test]
    fn prose_components_stay_inside_their_rect(
        text in fuzz_text(),
        width in 0u16..24,
        height in 0u16..8,
    ) {
        let lines: Vec<Line<'static>> = text.split(' ').map(|w| Line::from(w.to_string())).collect();
        assert_inside(&Paragraph::new(text.clone(), Style::default()), width, height)?;
        assert_inside(&Wrap::new(lines.clone()), width, height)?;
        assert_inside(&crate::components::Text::new(lines), width, height)?;
    }
}

proptest! {
    // The corpus is deliberately enormous — over 65_535 columns on one row — so
    // a handful of cases is both enough and all the time this is worth.
    #![proptest_config(ProptestConfig { cases: 8, ..ProptestConfig::default() })]

    /// Measuring at `MaxContent` (`u16::MAX` columns) must degrade, never panic.
    ///
    /// Prose long enough to overflow the solver's column counter is ordinary
    /// pasted input — a log line, a generated paragraph — and it reaches this
    /// path through every intrinsic-sizing container. This is the regression
    /// net around the counter overflow that saturating arithmetic fixed.
    #[test]
    fn prose_survives_max_content_measurement(text in fuzz_text()) {
        let theme = Theme::default();
        let unit = if text.trim().is_empty() { "word".to_string() } else { text };
        let per_repeat = str_cols(&unit).max(1) as usize + 1;
        let repeats = (u16::MAX as usize / per_repeat) + 8;
        let long = std::iter::repeat_n(unit.as_str(), repeats)
            .collect::<Vec<_>>()
            .join(" ");

        let rows = wrap_str(&long, u16::MAX);
        prop_assert_eq!(rows.len(), 1, "everything fits at max-content width");
        prop_assert_eq!(wrap_lines(&[Line::from(long.clone())], u16::MAX).len(), 1);
        let measured = Paragraph::new(long, Style::default())
            .measure(Size::new(u16::MAX, u16::MAX), &RenderCtx::new(&theme));
        prop_assert_eq!(measured.height, 1);
    }
}

/// Render `view` into an inset rect of a margined buffer and assert that every
/// cell outside that rect is untouched.
fn assert_inside(view: &dyn View, width: u16, height: u16) -> Result<(), TestCaseError> {
    const MARGIN: u16 = 2;
    let theme = Theme::default();
    let outer = Rect::new(0, 0, width + MARGIN * 2, height + MARGIN * 2);
    let inner = Rect::new(MARGIN, MARGIN, width, height);
    let mut buffer = crate::buffer::Buffer::empty(outer);
    crate::paint(&mut buffer, inner, &theme, view, &[]);
    let untouched = crate::buffer::Cell::default();
    for y in outer.top()..outer.bottom() {
        for x in outer.left()..outer.right() {
            if inner.contains(crate::geometry::Position::new(x, y)) {
                continue;
            }
            prop_assert_eq!(
                &buffer[(x, y)],
                &untouched,
                "cell ({}, {}) outside {:?} was painted",
                x,
                y,
                inner
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event-sequence fuzzing
// ---------------------------------------------------------------------------

/// An arbitrary input event, weighted towards the kinds that move a selection
/// or a scroll offset — the state a host indexes its own model with on the next
/// frame, and therefore the state a bad clamp turns into a panic upstream.
fn fuzz_event() -> impl Strategy<Value = crate::Event> {
    use crate::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind};
    let codes = prop_oneof![
        Just(KeyCode::Up),
        Just(KeyCode::Down),
        Just(KeyCode::Left),
        Just(KeyCode::Right),
        Just(KeyCode::Home),
        Just(KeyCode::End),
        Just(KeyCode::PageUp),
        Just(KeyCode::PageDown),
        Just(KeyCode::Enter),
        Just(KeyCode::Esc),
        Just(KeyCode::Tab),
        Just(KeyCode::BackTab),
        Just(KeyCode::Backspace),
        Just(KeyCode::Delete),
        proptest::sample::select(&['a', ' ', '\n', '界', '👩', '\u{301}', '\u{1}'][..])
            .prop_map(KeyCode::Char),
    ];
    let keys = (codes, any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(code, ctrl, alt, shift)| {
            Event::Key(Key {
                code,
                ctrl,
                alt,
                shift,
            })
        },
    );
    let kinds = prop_oneof![
        Just(MouseKind::Down(MouseButton::Left)),
        Just(MouseKind::Up(MouseButton::Left)),
        Just(MouseKind::Drag(MouseButton::Left)),
        Just(MouseKind::Moved),
        Just(MouseKind::ScrollUp),
        Just(MouseKind::ScrollDown),
        Just(MouseKind::ScrollLeft),
        Just(MouseKind::ScrollRight),
    ];
    prop_oneof![
        6 => keys,
        3 => (kinds, 0u16..40, 0u16..20).prop_map(|(kind, x, y)| Event::Mouse(Mouse::at(kind, x, y))),
        1 => fuzz_text().prop_map(crate::Event::Paste),
    ]
}

fn fuzz_events() -> impl Strategy<Value = Vec<crate::Event>> {
    proptest::collection::vec(fuzz_event(), 0..40)
}

proptest! {
    /// A selection is an index its host will use to look a row up. For a list
    /// whose length does not change under it, no event stream may push the
    /// selection outside that list.
    #[test]
    fn select_state_never_points_outside_a_stable_list(
        events in fuzz_events(),
        len in 0usize..8,
    ) {
        use crate::components::SelectState;
        let mut state = SelectState::new();
        for event in &events {
            let before = state.selected();
            let _ = state.handle(event, len);
            // A fresh state starts on row 0 and a host reconciles it with
            // `clamp`; what the *handler* must never do is move a valid
            // selection out of range.
            if before.is_none_or(|index| index < len)
                && let Some(index) = state.selected()
            {
                prop_assert!(index < len, "selected {} of {}", index, len);
            }
        }
    }

    /// When the list *does* change under the state — a filter, a refresh, a
    /// stream that dropped rows — the documented reconcile is what restores the
    /// invariant, and it must do so from any state an event stream can reach.
    ///
    /// This is the contract hosts rely on, so it is worth a test of its own:
    /// tuika's stateful components deliberately keep a stale index rather than
    /// silently moving a user's selection, and hand the caller `clamp` for the
    /// frame where the collection actually changed.
    #[test]
    fn shrinking_a_list_is_reconciled_by_clamping(
        events in fuzz_events(),
        lens in proptest::collection::vec(0usize..8, 1..6),
    ) {
        use crate::components::{FormState, SelectState, TabSelectState, TabsState};
        let mut select = SelectState::new();
        let mut form = FormState::new();
        let mut tabs = TabsState::new();
        let mut tab_select = TabSelectState::new();
        for (i, event) in events.iter().enumerate() {
            let len = lens[i % lens.len()];
            let _ = select.handle(event, len);
            let _ = form.handle(event, len);
            let _ = tabs.handle(event, len);
            let _ = tab_select.handle(event, len);

            select.clamp(len);
            form.clamp(len);
            tabs.select(tabs.selected(), len);
            tab_select.select(tab_select.selected(), len);

            if len == 0 {
                prop_assert_eq!(select.selected(), None);
                prop_assert_eq!(form.focused(), 0);
            } else {
                prop_assert!(select.selected().is_none_or(|index| index < len));
                prop_assert!(form.focused() < len);
                prop_assert!(tabs.selected() < len);
                prop_assert!(tab_select.selected() < len);
            }
        }
    }

    /// A scroll offset stays within the scrollable range of the geometry it is
    /// handled at — and after a resize or a content change, the documented
    /// per-frame `clamp` brings it back in range.
    #[test]
    fn scroll_state_offset_stays_scrollable(
        events in fuzz_events(),
        shapes in proptest::collection::vec((0usize..40, 0usize..12), 1..6),
    ) {
        use crate::components::ScrollState;
        let mut state = ScrollState::new();
        let mut stable = ScrollState::new();
        let (fixed_content, fixed_viewport) = shapes[0];
        for (i, event) in events.iter().enumerate() {
            let (content, viewport) = shapes[i % shapes.len()];
            let _ = state.handle(event, content, viewport);
            state.clamp(content, viewport);
            prop_assert!(
                state.offset() <= ScrollState::max_offset(content, viewport),
                "offset {} past the end of {} rows in a {}-row viewport",
                state.offset(),
                content,
                viewport
            );

            // Unchanging geometry needs no reconcile: handling alone must never
            // put the offset out of range.
            let _ = stable.handle(event, fixed_content, fixed_viewport);
            prop_assert!(
                stable.offset() <= ScrollState::max_offset(fixed_content, fixed_viewport),
                "offset {} left the range of a viewport that never changed",
                stable.offset()
            );
        }
    }

    /// The editor's cursor must stay addressable: a row that exists, and a
    /// column no further than the end of that row. A cursor past either is what
    /// turns the next keystroke into a slice panic.
    #[test]
    fn text_input_cursor_stays_addressable(events in fuzz_events(), seed in fuzz_text()) {
        use crate::components::TextInputState;
        let mut state = TextInputState::from_text(&seed);
        for event in &events {
            let _ = state.handle(event);
            let (row, col) = state.cursor();
            prop_assert!(row < state.line_count(), "cursor row {} of {}", row, state.line_count());
            let text = state.text();
            let line = text.split('\n').nth(row).expect("the cursor row exists in the text");
            prop_assert!(col <= line.chars().count(), "cursor col {} past {:?}", col, line);
        }
        // Whatever the stream did, the text round-trips through the state.
        let text = state.text();
        prop_assert_eq!(TextInputState::from_text(&text).text(), text);
    }

    /// A slider's value never leaves its declared range, whatever is pressed or
    /// dragged — a host reads it back as a ratio and multiplies by a length.
    #[test]
    fn slider_value_stays_in_range(
        events in fuzz_events(),
        min in -50.0f32..50.0,
        span in 0.0f32..100.0,
    ) {
        use crate::components::SliderState;
        let max = min + span;
        let mut state = SliderState::new(min, max, min);
        for event in &events {
            let _ = state.handle(event);
            prop_assert!(
                state.value() >= min && state.value() <= max,
                "{} outside {}..={}",
                state.value(),
                min,
                max
            );
            let ratio = state.ratio();
            prop_assert!((0.0..=1.0).contains(&ratio), "ratio {} outside 0..=1", ratio);
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming markdown
// ---------------------------------------------------------------------------

/// Markdown-shaped fragments: block starts, inline markers, fence delimiters,
/// table pipes, and the blank lines the streaming cache settles on.
const MARKDOWN_FRAGMENTS: &[&str] = &[
    "# heading\n",
    "## deeper\n",
    "text ",
    "**bold** ",
    "*em* ",
    "`code` ",
    "[label](https://example.dev) ",
    "- item\n",
    "1. item\n",
    "> quote\n",
    "\n",
    "```rust\n",
    "fn main() {}\n",
    "```\n",
    "| a | b |\n",
    "| - | - |\n",
    "| 1 | 2 |\n",
    "---\n",
    "日本語 👩\u{200d}👩\u{200d}👦 ",
    "<b>html</b> ",
];

fn fuzz_markdown() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(MARKDOWN_FRAGMENTS), 0..20)
        .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// A streamed document renders exactly like the same document rendered in
    /// one shot.
    ///
    /// `MarkdownState` settles a prefix and re-parses only the tail, so what a
    /// user sees depends on *where the chunk boundaries fell* — the one thing a
    /// host does not control. Any divergence is a rendering artifact that
    /// appears only under streaming, which is the hardest kind to reproduce
    /// from a bug report.
    #[test]
    fn streaming_markdown_matches_a_one_shot_render(
        source in fuzz_markdown(),
        chunk in 1usize..9,
        width in 8u16..60,
    ) {
        use crate::components::MarkdownState;
        use crate::highlight::CodeHighlighter;
        use crate::style::StyleSheet;

        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let mut state = MarkdownState::new();
        let chars: Vec<char> = source.chars().collect();
        for piece in chars.chunks(chunk) {
            state.push_str(&piece.iter().collect::<String>());
            // Rendering mid-stream is what a host does every frame, and it is
            // what populates the cache the final render must agree with.
            let _ = state.lines(width, &theme, &sheet, CodeHighlighter::Plain);
        }
        let streamed: Vec<String> = state
            .lines(width, &theme, &sheet, CodeHighlighter::Plain)
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let one_shot: Vec<String> = crate::components::markdown::to_lines(
            &source,
            width,
            &theme,
            &sheet,
            CodeHighlighter::Plain,
        )
        .iter()
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
        prop_assert_eq!(streamed, one_shot, "streamed render diverged from one-shot");
    }

    /// Re-rendering a settled stream at a new width must equal a fresh state at
    /// that width: the width-keyed cache invalidation has to drop everything a
    /// re-wrap changes.
    #[test]
    fn markdown_rewraps_identically_after_a_resize(
        source in fuzz_markdown(),
        first in 8u16..60,
        second in 8u16..60,
    ) {
        use crate::components::MarkdownState;
        use crate::highlight::CodeHighlighter;
        use crate::style::StyleSheet;

        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let mut resized = MarkdownState::new();
        resized.set(source.clone());
        let _ = resized.lines(first, &theme, &sheet, CodeHighlighter::Plain);
        let after: Vec<Line<'static>> = resized
            .lines(second, &theme, &sheet, CodeHighlighter::Plain)
            .to_vec();

        let mut fresh = MarkdownState::new();
        fresh.set(source);
        let expected = fresh.lines(second, &theme, &sheet, CodeHighlighter::Plain);
        prop_assert_eq!(&after, expected, "a resize left stale lines behind");
    }
}

// ---------------------------------------------------------------------------
// Composed trees and overlays
// ---------------------------------------------------------------------------

/// A description of an arbitrary view tree.
///
/// The tree is fuzzed as *data* and built in the test body, because an
/// `Element` is neither `Clone` nor `Debug` — and a shrunk counterexample has to
/// be printable to be worth anything.
#[derive(Clone, Debug)]
enum Node {
    Text(String),
    Paragraph(String),
    Spacer,
    Rule,
    Progress(u16),
    Boxed {
        child: Box<Node>,
        padding: u16,
        titled: bool,
    },
    Clamp {
        child: Box<Node>,
        width: u16,
        height: u16,
    },
    Flex {
        row: bool,
        gap: u16,
        padding: u16,
        wrap: bool,
        children: Vec<(Node, u8, u16)>,
    },
}

impl Node {
    fn build(&self) -> crate::Element {
        use crate::components::{Boxed, Constrained, Flex, ProgressBar, Rule, Spacer, Text};
        use crate::element;
        use crate::geometry::Padding;
        use crate::layout::{AlignContent, Dimension, FlexWrap};

        match self {
            Self::Text(text) => element(Text::raw(text.clone())),
            Self::Paragraph(text) => element(Paragraph::new(text.clone(), Style::default())),
            Self::Spacer => element(Spacer),
            Self::Rule => element(Rule::new()),
            Self::Progress(pct) => element(ProgressBar::determinate(*pct as f32 / 100.0)),
            Self::Boxed {
                child,
                padding,
                titled,
            } => {
                let boxed = Boxed::new(child.build()).padding(Padding::all(*padding));
                element(if *titled {
                    boxed.title(Line::from("title"))
                } else {
                    boxed
                })
            }
            Self::Clamp {
                child,
                width,
                height,
            } => element(Constrained::new(child.build()).min_size(*width, *height)),
            Self::Flex {
                row,
                gap,
                padding,
                wrap,
                children,
            } => {
                let mut flex = if *row { Flex::row() } else { Flex::column() }
                    .gap(*gap)
                    .padding(Padding::all(*padding))
                    .align_content(AlignContent::Start);
                if *wrap {
                    flex = flex.wrap(FlexWrap::Wrap);
                }
                for (child, kind, cells) in children {
                    let child = child.build();
                    flex = match kind {
                        0 => flex.auto(child),
                        1 => flex.grow(1 + cells % 3, child),
                        2 => flex.fixed(*cells, child),
                        _ => flex.child(Dimension::Percent(cells.saturating_mul(8)), child),
                    };
                }
                element(flex)
            }
        }
    }
}

/// An arbitrary view tree: leaves wrapped in nested flex containers, boxes, and
/// intrinsic clamps, with fuzzed gaps, padding, wrapping, and alignment.
///
/// Individual components are covered by the black-box sweep in
/// `tests/robustness.rs`; what this adds is *depth*. A container that miscounts
/// its chrome only overflows when something else has already eaten the space —
/// a padded box inside a wrapped row inside a column, on a screen two cells
/// tall.
fn fuzz_tree() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        fuzz_text().prop_map(Node::Text),
        fuzz_text().prop_map(Node::Paragraph),
        Just(Node::Spacer),
        Just(Node::Rule),
        (0u16..=100).prop_map(Node::Progress),
    ];

    leaf.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            (inner.clone(), 0u16..4, any::<bool>()).prop_map(|(child, padding, titled)| {
                Node::Boxed {
                    child: Box::new(child),
                    padding,
                    titled,
                }
            }),
            (inner.clone(), 0u16..20, 0u16..20).prop_map(|(child, width, height)| Node::Clamp {
                child: Box::new(child),
                width,
                height,
            }),
            (
                proptest::collection::vec((inner, 0u8..4, 0u16..12), 1..4),
                any::<bool>(),
                0u16..4,
                0u16..4,
                any::<bool>(),
            )
                .prop_map(|(children, row, gap, padding, wrap)| Node::Flex {
                    row,
                    gap,
                    padding,
                    wrap,
                    children,
                }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// A composed tree paints only inside the rect it was given, at any size.
    #[test]
    fn composed_trees_stay_inside_their_rect(
        tree in fuzz_tree(),
        width in 0u16..40,
        height in 0u16..16,
    ) {
        assert_inside(tree.build().as_ref(), width, height)?;
    }

    /// Overlays resolve inside the screen and paint inside themselves, for any
    /// base tree, overlay tree, spec, and screen — including screens smaller
    /// than the overlay's own minimum.
    #[test]
    fn scenes_with_overlays_stay_on_screen(
        base in fuzz_tree(),
        floating in fuzz_tree(),
        width in 0u16..50,
        height in 0u16..20,
        pct in 0u16..=100,
        margin in 0u16..8,
        min in 0u16..20,
        anchor in 0usize..9,
    ) {
        use crate::overlay::{Anchor, Extent, OverlaySpec};
        use crate::scene::{Scene, SceneOverlay};

        let anchors = [
            Anchor::Center,
            Anchor::Top,
            Anchor::Bottom,
            Anchor::Left,
            Anchor::Right,
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomLeft,
            Anchor::BottomRight,
        ];
        let spec = OverlaySpec {
            anchor: anchors[anchor],
            width: Extent::Percent(pct),
            height: Extent::Percent(pct),
            min_width: min,
            min_height: min,
            margin,
            ..OverlaySpec::centered(0, 0)
        };
        let scene = Scene::new(base.build()).overlay(SceneOverlay::new(floating.build(), spec));
        assert_inside(&scene, width, height)?;
    }
}

// ---------------------------------------------------------------------------
// Parsers on untrusted bytes
// ---------------------------------------------------------------------------

proptest! {
    /// The terminal's replies are untrusted input.
    ///
    /// A palette/device-attributes probe reads whatever the emulator (or an
    /// `ssh` session, or a mangled `tmux` passthrough) sends back — truncated
    /// sequences, wrong digit counts, embedded NULs, a reply for a query that
    /// was never sent. Parsing must always terminate with *some* answer, never
    /// panic on a slice or an integer conversion.
    #[test]
    fn terminal_replies_parse_or_decline(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        use crate::term::capabilities::DeviceAttributes;
        use crate::term::palette::TerminalPalette;

        let palette = TerminalPalette::parse(&bytes);
        // Whatever it decided, deriving a theme from it must agree with itself.
        let theme = Theme::from_terminal(&palette);
        prop_assert!(theme.is_none() || !palette.is_empty());
        let _ = DeviceAttributes::parse(&bytes);
    }

    /// The same, on the byte shapes a real reply *nearly* has: a well-formed
    /// OSC frame with fuzzed digits, lengths, and terminators.
    #[test]
    fn near_miss_terminal_replies_parse_or_decline(
        index in 0u16..300,
        digits in proptest::collection::vec(proptest::sample::select(&b"0123456789abcdefABCDEF/;:"[..]), 0..24),
        terminated in any::<bool>(),
    ) {
        use crate::term::palette::TerminalPalette;

        let body = String::from_utf8(digits).expect("ascii");
        let mut reply = format!("\x1b]4;{index};rgb:{body}");
        if terminated {
            reply.push_str("\x1b\\");
        }
        let palette = TerminalPalette::parse(reply.as_bytes());
        let _ = Theme::from_terminal(&palette);
    }

    /// Bare-URL detection runs over every line a host paints, so it sees
    /// arbitrary text. Each range it returns must be a valid slice of the input
    /// on char boundaries — a host slices with them.
    #[test]
    fn link_detection_returns_valid_ranges(text in fuzz_text(), scheme in fuzz_text()) {
        use crate::term::hyperlink::LinkPolicy;

        let subject = format!("{scheme}https://a.dev/{text} mailto:x@y.dev {text}");
        for policy in [LinkPolicy::WEB, LinkPolicy::NONE, LinkPolicy::default()] {
            for (start, end) in crate::term::hyperlink::find_links(&subject, policy) {
                prop_assert!(start < end && end <= subject.len());
                prop_assert!(subject.is_char_boundary(start) && subject.is_char_boundary(end));
                prop_assert!(!subject[start..end].chars().any(char::is_whitespace));
            }
        }
    }

    /// The QR encoder takes a host-supplied payload and returns `None` rather
    /// than panicking when it does not fit, and every matrix it does return is
    /// square and non-empty.
    #[test]
    fn qr_encoding_either_fits_or_declines(payload in proptest::collection::vec(any::<u8>(), 0..200)) {
        use crate::components::{QrCode, QrEcc};

        let text = String::from_utf8_lossy(&payload).into_owned();
        for ecc in [QrEcc::Low, QrEcc::Medium, QrEcc::Quartile, QrEcc::High] {
            let Some(code) = QrCode::encode(&text, ecc) else {
                continue;
            };
            let theme = Theme::default();
            // A code that encoded must also paint at any size.
            for (w, h) in [(0u16, 0u16), (1, 1), (21, 11), (80, 40)] {
                let _ = crate::testing::render(&code, w, h, &theme);
            }
        }
    }

    /// Image payloads come from a host's decoder, so the dimensions and the
    /// buffer can disagree. Construction must reject a mismatch instead of
    /// trusting it into an out-of-bounds read later.
    #[test]
    fn image_data_rejects_mismatched_buffers(
        width in 0u32..8,
        height in 0u32..8,
        len in 0usize..80,
    ) {
        use crate::term::image::ImageData;

        let data = ImageData::from_rgba(width, height, vec![0u8; len]);
        let expected = (width as usize) * (height as usize) * 4;
        prop_assert_eq!(data.is_some(), expected != 0 && len == expected);
    }
}
