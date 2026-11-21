//! A multi-line editor: [`TextInputState`]'s buffer plus a selection, undo and
//! redo, editor word motions, visual-line cursor movement, mouse placement, and
//! a scrolling view with optional line numbers and syntax colouring.
//!
//! [`TextInput`](crate::components::TextInput) is the lean composer: a buffer,
//! a cursor, and readline keys. [`TextArea`] is what a host reaches for when the
//! input is a real editing surface — a chat composer people draft long prompts
//! in, a commit-message box, a config editor. It reuses the same buffer, wrap,
//! and token model, so the two agree on what a line, a grapheme, and a
//! [`Trigger`] token are, and a host can move from one to the other without
//! re-learning the editing model.
//!
//! State and view are split the usual way: [`TextAreaState`] owns the text,
//! cursor, selection, and history and applies input; [`TextArea`] borrows it and
//! renders. Rendering records the area and the scroll position back into the
//! state (through a `Cell`), which is what lets mouse clicks, Page Up/Down, and
//! Up/Down across *wrapped* rows work without the host threading a size into
//! every call.

use std::cell::Cell;
use std::collections::VecDeque;

use super::textinput::{
    TextInputState, TextSpan, Token, Trigger, VisualRow, grapheme_boundaries,
    grapheme_boundary_at_or_before, visual_cursor_at, wrap_visual_rows,
};
use crate::event::{Event, InputOutcome, Key, KeyCode, Mouse, MouseButton, MouseKind};
use crate::geometry::{Rect, Size};
use crate::highlight::Highlighter;
use crate::style::Style;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

/// A position in the buffer as `(row, col)`: logical line and char index, the
/// same coordinates as [`TextInputState::cursor`].
pub type TextPos = (usize, usize);

/// Undo entries kept by default before the oldest is dropped.
const DEFAULT_UNDO_LIMIT: usize = 500;

/// Rows a mouse wheel notch scrolls.
const WHEEL_ROWS: usize = 3;

/// What kind of edit an undo entry records; consecutive edits of the same
/// typing kind merge into one entry so undo steps by word, not by keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Insert,
    Backspace,
    Delete,
    Other,
}

/// One undoable change, stored as the rows it replaced rather than a whole
/// snapshot, so history costs what the edits touched and not the buffer size
/// times the depth.
#[derive(Clone, Debug)]
struct Edit {
    /// First logical row of the changed region.
    start: usize,
    /// The region's rows before the edit.
    before: Vec<String>,
    /// The region's rows after the edit.
    after: Vec<String>,
    cursor_before: TextPos,
    anchor_before: Option<TextPos>,
    cursor_after: TextPos,
    kind: EditKind,
}

/// What the last render recorded: where the text was drawn and from which
/// visual row, so input that depends on the frame can be resolved later.
#[derive(Clone, Copy, Debug, Default)]
struct ViewMemo {
    /// The text rect (gutter excluded) of the last render.
    area: Rect,
    /// First visible visual row.
    top: usize,
    /// Scroll to keep the cursor visible on the next render. Cleared by the
    /// mouse wheel so a user can look away from the cursor.
    follow: bool,
}

/// The editable text, cursor, selection, and undo history of a [`TextArea`].
///
/// Built on a [`TextInputState`] (reachable through
/// [`input`](Self::input)), so tokens, Enter modes, and grapheme-aware editing
/// behave exactly as they do there. On top of it:
///
/// - **Selection.** Shift with any motion extends it; typing, pasting, or
///   deleting replaces it. [`selected_text`](Self::selected_text) and
///   [`cut`](Self::cut) hand the text to the host, which owns the clipboard.
/// - **Undo and redo.** `Ctrl+Z`, and `Ctrl+Y` or `Ctrl+Shift+Z`. Typing groups
///   by word, and a selection is restored with the text it covered.
/// - **Word motions.** `Ctrl`/`Alt` + `←`/`→` stop at word, punctuation, and
///   line boundaries; `Ctrl`/`Alt` + `Backspace`/`Delete` delete by word.
/// - **Visual movement.** `↑`/`↓` move by wrapped row and keep the column they
///   started from; `PageUp`/`PageDown` move by a screen; `Ctrl+Home`/`Ctrl+End`
///   jump to the ends of the buffer.
/// - **Mouse.** Click places the cursor, drag selects, Shift+click extends, and
///   the wheel scrolls without moving the cursor.
///
/// The readline keys [`TextInputState::handle`] documents (`C-a`, `C-e`, `C-k`,
/// `C-w`, `M-f`, …) work here too.
///
/// ```
/// use tuika::prelude::*;
///
/// let mut state = TextAreaState::from_text("hello world");
/// state.select_all();
/// assert_eq!(state.selected_text().as_deref(), Some("hello world"));
///
/// state.insert_str("bye");
/// assert_eq!(state.text(), "bye");
/// assert!(state.undo());
/// assert_eq!(state.text(), "hello world");
/// ```
#[derive(Clone, Debug)]
pub struct TextAreaState {
    input: TextInputState,
    /// Selection anchor; the selection runs from here to the cursor.
    anchor: Option<TextPos>,
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    undo_limit: usize,
    /// Whether the next typing edit may merge into the top undo entry.
    coalesce: bool,
    /// Visual column that vertical motion aims for, kept across a run of
    /// Up/Down presses so passing a short line does not lose it.
    goal_col: Option<u16>,
    /// A left-button drag that started inside the text is in progress.
    dragging: bool,
    view: Cell<ViewMemo>,
}

