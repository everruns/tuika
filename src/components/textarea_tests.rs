// Many steps drive state for its side effects and assert on it afterwards.
#![allow(unused_must_use)]

use super::*;
use crate::buffer::Buffer;
use crate::event::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind};
use crate::style::{Color, Theme};
use crate::tests::support::{rainbow_theme, row};
use crate::text::Span;

fn key(code: KeyCode) -> Event {
    Event::Key(Key::new(code))
}

fn chord(code: KeyCode, ctrl: bool, alt: bool, shift: bool) -> Event {
    Event::Key(Key {
        code,
        ctrl,
        alt,
        shift,
    })
}

fn shift(code: KeyCode) -> Event {
    chord(code, false, false, true)
}

fn ctrl(code: KeyCode) -> Event {
    chord(code, true, false, false)
}

fn type_str(state: &mut TextAreaState, text: &str) {
    for ch in text.chars() {
        assert_eq!(state.handle(&key(KeyCode::Char(ch))), InputOutcome::Changed);
    }
}

fn mouse(kind: MouseKind, column: u16, row: u16) -> Event {
    Event::Mouse(Mouse::at(kind, column, row))
}

/// Render `view` at `area` inside a larger buffer, returning the buffer.
fn paint(view: &TextArea<'_>, theme: &Theme, area: Rect, outer: Rect) -> Buffer {
    let mut buffer = Buffer::empty(outer);
    let ctx = RenderCtx::new(theme);
    let mut surface = Surface::new(&mut buffer, area);
    view.render(area, &mut surface, &ctx);
    buffer
}

fn rows(state: &TextAreaState, width: u16, height: u16) -> Vec<String> {
    let area = Rect::new(0, 0, width, height);
    let buffer = paint(&TextArea::new(state), &Theme::default(), area, area);
    (0..height).map(|y| row(&buffer, y)).collect()
}

#[test]
fn typing_undoes_a_word_at_a_time_and_redoes() {
    let mut state = TextAreaState::new();
    type_str(&mut state, "hello big world");
    assert_eq!(state.text(), "hello big world");

    assert_eq!(
        state.handle(&ctrl(KeyCode::Char('z'))),
        InputOutcome::Changed
    );
    assert_eq!(state.text(), "hello big ");
    state.undo();
    assert_eq!(state.text(), "hello ");
    state.undo();
    assert_eq!(state.text(), "");
    assert!(!state.can_undo());
    assert_eq!(
        state.handle(&ctrl(KeyCode::Char('z'))),
        InputOutcome::Consumed
    );

    assert_eq!(
        state.handle(&ctrl(KeyCode::Char('y'))),
        InputOutcome::Changed
    );
    assert_eq!(state.text(), "hello ");
    // Ctrl+Shift+Z redoes too, whichever case the terminal reports.
    state.handle(&chord(KeyCode::Char('Z'), true, false, true));
    assert_eq!(state.text(), "hello big ");
    assert_eq!(state.cursor(), (0, 10));

    // A new edit drops the redo branch.
    type_str(&mut state, "x");
    assert!(!state.can_redo());
}

#[test]
fn moving_the_cursor_ends_a_typing_group() {
    let mut state = TextAreaState::new();
    type_str(&mut state, "ab");
    state.handle(&key(KeyCode::Left));
    type_str(&mut state, "c");
    assert_eq!(state.text(), "acb");
    state.undo();
    assert_eq!(state.text(), "ab");
    assert_eq!(state.cursor(), (0, 1));
}

#[test]
fn backspace_runs_group_and_join_lines_undo() {
    let mut state = TextAreaState::from_text("one\ntwo");
    state.set_cursor(1, 0);
    state.handle(&key(KeyCode::Backspace));
    assert_eq!(state.text(), "onetwo");
    state.handle(&key(KeyCode::Backspace));
    state.handle(&key(KeyCode::Backspace));
    assert_eq!(state.text(), "otwo");
    state.undo();
    assert_eq!(state.text(), "one\ntwo");
    assert_eq!(state.cursor(), (1, 0));
}