impl Default for TextAreaState {
    fn default() -> Self {
        Self::new()
    }
}

impl TextAreaState {
    /// An empty buffer.
    pub fn new() -> Self {
        Self {
            input: TextInputState::new(),
            anchor: None,
            undo: VecDeque::new(),
            redo: Vec::new(),
            undo_limit: DEFAULT_UNDO_LIMIT,
            coalesce: false,
            goal_col: None,
            dragging: false,
            view: Cell::new(ViewMemo {
                follow: true,
                ..ViewMemo::default()
            }),
        }
    }

    /// Seed from `text`, cursor at the end, with an empty history.
    pub fn from_text(text: &str) -> Self {
        let mut state = Self::new();
        state.input.set_text(text);
        state
    }

    /// The underlying buffer and cursor, for [`tokens`](TextInputState::tokens),
    /// [`mode`](TextInputState::mode), and the other read-only queries.
    pub fn input(&self) -> &TextInputState {
        &self.input
    }

    /// The full text, logical lines joined with `\n`.
    pub fn text(&self) -> String {
        self.input.text()
    }

    /// Whether the buffer is a single empty line.
    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    /// Number of logical lines.
    pub fn line_count(&self) -> usize {
        self.input.line_count()
    }

    /// Cursor as `(row, col)` in logical (char-index) coordinates.
    pub fn cursor(&self) -> TextPos {
        self.input.cursor()
    }

    /// Set which Enter chord submits; the other inserts a newline.
    pub fn set_mode(&mut self, mode: super::TextInputMode) {
        self.input.set_mode(mode);
    }

    /// Every token `triggers` match, as [`TextInputState::tokens`].
    pub fn tokens(&self, triggers: &[Trigger]) -> Vec<Token> {
        self.input.tokens(triggers)
    }

    /// The token the cursor is inside, as [`TextInputState::active_token`].
    pub fn active_token(&self, triggers: &[Trigger]) -> Option<Token> {
        self.input.active_token(triggers)
    }

    // ---- selection -------------------------------------------------------

    /// The selected range as `(start, end)` in reading order, or `None` when
    /// nothing is selected. `end` is exclusive.
    pub fn selection(&self) -> Option<(TextPos, TextPos)> {
        let anchor = self.anchor?;
        let cursor = self.cursor();
        (anchor != cursor).then(|| (anchor.min(cursor), anchor.max(cursor)))
    }