#[test]
fn shift_motion_selects_and_typing_replaces_it() {
    let mut state = TextAreaState::from_text("hello world");
    state.set_cursor(0, 0);
    for _ in 0..5 {
        state.handle(&shift(KeyCode::Right));
    }
    assert_eq!(state.selection(), Some(((0, 0), (0, 5))));
    assert_eq!(state.selected_text().as_deref(), Some("hello"));

    type_str(&mut state, "Jo");
    assert_eq!(state.text(), "Jo world");
    assert_eq!(state.selection(), None);

    // Undo restores the text *and* the selection it replaced.
    state.undo();
    assert_eq!(state.text(), "hello world");
    assert_eq!(state.selection(), Some(((0, 0), (0, 5))));
}

#[test]
fn plain_left_right_collapse_a_selection_to_its_edge() {
    let mut state = TextAreaState::from_text("abcdef");
    state.set_selection((0, 1), (0, 4));
    state.handle(&key(KeyCode::Left));
    assert_eq!((state.cursor(), state.selection()), ((0, 1), None));
    state.set_selection((0, 4), (0, 1));
    state.handle(&key(KeyCode::Right));
    assert_eq!((state.cursor(), state.selection()), ((0, 4), None));
}

#[test]
fn multi_line_selection_text_cut_and_paste() {
    let mut state = TextAreaState::from_text("alpha\nbeta\ngamma");
    state.set_selection((0, 2), (2, 3));
    assert_eq!(state.selected_text().as_deref(), Some("pha\nbeta\ngam"));
    assert_eq!(state.cut().as_deref(), Some("pha\nbeta\ngam"));
    assert_eq!(state.text(), "alma");
    assert_eq!(state.cursor(), (0, 2));
    assert_eq!(state.cut(), None);

    state.select_all();
    assert_eq!(
        state.handle(&Event::Paste("x\ny".into())),
        InputOutcome::Changed
    );
    assert_eq!(state.text(), "x\ny");
    state.undo();
    assert_eq!(state.text(), "alma");
    state.undo();
    assert_eq!(state.text(), "alpha\nbeta\ngamma");
}

#[test]
fn esc_clears_a_selection_and_is_ignored_without_one() {
    let mut state = TextAreaState::from_text("abc");
    state.select_all();
    assert_eq!(state.handle(&key(KeyCode::Esc)), InputOutcome::Changed);
    assert_eq!(state.selection(), None);
    assert_eq!(state.handle(&key(KeyCode::Esc)), InputOutcome::Ignored);
    assert_eq!(state.handle(&key(KeyCode::Tab)), InputOutcome::Ignored);
}

#[test]
fn word_motions_stop_at_class_and_line_boundaries() {
    let mut state = TextAreaState::from_text("foo.bar  baz\nnext");
    state.set_cursor(0, 0);
    let mut stops = Vec::new();
    for _ in 0..5 {
        state.handle(&ctrl(KeyCode::Right));
        stops.push(state.cursor());
    }
    assert_eq!(stops, [(0, 3), (0, 4), (0, 7), (0, 12), (1, 0)]);

    let mut back = Vec::new();
    for _ in 0..5 {
        state.handle(&chord(KeyCode::Left, false, true, false));
        back.push(state.cursor());
    }
    assert_eq!(back, [(0, 12), (0, 9), (0, 4), (0, 3), (0, 0)]);
}

#[test]
fn word_motion_never_splits_a_grapheme() {
    // "e" + combining acute: the mark is not alphanumeric, but the stop must
    // land after the whole cluster.
    let mut state = TextAreaState::from_text("cafe\u{301} ok");
    state.set_cursor(0, 0);
    state.move_word_right(false);
    assert_eq!(state.cursor(), (0, 5));
}

#[test]
fn shift_word_motion_extends_and_word_delete_is_undoable() {
    let mut state = TextAreaState::from_text("one two three");
    state.set_cursor(0, 0);
    state.handle(&chord(KeyCode::Right, true, false, true));
    state.handle(&chord(KeyCode::Right, true, false, true));
    assert_eq!(state.selected_text().as_deref(), Some("one two"));

    let mut state = TextAreaState::from_text("one two three");
    state.handle(&chord(KeyCode::Backspace, true, false, false));
    assert_eq!(state.text(), "one two ");
    state.set_cursor(0, 0);
    state.handle(&chord(KeyCode::Delete, true, false, false));
    assert_eq!(state.text(), " two ");
    state.undo();
    state.undo();
    assert_eq!(state.text(), "one two three");
}

#[test]
fn readline_keys_still_work() {
    let mut state = TextAreaState::from_text("hello world");
    state.handle(&ctrl(KeyCode::Char('a')));
    assert_eq!(state.cursor(), (0, 0));
    state.handle(&ctrl(KeyCode::Char('k')));
    assert_eq!(state.text(), "");
    state.undo();
    assert_eq!(state.text(), "hello world");
    state.handle(&ctrl(KeyCode::Char('e')));
    state.handle(&ctrl(KeyCode::Char('w')));
    assert_eq!(state.text(), "hello ");
}

#[test]
fn enter_follows_the_mode() {
    let mut state = TextAreaState::from_text("hi");
    assert_eq!(state.handle(&key(KeyCode::Enter)), InputOutcome::Submitted);
    assert_eq!(state.text(), "hi");
    assert_eq!(state.handle(&shift(KeyCode::Enter)), InputOutcome::Changed);
    assert_eq!(state.text(), "hi\n");
    state.set_mode(super::super::TextInputMode::SubmitOnShiftEnter);
    assert_eq!(state.handle(&key(KeyCode::Enter)), InputOutcome::Changed);
    assert_eq!(
        state.handle(&shift(KeyCode::Enter)),
        InputOutcome::Submitted
    );
}

#[test]
fn movement_at_a_bound_is_consumed() {
    let mut state = TextAreaState::from_text("ab");
    state.set_cursor(0, 0);
    assert_eq!(state.handle(&key(KeyCode::Left)), InputOutcome::Consumed);
    assert_eq!(
        state.handle(&key(KeyCode::Backspace)),
        InputOutcome::Consumed
    );
    assert!(!state.can_undo());
}

#[test]
fn up_down_move_by_wrapped_row_and_keep_the_goal_column() {
    // At width 10, "aaaa bbbb cccc" wraps to "aaaa bbbb " / "cccc".
    let mut state = TextAreaState::from_text("aaaa bbbb cccc\nxy\nlong line here");
    state.set_cursor(0, 7);
    rows(&state, 10, 6);
    state.handle(&key(KeyCode::Down));
    assert_eq!(
        state.cursor(),
        (0, 14),
        "second wrapped row, clamped to its end"
    );
    state.handle(&key(KeyCode::Down));
    assert_eq!(state.cursor(), (1, 2), "clamped on the short line");
    state.handle(&key(KeyCode::Down));
    assert_eq!(state.cursor(), (2, 7), "goal column restored");
    state.handle(&key(KeyCode::Up));
    state.handle(&key(KeyCode::Up));
    assert_eq!(state.cursor(), (0, 14));
    state.handle(&key(KeyCode::Up));
    assert_eq!(state.cursor(), (0, 7), "first wrapped row, goal column");
    // Past the top goes to the buffer start, past the bottom to its end.
    state.handle(&key(KeyCode::Up));
    assert_eq!(state.cursor(), (0, 0));
    state.handle(&ctrl(KeyCode::End));
    assert_eq!(state.cursor(), (2, 14));
    state.handle(&ctrl(KeyCode::Home));
    assert_eq!(state.cursor(), (0, 0));
}

#[test]
fn vertical_motion_before_any_render_moves_by_logical_line() {
    let mut state = TextAreaState::from_text("abc\nde\nfghij");
    state.set_cursor(0, 3);
    state.move_down(false);
    assert_eq!(state.cursor(), (1, 2));
    state.move_down(false);
    assert_eq!(state.cursor(), (2, 3));
}

#[test]
fn page_keys_move_a_screen_and_the_view_follows() {
    let text: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let mut state = TextAreaState::from_text(&text.join("\n"));
    state.set_cursor(0, 0);
    rows(&state, 20, 5);
    state.handle(&key(KeyCode::PageDown));
    assert_eq!(state.cursor(), (4, 0));
    state.handle(&shift(KeyCode::PageDown));
    assert_eq!(state.cursor(), (8, 0));
    assert_eq!(
        state.selected_text().as_deref(),
        Some("line 4\nline 5\nline 6\nline 7\n")
    );
    let shown = rows(&state, 20, 5);
    assert_eq!(shown[4], "line 8");
    assert_eq!(state.scroll_top(), 4);
    assert_eq!(state.cursor_screen(), Some((0, 4)));
}