    /// The selected text, or `None` when nothing is selected — what a host
    /// copies (through [`term::clipboard`](crate::term::clipboard) or its own
    /// clipboard).
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        Some(text_between(&self.input.lines, start, end))
    }

    /// Select from `anchor` to `cursor` (both clamped into the buffer), leaving
    /// the cursor at `cursor`.
    pub fn set_selection(&mut self, anchor: TextPos, cursor: TextPos) {
        let anchor = self.clamp_pos(anchor);
        self.input.set_cursor(cursor.0, cursor.1);
        self.anchor = Some(anchor);
        self.end_motion();
    }

    /// Select the whole buffer, cursor at the end.
    pub fn select_all(&mut self) {
        let last = self.input.lines.len() - 1;
        let end = (last, self.input.lines[last].chars().count());
        self.set_selection((0, 0), end);
    }

    /// Drop the selection, keeping the cursor where it is.
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// Delete the selection and return its text, as one undoable edit — the
    /// host puts the result on the clipboard. `None` when nothing is selected.
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selection();
        Some(text)
    }

    // ---- history ---------------------------------------------------------

    /// Undo the last edit, restoring the cursor and any selection it replaced.
    /// Returns whether there was anything to undo.
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop_back() else {
            return false;
        };
        let end = edit.start + edit.after.len();
        self.input
            .lines
            .splice(edit.start..end, edit.before.iter().cloned());
        self.input.revision = self.input.revision.wrapping_add(1);
        self.place(edit.cursor_before);
        self.anchor = edit.anchor_before;
        self.redo.push(edit);
        self.after_history();
        true
    }

    /// Redo the last undone edit. Returns whether there was anything to redo.
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        let end = edit.start + edit.before.len();
        self.input
            .lines
            .splice(edit.start..end, edit.after.iter().cloned());
        self.input.revision = self.input.revision.wrapping_add(1);
        self.place(edit.cursor_after);
        self.anchor = None;
        self.undo.push_back(edit);
        self.after_history();
        true
    }

    /// Whether [`undo`](Self::undo) would change anything.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether [`redo`](Self::redo) would change anything.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Forget all undo and redo history — after a submit, typically, so the
    /// next draft does not undo into the last one.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.coalesce = false;
    }

    /// Cap the number of undo steps kept (at least one). Older steps are
    /// dropped first.
    pub fn set_undo_limit(&mut self, limit: usize) {
        self.undo_limit = limit.max(1);
        while self.undo.len() > self.undo_limit {
            self.undo.pop_front();
        }
    }

    fn after_history(&mut self) {
        self.coalesce = false;
        self.goal_col = None;
        self.follow_cursor();
    }

    // ---- edits -----------------------------------------------------------

    /// Replace all text as one undoable edit; cursor moves to the end.
    pub fn set_text(&mut self, text: &str) {
        let last = self.input.lines.len() - 1;
        self.apply(EditKind::Other, (0, last), |input| input.set_text(text));
    }

    /// Clear to a single empty line as one undoable edit.
    pub fn clear(&mut self) {
        let last = self.input.lines.len() - 1;
        self.apply(EditKind::Other, (0, last), TextInputState::clear);
    }

    /// Move the cursor to `(row, col)`, clamped into the buffer and to a
    /// grapheme boundary, and drop the selection.
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.anchor = None;
        self.input.set_cursor(row, col);
        self.end_motion();
    }

    /// Insert one char at the cursor, replacing the selection.
    pub fn insert_char(&mut self, ch: char) {
        let kind = if ch == '\n' {
            EditKind::Other
        } else {
            // Typing over a selection starts a typing group of its own, so the
            // undo that removes the word also brings the selection back.
            if self.selection().is_some() {
                self.coalesce = false;
            }
            EditKind::Insert
        };
        self.replace_selection_with(kind, |input| input.insert_char(ch));
        // A word ends a typing group, so undo removes a word at a time.
        if ch.is_whitespace() {
            self.coalesce = false;
        }
    }

    /// Insert a string (honoring embedded newlines) at the cursor, replacing
    /// the selection, as one undoable edit.
    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() && self.selection().is_none() {
            return;
        }
        self.replace_selection_with(EditKind::Other, |input| input.insert_str(text));
    }

    /// Insert a newline at the cursor, replacing the selection.
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the selection, or the grapheme before the cursor.
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let row = self.input.row;
        self.apply(
            EditKind::Backspace,
            (row.saturating_sub(1), row),
            TextInputState::backspace,
        );
    }

    /// Delete the selection, or the grapheme at the cursor.
    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        let row = self.input.row;
        self.apply(EditKind::Delete, (row, row + 1), TextInputState::delete);
    }

    /// Delete the selection, or back to the previous word boundary.
    pub fn delete_word_left(&mut self) {
        if self.delete_selection() {
            return;
        }
        let from = self.word_left_of(self.cursor());
        self.remove(from, self.cursor());
    }

    /// Delete the selection, or forward to the next word boundary.
    pub fn delete_word_right(&mut self) {
        if self.delete_selection() {
            return;
        }
        let to = self.word_right_of(self.cursor());
        self.remove(self.cursor(), to);
    }

    /// Delete to the end of the line, or join the next line at line end
    /// (emacs `C-k`).
    pub fn kill_to_line_end(&mut self) {
        let row = self.input.row;
        self.anchor = None;
        self.apply(
            EditKind::Other,
            (row, row + 1),
            TextInputState::kill_to_line_end,
        );
    }

    /// Delete from the start of the line to the cursor (emacs `C-u`).
    pub fn kill_to_line_start(&mut self) {
        let row = self.input.row;
        self.anchor = None;
        self.apply(
            EditKind::Other,
            (row, row),
            TextInputState::kill_to_line_start,
        );
    }

    /// Replace `token`'s range with `replacement` as one undoable edit, as
    /// [`TextInputState::replace_token`] — completion.
    pub fn replace_token(&mut self, token: &Token, replacement: &str) {
        if token.row >= self.input.lines.len() {
            return;
        }
        self.anchor = None;
        self.apply(EditKind::Other, (token.row, token.row), |input| {
            input.replace_token(token, replacement)
        });
    }

    /// Delete the selection as one edit. Returns whether there was one.
    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.remove(start, end);
        true
    }

    /// Remove `start..end` as one edit, leaving the cursor at `start`.
    fn remove(&mut self, start: TextPos, end: TextPos) {
        if start >= end {
            self.anchor = None;
            return;
        }
        self.apply(EditKind::Other, (start.0, end.0), |input| {
            remove_range(input, start, end)
        });
    }

    /// Run `edit` after deleting the selection, all as one undo entry.
    fn replace_selection_with(&mut self, kind: EditKind, edit: impl FnOnce(&mut TextInputState)) {
        match self.selection() {
            Some((start, end)) => self.apply(kind, (start.0, end.0), |input| {
                remove_range(input, start, end);
                edit(input);
            }),
            None => {
                let row = self.input.row;
                self.anchor = None;
                self.apply(kind, (row, row), edit);
            }
        }
    }

    /// Apply `edit`, which may touch only logical rows `rows.0..=rows.1` of
    /// the current buffer, and record it for undo.
    ///
    /// Every edit is local — a line split, a join with a neighbour, a range
    /// delete — so recording the touched rows before and after is enough to
    /// reverse it, and history never copies the whole buffer.
    fn apply(
        &mut self,
        kind: EditKind,
        rows: (usize, usize),
        edit: impl FnOnce(&mut TextInputState),
    ) {
        let last = self.input.lines.len() - 1;
        let lo = rows.0.min(last);
        let hi = rows.1.min(last).max(lo);
        let before = self.input.lines[lo..=hi].to_vec();
        let old_len = self.input.lines.len();
        let cursor_before = self.cursor();
        let anchor_before = self.anchor;
        let revision = self.input.revision;

        edit(&mut self.input);

        self.anchor = None;
        self.goal_col = None;
        self.follow_cursor();
        if self.input.revision == revision {
            return;
        }
        // Rows outside lo..=hi are untouched, so the region's new height is
        // its old height plus the buffer's growth.
        let count = (hi - lo + 1 + self.input.lines.len()) - old_len;
        let after = self.input.lines[lo..lo + count].to_vec();
        let cursor_after = self.cursor();
        self.redo.clear();

        if self.coalesce
            && kind != EditKind::Other
            && let Some(top) = self.undo.back_mut()
            && top.kind == kind
            && top.start == lo
            && top.after.len() == before.len()
            && top.cursor_after == cursor_before
        {
            top.after = after;
            top.cursor_after = cursor_after;
        } else {
            self.undo.push_back(Edit {
                start: lo,
                before,
                after,
                cursor_before,
                anchor_before,
                cursor_after,
                kind,
            });
            while self.undo.len() > self.undo_limit {
                self.undo.pop_front();
            }
        }
        self.coalesce = kind != EditKind::Other;
    }

    // ---- motion ----------------------------------------------------------

    /// Start a motion: keep or start the selection when `extend`, otherwise
    /// drop it.
    fn begin_motion(&mut self, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor());
            }
        } else {
            self.anchor = None;
        }
    }

    fn end_motion(&mut self) {
        self.coalesce = false;
        self.follow_cursor();
    }

    fn horizontal(&mut self, extend: bool, to: impl FnOnce(&Self) -> TextPos) {
        self.begin_motion(extend);
        let pos = to(self);
        self.place(pos);
        self.goal_col = None;
        self.end_motion();
    }

    /// Move one grapheme left; with a selection and no Shift, collapse it to
    /// its start instead.
    pub fn move_left(&mut self, extend: bool) {
        if !extend && let Some((start, _)) = self.selection() {
            self.horizontal(false, |_| start);
            return;
        }
        self.begin_motion(extend);
        self.input.move_left();
        self.goal_col = None;
        self.end_motion();
    }

    /// Move one grapheme right; with a selection and no Shift, collapse it to
    /// its end instead.
    pub fn move_right(&mut self, extend: bool) {
        if !extend && let Some((_, end)) = self.selection() {
            self.horizontal(false, |_| end);
            return;
        }
        self.begin_motion(extend);
        self.input.move_right();
        self.goal_col = None;
        self.end_motion();
    }

    /// Move to the previous word boundary.
    pub fn move_word_left(&mut self, extend: bool) {
        self.horizontal(extend, |s| s.word_left_of(s.cursor()));
    }

    /// Move to the next word boundary.
    pub fn move_word_right(&mut self, extend: bool) {
        self.horizontal(extend, |s| s.word_right_of(s.cursor()));
    }

    /// Move to the start of the logical line.
    pub fn move_home(&mut self, extend: bool) {
        self.horizontal(extend, |s| (s.input.row, 0));
    }

    /// Move to the end of the logical line.
    pub fn move_end(&mut self, extend: bool) {
        self.horizontal(extend, |s| {
            let row = s.input.row;
            (row, s.input.lines[row].chars().count())
        });
    }

    /// Move to the start of the buffer.
    pub fn move_to_start(&mut self, extend: bool) {
        self.horizontal(extend, |_| (0, 0));
    }

    /// Move to the end of the buffer.
    pub fn move_to_end(&mut self, extend: bool) {
        self.horizontal(extend, |s| {
            let last = s.input.lines.len() - 1;
            (last, s.input.lines[last].chars().count())
        });
    }

    /// Move up one row — a *wrapped* row once the view has rendered — keeping
    /// the column a run of vertical moves started from.
    pub fn move_up(&mut self, extend: bool) {
        self.vertical(extend, -1);
    }

    /// Move down one row, as [`move_up`](Self::move_up).
    pub fn move_down(&mut self, extend: bool) {
        self.vertical(extend, 1);
    }

    /// Move up by a screen of the last rendered height.
    pub fn page_up(&mut self, extend: bool) {
        let rows = self.page_rows();
        self.vertical(extend, -(rows as isize));
    }

    /// Move down by a screen of the last rendered height.
    pub fn page_down(&mut self, extend: bool) {
        let rows = self.page_rows();
        self.vertical(extend, rows as isize);
    }

    fn page_rows(&self) -> usize {
        usize::from(self.view.get().area.height.saturating_sub(1)).max(1)
    }

    fn vertical(&mut self, extend: bool, delta: isize) {
        self.begin_motion(extend);
        let width = self.view.get().area.width;
        let lines = &self.input.lines;
        // Before the first render there is no wrap width; move by logical line
        // so the state is still usable headless.
        let width = if width == 0 { u16::MAX } else { width };
        let rows = wrap_visual_rows(lines, width);
        let (vrow, vcol) = visual_cursor_at(lines, self.input.row, self.input.col, width);
        let goal = *self.goal_col.get_or_insert(vcol);
        let target = vrow as isize + delta;
        let pos = if target < 0 {
            (0, 0)
        } else if target as usize >= rows.len() {
            let last = lines.len() - 1;
            (last, lines[last].chars().count())
        } else {
            pos_in_row(&rows, target as usize, goal)
        };
        self.place(pos);
        self.end_motion();
    }

    /// Put the cursor at `pos` without touching selection or history.
    fn place(&mut self, pos: TextPos) {
        self.input.set_cursor(pos.0, pos.1);
    }

    fn clamp_pos(&self, (row, col): TextPos) -> TextPos {
        let row = row.min(self.input.lines.len() - 1);
        (
            row,
            grapheme_boundary_at_or_before(&self.input.lines[row], col),
        )
    }

    /// The previous word boundary from `pos`: back over whitespace, then over
    /// one run of the same character class. At a line start it steps to the
    /// end of the previous line, the way editors treat the newline as a stop.
    fn word_left_of(&self, (row, col): TextPos) -> TextPos {
        if col == 0 {
            return match row {
                0 => (0, 0),
                _ => (row - 1, self.input.lines[row - 1].chars().count()),
            };
        }
        let line = &self.input.lines[row];
        let chars: Vec<char> = line.chars().collect();
        let mut i = col.min(chars.len());
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let class = char_class(chars[i - 1]);
            while i > 0 && char_class(chars[i - 1]) == class {
                i -= 1;
            }
        }
        (row, grapheme_boundary_at_or_before(line, i))
    }

    /// The next word boundary from `pos`: over whitespace, then one run of the
    /// same character class. At a line end it steps to the next line's start.
    fn word_right_of(&self, (row, col): TextPos) -> TextPos {
        let line = &self.input.lines[row];
        let chars: Vec<char> = line.chars().collect();
        if col >= chars.len() {
            return if row + 1 < self.input.lines.len() {
                (row + 1, 0)
            } else {
                (row, chars.len())
            };
        }
        let mut i = col;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() {
            let class = char_class(chars[i]);
            while i < chars.len() && char_class(chars[i]) == class {
                i += 1;
            }
        }
        // Combining marks classify as punctuation; snap forward so a word stop
        // never lands inside a grapheme cluster.
        let i = grapheme_boundaries(line)
            .into_iter()
            .find(|b| *b >= i)
            .unwrap_or(chars.len());
        (row, i)
    }

    // ---- view bookkeeping --------------------------------------------------

    fn follow_cursor(&mut self) {
        let mut memo = self.view.get();
        memo.follow = true;
        self.view.set(memo);
    }

    /// The cursor cell in terminal coordinates as of the last render of a
    /// [`TextArea`] over this state, or `None` before the first render or when
    /// the cursor is scrolled out of view. Hand it to the terminal's
    /// `set_cursor_position` after painting.
    pub fn cursor_screen(&self) -> Option<(u16, u16)> {
        let memo = self.view.get();
        let area = memo.area;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let (vrow, vcol) = visual_cursor_at(
            &self.input.lines,
            self.input.row,
            self.input.col,
            area.width,
        );
        let vrow = vrow as usize;
        if vrow < memo.top || vrow >= memo.top + usize::from(area.height) {
            return None;
        }
        Some((
            area.x.saturating_add(vcol.min(area.width - 1)),
            area.y.saturating_add((vrow - memo.top) as u16),
        ))
    }

    /// The first visible visual row as of the last render.
    pub fn scroll_top(&self) -> usize {
        self.view.get().top
    }

    /// The buffer position under terminal cell `(x, y)`, resolved against the
    /// last render. Cells above or below the text resolve to the rows just
    /// beyond the visible ones, so a drag past an edge keeps selecting.
    fn pos_at(&self, x: u16, y: u16) -> TextPos {
        let memo = self.view.get();
        let area = memo.area;
        let rows = wrap_visual_rows(&self.input.lines, area.width);
        let vrow = if y < area.y {
            memo.top.saturating_sub(usize::from(area.y - y))
        } else {
            memo.top + usize::from(y - area.y)
        };
        let vrow = vrow.min(rows.len().saturating_sub(1));
        pos_in_row(&rows, vrow, x.saturating_sub(area.x))
    }

    // ---- input -----------------------------------------------------------

    /// Apply an input event.
    ///
    /// Returns [`InputOutcome::Changed`] when text, cursor, selection, or
    /// scroll changed, [`InputOutcome::Consumed`] when a recognized key was
    /// already at its bound, [`InputOutcome::Submitted`] for the submit chord
    /// of the configured [`TextInputMode`](super::TextInputMode) (the buffer is
    /// left as is), and [`InputOutcome::Ignored`] otherwise — `Tab`, and `Esc`
    /// with no selection, are left for the host.
    pub fn handle(&mut self, event: &Event) -> InputOutcome {
        let before = (
            self.cursor(),
            self.anchor,
            self.input.revision,
            self.view.get().top,
        );
        let handled = match event {
            Event::Key(key) => self.handle_key(key),
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            Event::Paste(text) => {
                self.insert_str(text);
                Some(false)
            }
            Event::Resize { .. } => None,
        };
        let after = (
            self.cursor(),
            self.anchor,
            self.input.revision,
            self.view.get().top,
        );
        match handled {
            None => InputOutcome::Ignored,
            Some(true) => InputOutcome::Submitted,
            Some(false) if before == after => InputOutcome::Consumed,
            Some(false) => InputOutcome::Changed,
        }
    }

    /// `Some(submit)` when the key was recognized.
    fn handle_key(&mut self, key: &Key) -> Option<bool> {
        let shift = key.shift;
        let code = match key.code {
            // Ctrl+Shift+letter arrives as an uppercase char on some terminals.
            KeyCode::Char(c) if key.ctrl || key.alt => KeyCode::Char(c.to_ascii_lowercase()),
            code => code,
        };
        match (key.ctrl, key.alt, code) {
            (true, false, KeyCode::Char('z')) if shift => {
                self.redo();
            }
            (true, false, KeyCode::Char('z')) => {
                self.undo();
            }
            (true, false, KeyCode::Char('y')) => {
                self.redo();
            }
            (true, _, KeyCode::Left) | (false, true, KeyCode::Left) => self.move_word_left(shift),
            (true, _, KeyCode::Right) | (false, true, KeyCode::Right) => {
                self.move_word_right(shift)
            }
            (true, false, KeyCode::Home) => self.move_to_start(shift),
            (true, false, KeyCode::End) => self.move_to_end(shift),
            (true, false, KeyCode::Backspace) | (false, true, KeyCode::Backspace) => {
                self.delete_word_left()
            }
            (true, false, KeyCode::Delete) | (false, true, KeyCode::Delete) => {
                self.delete_word_right()
            }
            // Readline keys, as TextInputState binds them.
            (true, false, KeyCode::Char('a')) => self.move_home(shift),
            (true, false, KeyCode::Char('e')) => self.move_end(shift),
            (true, false, KeyCode::Char('f')) => self.move_right(shift),
            (true, false, KeyCode::Char('b')) => self.move_left(shift),
            (true, false, KeyCode::Char('p')) => self.move_up(shift),
            (true, false, KeyCode::Char('n')) => self.move_down(shift),
            (true, false, KeyCode::Char('h')) => self.backspace(),
            (true, false, KeyCode::Char('d')) => self.delete(),
            (true, false, KeyCode::Char('j')) => self.newline(),
            (true, false, KeyCode::Char('k')) => self.kill_to_line_end(),
            (true, false, KeyCode::Char('u')) => self.kill_to_line_start(),
            (true, false, KeyCode::Char('w')) => self.delete_word_left(),
            (false, true, KeyCode::Char('f')) => self.move_word_right(shift),
            (false, true, KeyCode::Char('b')) => self.move_word_left(shift),
            (false, true, KeyCode::Char('d')) => self.delete_word_right(),
            (false, false, KeyCode::Char(c)) => self.insert_char(c),
            (false, false, KeyCode::Enter) => {
                let submit = match self.input.mode() {
                    super::TextInputMode::SubmitOnEnter => !shift,
                    super::TextInputMode::SubmitOnShiftEnter => shift,
                };
                if submit {
                    return Some(true);
                }
                self.newline();
            }
            (false, false, KeyCode::Backspace) => self.backspace(),
            (false, false, KeyCode::Delete) => self.delete(),
            (false, false, KeyCode::Left) => self.move_left(shift),
            (false, false, KeyCode::Right) => self.move_right(shift),
            (false, false, KeyCode::Up) => self.move_up(shift),
            (false, false, KeyCode::Down) => self.move_down(shift),
            (false, false, KeyCode::Home) => self.move_home(shift),
            (false, false, KeyCode::End) => self.move_end(shift),
            (false, false, KeyCode::PageUp) => self.page_up(shift),
            (false, false, KeyCode::PageDown) => self.page_down(shift),
            (false, false, KeyCode::Esc) if self.selection().is_some() => self.clear_selection(),
            _ => return None,
        }
        Some(false)
    }

    fn handle_mouse(&mut self, mouse: &Mouse) -> Option<bool> {
        let memo = self.view.get();
        let area = memo.area;
        let inside = mouse.column >= area.x
            && mouse.column < area.right()
            && mouse.row >= area.y
            && mouse.row < area.bottom();
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) if inside => {
                let pos = self.pos_at(mouse.column, mouse.row);
                if mouse.shift {
                    self.begin_motion(true);
                } else {
                    self.anchor = Some(pos);
                }
                self.place(pos);
                self.goal_col = None;
                self.dragging = true;
                self.end_motion();
            }
            MouseKind::Drag(MouseButton::Left) if self.dragging => {
                let pos = self.pos_at(mouse.column, mouse.row);
                self.place(pos);
                self.end_motion();
            }
            MouseKind::Up(MouseButton::Left) if self.dragging => {
                self.dragging = false;
                if self.anchor == Some(self.cursor()) {
                    self.anchor = None;
                }
            }
            MouseKind::ScrollUp | MouseKind::ScrollDown if inside => {
                let total = wrap_visual_rows(&self.input.lines, area.width).len();
                let max_top = total.saturating_sub(usize::from(area.height));
                let top = if mouse.kind == MouseKind::ScrollUp {
                    memo.top.saturating_sub(WHEEL_ROWS)
                } else {
                    (memo.top + WHEEL_ROWS).min(max_top)
                };
                self.view.set(ViewMemo {
                    top,
                    follow: false,
                    ..memo
                });
            }
            _ => return None,
        }
        Some(false)
    }
}

/// Character classes a word motion stops between.
fn char_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// Byte offset of char index `col` in `line` (or its length past the end).
fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
}

/// The text between two positions, lines joined with `\n`.
fn text_between(lines: &[String], start: TextPos, end: TextPos) -> String {
    if start.0 == end.0 {
        let line = &lines[start.0];
        return line[byte_at(line, start.1)..byte_at(line, end.1)].to_string();
    }
    let mut out = String::new();
    let first = &lines[start.0];
    out.push_str(&first[byte_at(first, start.1)..]);
    for line in &lines[start.0 + 1..end.0] {
        out.push('\n');
        out.push_str(line);
    }
    out.push('\n');
    let last = &lines[end.0];
    out.push_str(&last[..byte_at(last, end.1)]);
    out
}

/// Delete `start..end` from `input`, leaving the cursor at `start`.
fn remove_range(input: &mut TextInputState, start: TextPos, end: TextPos) {
    let head = &input.lines[start.0][..byte_at(&input.lines[start.0], start.1)];
    let tail = &input.lines[end.0][byte_at(&input.lines[end.0], end.1)..];
    let joined = format!("{head}{tail}");
    input.lines.splice(start.0..=end.0, [joined]);
    input.row = start.0;
    input.col = start.1;
    input.revision = input.revision.wrapping_add(1);
}