#[test]
fn renders_and_scrolls_to_the_cursor() {
    let mut state = TextAreaState::from_text("a\nb\nc\nd\ne");
    assert_eq!(rows(&state, 5, 3), ["c", "d", "e"]);
    assert_eq!(state.cursor_screen(), Some((1, 2)));
    state.handle(&ctrl(KeyCode::Home));
    assert_eq!(rows(&state, 5, 3), ["a", "b", "c"]);
    // Moving within the view does not scroll it.
    state.handle(&key(KeyCode::Down));
    assert_eq!(rows(&state, 5, 3), ["a", "b", "c"]);
    assert_eq!(state.cursor_screen(), Some((0, 1)));
}

#[test]
fn mouse_click_drag_shift_click_and_wheel() {
    let text: Vec<String> = (0..10).map(|i| format!("row {i} text")).collect();
    let mut state = TextAreaState::from_text(&text.join("\n"));
    state.set_cursor(0, 0);
    let area = Rect::new(2, 1, 20, 4);
    let outer = Rect::new(0, 0, 30, 8);
    paint(&TextArea::new(&state), &Theme::default(), area, outer);

    // Click on row 1, column 4 of the text.
    let click = mouse(MouseKind::Down(MouseButton::Left), 6, 2);
    assert_eq!(state.handle(&click), InputOutcome::Changed);
    assert_eq!((state.cursor(), state.selection()), ((1, 4), None));

    // Drag to row 2, column 3 selects the range.
    state.handle(&mouse(MouseKind::Drag(MouseButton::Left), 5, 3));
    assert_eq!(state.selected_text().as_deref(), Some("1 text\nrow"));
    state.handle(&mouse(MouseKind::Up(MouseButton::Left), 5, 3));
    assert_eq!(state.selection(), Some(((1, 4), (2, 3))));

    // Shift+click extends from the existing anchor.
    let mut extend = Mouse::at(MouseKind::Down(MouseButton::Left), 2, 1);
    extend.shift = true;
    state.handle(&Event::Mouse(extend));
    assert_eq!(state.selection(), Some(((0, 0), (1, 4))));

    // A click past a line's end lands at the end; outside the area is ignored.
    state.handle(&mouse(MouseKind::Down(MouseButton::Left), 21, 4));
    assert_eq!(state.cursor(), (3, 10));
    assert_eq!(
        state.handle(&mouse(MouseKind::Down(MouseButton::Left), 0, 0)),
        InputOutcome::Ignored
    );

    // The wheel scrolls without moving the cursor, and the view stays put.
    state.handle(&mouse(MouseKind::ScrollDown, 5, 2));
    assert_eq!(state.scroll_top(), 3);
    let buffer = paint(&TextArea::new(&state), &Theme::default(), area, outer);
    assert_eq!(row(&buffer, 1), "  row 3 text");
    assert_eq!(state.cursor(), (3, 10));
    state.handle(&mouse(MouseKind::ScrollDown, 5, 2));
    state.handle(&mouse(MouseKind::ScrollDown, 5, 2));
    assert_eq!(state.scroll_top(), 6, "clamped to the last screen");
    assert_eq!(state.cursor_screen(), None, "cursor scrolled out of view");
    // Typing brings the cursor back into view.
    type_str(&mut state, "!");
    paint(&TextArea::new(&state), &Theme::default(), area, outer);
    assert_eq!(state.scroll_top(), 3);
}

#[test]
fn click_past_a_soft_wrapped_row_stays_on_that_row() {
    let mut state = TextAreaState::from_text("aaaa bbbb cccc");
    rows(&state, 10, 3);
    state.handle(&mouse(MouseKind::Down(MouseButton::Left), 9, 0));
    // Row 0 is "aaaa bbbb " (10 cells): the click lands on its last cell.
    assert_eq!(state.cursor(), (0, 9));
    state.handle(&mouse(MouseKind::Down(MouseButton::Left), 8, 1));
    assert_eq!(state.cursor(), (0, 14));
}