/// The buffer position at visual column `x` of visual row `index`.
fn pos_in_row(rows: &[VisualRow<'_>], index: usize, x: u16) -> TextPos {
    let row = &rows[index];
    let mut acc = 0u16;
    for cell in &row.cells {
        if acc.saturating_add(cell.width) > x {
            return (row.logical, cell.start);
        }
        acc = acc.saturating_add(cell.width);
    }
    // Past the end of the row. A soft-wrapped row's end column renders at the
    // start of the next row, so stay on this row's last grapheme instead.
    let continues = rows
        .get(index + 1)
        .is_some_and(|next| next.logical == row.logical);
    match row.cells.last() {
        Some(cell) if continues => (row.logical, cell.start),
        _ => (row.logical, row.end),
    }
}

/// Renders a [`TextAreaState`]: wrapped text scrolled to the cursor, the
/// selection, host highlights, optional syntax colouring and line numbers.
///
/// It borrows the state for one frame and records where it drew, so call
/// [`TextAreaState::cursor_screen`] after painting to place the terminal
/// cursor. Text wraps at word boundaries exactly as
/// [`TextInput`](crate::components::TextInput) does.
///
/// | Builder | Default | Effect |
/// |---|---|---|
/// | [`style`](Self::style) | inherited | base text style |
/// | [`selection_style`](Self::selection_style) | `Theme::selection_fg` on `selection_bg` | selected cells |
/// | [`line_numbers`](Self::line_numbers) | off | a gutter numbering logical lines, in `Theme::dim` |
/// | [`highlighter`](Self::highlighter) | none | syntax colouring through a [`Highlighter`] |
/// | [`highlights`](Self::highlights) | none | host ranges painted over the text |
/// | [`placeholder`](Self::placeholder) | none | text shown while empty |
///
/// ```
/// use tuika::prelude::*;
///
/// let state = TextAreaState::from_text("fn main() {}\n");
/// let view = TextArea::new(&state).line_numbers(true);
/// let frame = tuika::testing::render(&view, 16, 2, &Theme::default());
/// assert_eq!(tuika::testing::grid(&frame), "1 fn main() {}  \n2               ");
/// ```
///
/// ![textarea demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/textarea.gif)
pub struct TextArea<'a> {
    state: &'a TextAreaState,
    style: Style,
    selection_style: Option<Style>,
    gutter_style: Option<Style>,
    line_numbers: bool,
    highlights: Vec<TextSpan>,
    highlighter: Option<(&'a dyn Highlighter, String)>,
    placeholder: Option<(String, Style)>,
}

impl<'a> TextArea<'a> {
    /// Render `state`.
    pub fn new(state: &'a TextAreaState) -> Self {
        Self {
            state,
            style: Style::default(),
            selection_style: None,
            gutter_style: None,
            line_numbers: false,
            highlights: Vec::new(),
            highlighter: None,
            placeholder: None,
        }
    }

    /// Set the base text style.
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Style painted over selected cells.
    pub fn selection_style(mut self, style: Style) -> Self {
        self.selection_style = Some(style);
        self
    }

    /// Number logical lines in a left gutter.
    pub fn line_numbers(mut self, on: bool) -> Self {
        self.line_numbers = on;
        self
    }

    /// Style of the line-number gutter.
    pub fn gutter_style(mut self, style: Style) -> Self {
        self.gutter_style = Some(style);
        self
    }

    /// Paint host-computed ranges over the text, as
    /// [`TextInput::highlights`](crate::components::TextInput::highlights).
    pub fn highlights(mut self, spans: Vec<TextSpan>) -> Self {
        self.highlights = spans;
        self
    }

    /// Colour the text as `lang` through `highlighter` — the same boundary
    /// [`CodeBlock`](crate::components::CodeBlock) uses, so a host's
    /// tree-sitter highlighter works unchanged. Wrap it in a
    /// [`CachedHighlighter`](crate::highlight::CachedHighlighter) for long
    /// buffers: the whole text is highlighted each frame.
    pub fn highlighter(
        mut self,
        highlighter: &'a dyn Highlighter,
        lang: impl Into<String>,
    ) -> Self {
        self.highlighter = Some((highlighter, lang.into()));
        self
    }

    /// Text drawn in place of an empty buffer, in its own style.
    pub fn placeholder(mut self, text: impl Into<String>, style: Style) -> Self {
        self.placeholder = Some((text.into(), style));
        self
    }

    /// Gutter width in cells for `area_width`: the widest line number plus a
    /// space, or nothing when off or when it would leave no room for text.
    fn gutter_width(&self, area_width: u16) -> u16 {
        if !self.line_numbers {
            return 0;
        }
        let digits = self.state.line_count().to_string().len() as u16;
        let gutter = digits + 1;
        if gutter >= area_width { 0 } else { gutter }
    }

    /// Per-char syntax styles for each logical line, when a highlighter is
    /// set and knows the language.
    fn syntax(&self, ctx: &RenderCtx) -> Option<Vec<Vec<Style>>> {
        let (highlighter, lang) = self.highlighter.as_ref()?;
        let lines: Vec<&str> = self.state.input.lines.iter().map(String::as_str).collect();
        let spans = highlighter.highlight(lang, &lines, ctx.theme)?;
        Some(
            spans
                .into_iter()
                .map(|line| {
                    line.into_iter()
                        .flat_map(|span| {
                            let n = span.content.chars().count();
                            std::iter::repeat_n(span.style, n)
                        })
                        .collect()
                })
                .collect(),
        )
    }
}

impl View for TextArea<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let width = available
            .width
            .saturating_sub(self.gutter_width(available.width));
        let height = wrap_visual_rows(&self.state.input.lines, width)
            .len()
            .max(1);
        Size::new(available.width, height.min(usize::from(u16::MAX)) as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let gutter = self.gutter_width(area.width);
        let text = Rect::new(
            area.x.saturating_add(gutter),
            area.y,
            area.width - gutter,
            area.height,
        );
        let state = self.state;
        let lines = &state.input.lines;
        let rows = wrap_visual_rows(lines, text.width);
        let height = usize::from(text.height);

        let mut memo = state.view.get();
        memo.area = text;
        if memo.follow && height > 0 {
            let (cursor_row, _) =
                visual_cursor_at(lines, state.input.row, state.input.col, text.width);
            let cursor_row = cursor_row as usize;
            if cursor_row < memo.top {
                memo.top = cursor_row;
            } else if cursor_row >= memo.top + height {
                memo.top = cursor_row + 1 - height;
            }
        }
        memo.top = memo.top.min(rows.len().saturating_sub(height));
        state.view.set(memo);
        if text.width == 0 || text.height == 0 {
            return;
        }

        let gutter_style = self
            .gutter_style
            .unwrap_or_else(|| Style::default().fg(ctx.theme.dim));
        let selection_style = self.selection_style.unwrap_or_else(|| {
            Style::default()
                .fg(ctx.theme.selection_fg)
                .bg(ctx.theme.selection_bg)
        });
        let selection = state.selection();
        let selected = |pos: TextPos| selection.is_some_and(|(a, b)| pos >= a && pos < b);

        if state.is_empty()
            && let Some((placeholder, style)) = &self.placeholder
        {
            if gutter > 0 {
                surface.set_string(
                    area.x,
                    area.y,
                    &format!("{:>w$}", 1, w = usize::from(gutter - 1)),
                    gutter_style,
                );
            }
            surface.set_string(text.x, text.y, placeholder, *style);
            return;
        }

        let syntax = self.syntax(ctx);
        for (i, row) in rows.iter().enumerate().skip(memo.top).take(height) {
            let y = text.y.saturating_add((i - memo.top) as u16);
            if gutter > 0 && row.start == 0 {
                let number = format!("{:>w$}", row.logical + 1, w = usize::from(gutter - 1));
                surface.set_string(area.x, y, &number, gutter_style);
            }
            let line_syntax = syntax.as_ref().and_then(|s| s.get(row.logical));
            let mut x = text.x;
            for cell in &row.cells {
                if cell.width == 0 || x >= text.right() {
                    continue;
                }
                let mut style = self.style;
                if let Some(s) = line_syntax.and_then(|l| l.get(cell.start)) {
                    style = style.patch(*s);
                }
                for span in &self.highlights {
                    if span.row == row.logical && cell.start >= span.start && cell.start < span.end
                    {
                        style = style.patch(span.style);
                    }
                }
                if selected((row.logical, cell.start)) {
                    style = style.patch(selection_style);
                }
                surface.set_string(x, y, cell.text, style);
                x = x.saturating_add(cell.width);
            }
            // A selection that runs through the line break shows one selected
            // cell after the line's last row, so an empty line inside it shows.
            let last_of_line = rows
                .get(i + 1)
                .is_none_or(|next| next.logical != row.logical);
            let line_end = (row.logical, lines[row.logical].chars().count());
            if last_of_line && selected(line_end) && x < text.right() {
                surface.set_string(x, y, " ", self.style.patch(selection_style));
            }
        }
    }
}

#[cfg(test)]
#[path = "textarea_tests.rs"]
mod tests;