#[test]
fn selection_and_gutter_use_their_theme_slots() {
    let theme = rainbow_theme();
    let mut state = TextAreaState::from_text("ab\n\ncd");
    state.set_selection((0, 1), (2, 1));
    let area = Rect::new(0, 0, 8, 3);
    let view = TextArea::new(&state).line_numbers(true);
    let buffer = paint(&view, &theme, area, area);
    assert_eq!(row(&buffer, 0), "1 ab");
    assert_eq!(row(&buffer, 1), "2");
    assert_eq!(row(&buffer, 2), "3 cd");

    let gutter = &buffer[(0, 0)];
    assert_eq!(gutter.fg, theme.dim);
    let plain = &buffer[(2, 0)];
    assert_ne!(plain.bg, theme.selection_bg);
    for (x, y) in [(3, 0), (4, 0), (2, 1), (2, 2)] {
        let cell = &buffer[(x, y)];
        assert_eq!(
            (cell.fg, cell.bg),
            (theme.selection_fg, theme.selection_bg),
            "({x},{y})"
        );
    }
    // The selection ends before "d", and nothing past the end is painted.
    assert_ne!(buffer[(3, 2)].bg, theme.selection_bg);
    assert_ne!(buffer[(4, 2)].bg, theme.selection_bg);
}

#[test]
fn custom_styles_override_the_theme() {
    let theme = rainbow_theme();
    let mut state = TextAreaState::from_text("ab");
    state.set_selection((0, 0), (0, 1));
    let area = Rect::new(0, 0, 6, 1);
    let view = TextArea::new(&state)
        .line_numbers(true)
        .gutter_style(Style::default().fg(Color::Red))
        .selection_style(Style::default().bg(Color::Blue))
        .style(Style::default().fg(Color::Green));
    let buffer = paint(&view, &theme, area, area);
    assert_eq!(buffer[(0, 0)].fg, Color::Red);
    assert_eq!(
        (buffer[(2, 0)].fg, buffer[(2, 0)].bg),
        (Color::Green, Color::Blue)
    );
    assert_eq!(buffer[(3, 0)].fg, Color::Green);
}

struct UpperIsKeyword;

impl Highlighter for UpperIsKeyword {
    fn highlight(
        &self,
        lang: &str,
        lines: &[&str],
        _theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>> {
        (lang == "test").then(|| {
            lines
                .iter()
                .map(|line| {
                    line.chars()
                        .map(|c| {
                            let style = if c.is_uppercase() {
                                Style::default().fg(Color::Magenta)
                            } else {
                                Style::default()
                            };
                            Span::styled(c.to_string(), style)
                        })
                        .collect()
                })
                .collect()
        })
    }
}

#[test]
fn highlighter_colours_text_and_host_spans_and_selection_layer_on_top() {
    let theme = Theme::default();
    let mut state = TextAreaState::from_text("aBcD");
    state.set_selection((0, 3), (0, 4));
    let area = Rect::new(0, 0, 6, 1);
    let view = TextArea::new(&state)
        .highlighter(&UpperIsKeyword, "test")
        .highlights(vec![TextSpan::new(
            0,
            2,
            3,
            Style::default().fg(Color::Cyan),
        )]);
    let buffer = paint(&view, &theme, area, area);
    assert_eq!(buffer[(1, 0)].fg, Color::Magenta);
    assert_ne!(buffer[(0, 0)].fg, Color::Magenta);
    assert_eq!(buffer[(2, 0)].fg, Color::Cyan);
    assert_eq!(buffer[(3, 0)].bg, theme.selection_bg);

    // An unknown language falls back to plain text.
    let view = TextArea::new(&state).highlighter(&UpperIsKeyword, "other");
    let buffer = paint(&view, &theme, area, area);
    assert_ne!(buffer[(1, 0)].fg, Color::Magenta);
}

#[test]
fn placeholder_shows_while_empty() {
    let state = TextAreaState::new();
    let area = Rect::new(0, 0, 12, 2);
    let view = TextArea::new(&state)
        .line_numbers(true)
        .placeholder("Ask…", Style::default());
    let buffer = paint(&view, &Theme::default(), area, area);
    assert_eq!(row(&buffer, 0), "1 Ask…");
    assert_eq!(state.cursor_screen(), Some((2, 0)));
}

#[test]
fn measure_counts_wrapped_rows_beside_the_gutter() {
    let state = TextAreaState::from_text("aaaa bbbb\nc");
    let theme = Theme::default();
    let ctx = RenderCtx::new(&theme);
    let plain = TextArea::new(&state).measure(Size::new(5, 10), &ctx);
    assert_eq!(plain.height, 3);
    let numbered = TextArea::new(&state)
        .line_numbers(true)
        .measure(Size::new(7, 10), &ctx);
    assert_eq!(numbered.height, 3);
}

#[test]
fn degenerate_sizes_never_panic_or_paint_outside_the_area() {
    let mut state = TextAreaState::from_text("wide 世界 e\u{301} 👩‍💻 text\n\nmore lines here");
    state.set_selection((0, 2), (2, 4));
    let theme = Theme::default();
    let outer = Rect::new(0, 0, 14, 8);
    for width in 0..=10u16 {
        for height in 0..=5u16 {
            let area = Rect::new(2, 1, width, height);
            for numbers in [false, true] {
                let view = TextArea::new(&state)
                    .line_numbers(numbers)
                    .placeholder("p", Style::default());
                let buffer = paint(&view, &theme, area, outer);
                for y in outer.y..outer.bottom() {
                    for x in outer.x..outer.right() {
                        let inside =
                            x >= area.x && x < area.right() && y >= area.y && y < area.bottom();
                        if !inside {
                            assert_eq!(buffer[(x, y)].symbol(), " ", "{width}x{height} ({x},{y})");
                        }
                    }
                }
                if let Some((x, y)) = state.cursor_screen() {
                    assert!(x < area.right() && y < area.bottom(), "{width}x{height}");
                }
            }
            // Input against a degenerate frame must not panic either.
            let mut probe = state.clone();
            for event in [
                key(KeyCode::Down),
                key(KeyCode::PageDown),
                key(KeyCode::Up),
                mouse(MouseKind::Down(MouseButton::Left), 3, 2),
                mouse(MouseKind::Drag(MouseButton::Left), 0, 0),
                mouse(MouseKind::ScrollDown, 3, 2),
            ] {
                let _ = probe.handle(&event);
            }
        }
    }
}

#[test]
fn undo_limit_drops_the_oldest_steps() {
    let mut state = TextAreaState::new();
    state.set_undo_limit(2);
    for word in ["a ", "b ", "c "] {
        type_str(&mut state, word);
    }
    assert!(state.undo());
    assert!(state.undo());
    assert!(!state.undo());
    assert_eq!(state.text(), "a ");
    state.clear_history();
    assert!(!state.can_redo());
}

#[test]
fn set_text_clear_and_replace_token_are_undoable() {
    let mut state = TextAreaState::from_text("hi @fo");
    let token = state.active_token(&[Trigger::new('@')]).expect("token");
    state.replace_token(&token, "@foo.rs ");
    assert_eq!(state.text(), "hi @foo.rs ");
    state.clear();
    assert!(state.is_empty());
    state.set_text("new\ntext");
    assert_eq!(state.line_count(), 2);
    state.undo();
    assert!(state.is_empty());
    state.undo();
    assert_eq!(state.text(), "hi @foo.rs ");
    state.undo();
    assert_eq!(state.text(), "hi @fo");
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    fn op() -> impl Strategy<Value = Event> {
        prop_oneof![
            prop::sample::select(vec!['a', ' ', '世', '\u{301}', '.', 'Z'])
                .prop_map(|c| key(KeyCode::Char(c))),
            Just(shift(KeyCode::Enter)),
            Just(key(KeyCode::Backspace)),
            Just(key(KeyCode::Delete)),
            Just(key(KeyCode::Left)),
            Just(key(KeyCode::Right)),
            Just(key(KeyCode::Up)),
            Just(shift(KeyCode::Left)),
            Just(shift(KeyCode::Up)),
            Just(shift(KeyCode::Down)),
            Just(ctrl(KeyCode::Backspace)),
            Just(ctrl(KeyCode::Delete)),
            Just(ctrl(KeyCode::Right)),
            Just(ctrl(KeyCode::Char('k'))),
            Just(ctrl(KeyCode::Char('u'))),
            Just(Event::Paste("p\nq".into())),
        ]
    }

    proptest! {
        /// Undoing every step returns the original text, and redoing every
        /// step returns the final text — for any edit sequence.
        #[test]
        fn undo_and_redo_round_trip(
            seed in "[a-c \n]{0,12}",
            ops in prop::collection::vec(op(), 0..40),
        ) {
            let mut state = TextAreaState::from_text(&seed);
            rows(&state, 6, 3);
            for event in &ops {
                let _ = state.handle(event);
                let (row, col) = state.cursor();
                prop_assert!(row < state.line_count());
                prop_assert!(col <= state.input().text().split('\n').nth(row).unwrap().chars().count());
            }
            let edited = state.text();
            while state.undo() {}
            prop_assert_eq!(state.text(), seed);
            while state.redo() {}
            prop_assert_eq!(state.text(), edited);
        }
    }
}
