//! A multi-line text editor: buffer, cursor, editing, soft-wrap, and a screen
//! cursor position the host can hand to the terminal.
//!
//! Like the rest of tuika's interactive widgets, state and view are split:
//! [`TextInputState`] owns the text and cursor and applies edits (via
//! [`TextInputState::handle`] or the explicit methods); [`TextInput`] borrows it
//! and renders. The view is word-soft-wrapped to the render width — a logical
//! line longer than the area breaks at the last space that fits (hard-breaking a
//! word longer than the width), and a line that fills the width exactly wraps the
//! cursor onto a fresh row, like a real editor. Wrapping and cursor placement
//! count terminal cells, while editing moves and deletes whole grapheme clusters.
//!
//! It is a *rendering + edit model*, not a terminal: the host reads
//! [`TextInputState::cursor_screen`] after layout and calls the backend's
//! `set_cursor_position`. Hosts can configure whether Enter or Shift+Enter
//! submits via [`TextInputMode`]; the other chord inserts a newline.

use crate::geometry::Rect;
use crate::style::Style;
use unicode_segmentation::UnicodeSegmentation;

use crate::event::{Event, KeyCode};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};
use crate::width::grapheme_cols;

/// The editable text and cursor of a [`TextInput`].
#[derive(Clone, Debug)]
pub struct TextInputState {
    /// Logical lines (split on `\n`); always at least one (possibly empty).
    lines: Vec<String>,
    /// Cursor row (logical line index).
    row: usize,
    /// Cursor column (char index within `lines[row]`).
    col: usize,
    mode: TextInputMode,
    /// Monotonic marker for text mutations, used to classify handled no-ops.
    revision: u64,
}

/// Controls which Enter chord submits a text input.
///
/// Applied by both [`TextInputState::handle_enter`] and [`TextInputState::handle`]
/// when it sees Enter / Shift+Enter. Ctrl+J always inserts a newline regardless
/// of this mode (emacs newline / raw-mode LF from terminals without enhanced
/// keyboard reporting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextInputMode {
    /// Enter submits; Shift+Enter inserts a newline.
    #[default]
    SubmitOnEnter,
    /// Shift+Enter submits; Enter inserts a newline.
    SubmitOnShiftEnter,
}

/// Where a [`Trigger`] character is allowed to open a token.
///
/// This is the whole of tuika's opinion about inline tokens: *where* the opening
/// character may sit. What `@` or `/` (or `#`, or `:`) then **means** — a file
/// mention, a command, an issue, an emoji — is the host's, and so is what it
/// shows for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerAnchor {
    /// Any position, mid-word included (`foo@bar` opens a token at the `@`).
    Anywhere,
    /// Start of a word: the first column, or right after whitespace.
    #[default]
    WordStart,
    /// The first character of a logical line.
    LineStart,
    /// The very first character of the buffer — a whole-input mode switch, the
    /// way a command palette treats a leading `/`.
    BufferStart,
}

/// A character that opens an inline token in a [`TextInputState`].
///
/// A host declares the triggers it cares about and asks the state for the
/// [`Token`]s they produce ([`tokens`](TextInputState::tokens),
/// [`active_token`](TextInputState::active_token)); tuika finds and delimits
/// them, and does nothing else with them. Popups, completion sources, and
/// styling stay in the host, so a different app can give `/` and `@` entirely
/// different semantics — or use neither.
///
/// ```
/// use tuika::prelude::*;
///
/// // `/command` only as the whole input's first character; `@mention` anywhere
/// // a word starts; `#123` mid-word too.
/// let triggers = [
///     Trigger::new('/').anchor(TriggerAnchor::BufferStart),
///     Trigger::new('@'),
///     Trigger::new('#').anchor(TriggerAnchor::Anywhere),
/// ];
///
/// let state = TextInputState::from_text("review @src/lib.rs for #42");
/// let tokens = state.tokens(&triggers);
/// assert_eq!(tokens.len(), 2);
/// assert_eq!(tokens[0].text, "@src/lib.rs");
/// assert_eq!(tokens[0].query(), "src/lib.rs");
/// assert_eq!(tokens[1].text, "#42");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    /// The character that opens the token.
    pub start: char,
    /// Where that character may appear for it to count.
    pub anchor: TriggerAnchor,
    /// End the token at the first whitespace (the default). When false it runs
    /// to the end of its logical line, so a query can contain spaces —
    /// `/model gpt 5` as one token rather than three.
    pub stop_at_whitespace: bool,
}

impl Trigger {
    /// A trigger on `start`, anchored at a word start, ending at whitespace.
    pub fn new(start: char) -> Self {
        Self {
            start,
            anchor: TriggerAnchor::default(),
            stop_at_whitespace: true,
        }
    }

    /// Restrict where the trigger character may appear.
    pub fn anchor(mut self, anchor: TriggerAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Let the token run to the end of its line instead of stopping at the
    /// first whitespace.
    pub fn to_line_end(mut self) -> Self {
        self.stop_at_whitespace = false;
        self
    }

    /// Whether `col` on `line` is a position this trigger may open at.
    fn anchored_at(&self, chars: &[char], row: usize, col: usize) -> bool {
        match self.anchor {
            TriggerAnchor::Anywhere => true,
            TriggerAnchor::WordStart => {
                col == 0 || chars.get(col - 1).is_some_and(|c| c.is_whitespace())
            }
            TriggerAnchor::LineStart => col == 0,
            TriggerAnchor::BufferStart => row == 0 && col == 0,
        }
    }
}

/// A token a [`Trigger`] matched: where it sits and what it says.
///
/// Positions are **char** indices into the logical line, matching
/// [`TextInputState::cursor`], so a host can turn a token into a
/// [`TextSpan`] ([`span`](Token::span)) or replace it
/// ([`replace_token`](TextInputState::replace_token)) without re-deriving them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The trigger character that opened it.
    pub trigger: char,
    /// Logical line the token is on.
    pub row: usize,
    /// Char index of the trigger character.
    pub start: usize,
    /// Char index one past the token's last character.
    pub end: usize,
    /// The token including its trigger character (`"@src/lib.rs"`).
    pub text: String,
}

impl Token {
    /// The token without its trigger character — what a host filters on.
    pub fn query(&self) -> &str {
        let mut chars = self.text.chars();
        chars.next();
        chars.as_str()
    }

    /// This token as a styled range, for [`TextInput::highlights`].
    pub fn span(&self, style: Style) -> TextSpan {
        TextSpan {
            row: self.row,
            start: self.start,
            end: self.end,
            style,
        }
    }
}

/// A styled char range within one logical line, applied by
/// [`TextInput::highlights`].
///
/// Ranges are host-computed: from [`Token`]s, a regex, a spell checker, a
/// syntax pass — tuika only paints them. Later spans win where they overlap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSpan {
    /// Logical line the range is on.
    pub row: usize,
    /// First char index covered.
    pub start: usize,
    /// One past the last char index covered.
    pub end: usize,
    /// Style painted over the base text style.
    pub style: Style,
}

impl TextSpan {
    /// A styled range over `start..end` of logical line `row`.
    pub fn new(row: usize, start: usize, end: usize, style: Style) -> Self {
        Self {
            row,
            start,
            end,
            style,
        }
    }

    fn covers(&self, row: usize, col: usize) -> bool {
        self.row == row && col >= self.start && col < self.end
    }
}

/// Internal result of applying an Enter chord to a text input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextInputEvent {
    /// The chord inserted a newline; text changed.
    Changed,
    /// The chord requested submission.
    Submit,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInputState {
    /// An empty single-line buffer with cursor at the start.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            mode: TextInputMode::default(),
            revision: 0,
        }
    }

    /// Set how Enter and Shift+Enter submit or insert newlines.
    pub fn set_mode(&mut self, mode: TextInputMode) {
        self.mode = mode;
    }

    /// Return the current Enter behavior.
    pub fn mode(&self) -> TextInputMode {
        self.mode
    }

    /// Apply an Enter chord according to the configured mode.
    pub fn handle_enter(&mut self, shift: bool) -> crate::InputOutcome {
        match self.enter_event(shift) {
            TextInputEvent::Changed => crate::InputOutcome::Changed,
            TextInputEvent::Submit => crate::InputOutcome::Submitted,
        }
    }

    fn enter_event(&mut self, shift: bool) -> TextInputEvent {
        let submit = match self.mode {
            TextInputMode::SubmitOnEnter => !shift,
            TextInputMode::SubmitOnShiftEnter => shift,
        };
        if submit {
            TextInputEvent::Submit
        } else {
            self.newline();
            TextInputEvent::Changed
        }
    }

    /// Seed from `text`, cursor at the end.
    pub fn from_text(text: &str) -> Self {
        let mut s = Self::new();
        s.set_text(text);
        s
    }

    /// The full text, logical lines joined with `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Whether the buffer is a single empty line.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Number of logical lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Cursor as `(row, col)` in logical (char-index) coordinates.
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// Replace all text; cursor moves to the end.
    pub fn set_text(&mut self, text: &str) {
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        if self.lines != lines {
            self.revision = self.revision.wrapping_add(1);
        }
        self.lines = lines;
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].chars().count();
    }

    /// Clear to a single empty line. Preserves [`TextInputMode`].
    pub fn clear(&mut self) {
        if !self.is_empty() {
            self.revision = self.revision.wrapping_add(1);
        }
        self.lines.clear();
        self.lines.push(String::new());
        self.row = 0;
        self.col = 0;
    }

    /// Move the cursor to `(row, col)`, clamped into the buffer and to the
    /// preceding grapheme boundary. Lets a host mirror an external editor's
    /// cursor into this state for rendering.
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len().saturating_sub(1));
        self.col = grapheme_boundary_at_or_before(&self.lines[self.row], col);
    }

    fn row_chars(&self, row: usize) -> Vec<char> {
        self.lines[row].chars().collect()
    }

    fn set_row(&mut self, row: usize, chars: Vec<char>) {
        let line: String = chars.into_iter().collect();
        if self.lines[row] != line {
            self.lines[row] = line;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Insert one char at the cursor.
    pub fn insert_char(&mut self, ch: char) {
        if ch == '\n' {
            self.newline();
            return;
        }
        let mut chars = self.row_chars(self.row);
        let at = self.col.min(chars.len());
        chars.insert(at, ch);
        self.set_row(self.row, chars);
        self.col = at + 1;
    }

    /// Insert a string (honoring embedded newlines) at the cursor.
    pub fn insert_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.insert_char(ch);
        }
    }

    /// Split the current line at the cursor into two lines.
    pub fn newline(&mut self) {
        let chars = self.row_chars(self.row);
        let at = self.col.min(chars.len());
        let tail: String = chars[at..].iter().collect();
        let head: String = chars[..at].iter().collect();
        self.lines[self.row] = head;
        self.lines.insert(self.row + 1, tail);
        self.revision = self.revision.wrapping_add(1);
        self.row += 1;
        self.col = 0;
    }

    /// Delete the grapheme before the cursor (joining lines at column 0).
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let mut chars = self.row_chars(self.row);
            let start = previous_grapheme_boundary(&self.lines[self.row], self.col);
            chars.drain(start..self.col);
            self.col = start;
            self.set_row(self.row, chars);
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&cur);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Delete the grapheme at the cursor (joining the next line at line end).
    pub fn delete(&mut self) {
        let mut chars = self.row_chars(self.row);
        if self.col < chars.len() {
            let end = next_grapheme_boundary(&self.lines[self.row], self.col);
            chars.drain(self.col..end);
            self.set_row(self.row, chars);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn clamp_col(&mut self) {
        self.col = grapheme_boundary_at_or_before(&self.lines[self.row], self.col);
    }

    /// Move one grapheme left, wrapping to the end of the previous line.
    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.col = previous_grapheme_boundary(&self.lines[self.row], self.col);
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
        }
    }

    /// Move one grapheme right, wrapping to the start of the next line.
    pub fn move_right(&mut self) {
        let len = self.lines[self.row].chars().count();
        if self.col < len {
            self.col = next_grapheme_boundary(&self.lines[self.row], self.col);
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    /// Move to the previous line (clamping column), or to line start at the top.
    pub fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.clamp_col();
        } else {
            self.col = 0;
        }
    }

    /// Move to the next line (clamping column), or to line end at the bottom.
    pub fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.clamp_col();
        } else {
            self.col = self.lines[self.row].chars().count();
        }
    }

    /// Move the cursor to the start of the current line.
    pub fn move_home(&mut self) {
        self.col = 0;
    }

    /// Move the cursor to the end of the current line.
    pub fn move_end(&mut self) {
        self.col = self.lines[self.row].chars().count();
    }

    /// The column of the previous word start on the current line: skip trailing
    /// whitespace, then the word itself. Used by word-move and word-delete.
    fn prev_word_col(&self) -> usize {
        let chars = self.row_chars(self.row);
        let mut i = self.col.min(chars.len());
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    /// The column of the next word end on the current line: skip leading
    /// whitespace, then the word itself.
    fn next_word_col(&self) -> usize {
        let chars = self.row_chars(self.row);
        let len = chars.len();
        let mut i = self.col.min(len);
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    /// Move to the previous word boundary, crossing to the prior line at col 0.
    pub fn move_word_left(&mut self) {
        if self.col == 0 {
            self.move_left();
            return;
        }
        self.col = self.prev_word_col();
    }

    /// Move to the next word boundary, crossing to the next line at line end.
    pub fn move_word_right(&mut self) {
        if self.col >= self.lines[self.row].chars().count() {
            self.move_right();
            return;
        }
        self.col = self.next_word_col();
    }

    /// Delete from the cursor back to the previous word boundary (joins the
    /// prior line when already at col 0).
    pub fn delete_word_left(&mut self) {
        if self.col == 0 {
            self.backspace();
            return;
        }
        let start = self.prev_word_col();
        let mut chars = self.row_chars(self.row);
        chars.drain(start..self.col);
        self.col = start;
        self.set_row(self.row, chars);
    }

    /// Delete from the cursor forward to the next word boundary (joins the next
    /// line when already at line end).
    pub fn delete_word_right(&mut self) {
        let len = self.lines[self.row].chars().count();
        if self.col >= len {
            self.delete();
            return;
        }
        let end = self.next_word_col();
        let mut chars = self.row_chars(self.row);
        chars.drain(self.col..end);
        self.set_row(self.row, chars);
    }

    /// Delete from the cursor to the end of the line; at line end, joins the
    /// next line (emacs `C-k`).
    pub fn kill_to_line_end(&mut self) {
        let mut chars = self.row_chars(self.row);
        if self.col < chars.len() {
            chars.truncate(self.col);
            self.set_row(self.row, chars);
        } else {
            self.delete();
        }
    }

    /// Delete from the start of the line to the cursor (emacs `C-u`).
    pub fn kill_to_line_start(&mut self) {
        let chars = self.row_chars(self.row);
        let tail: Vec<char> = chars[self.col.min(chars.len())..].to_vec();
        self.set_row(self.row, tail);
        self.col = 0;
    }

    /// Apply an input event according to the configured [`TextInputMode`].
    ///
    /// Returns [`InputOutcome::Ignored`](crate::InputOutcome::Ignored) when the
    /// event is ignored, [`InputOutcome::Consumed`](crate::InputOutcome::Consumed)
    /// when a recognized edit or navigation is already at its bound,
    /// [`InputOutcome::Changed`](crate::InputOutcome::Changed) when text or the
    /// cursor changes, or
    /// [`InputOutcome::Submitted`](crate::InputOutcome::Submitted) when the Enter chord requested submission
    /// (the buffer is left unchanged so the host can read [`Self::text`] and
    /// clear).
    ///
    /// Enter / Shift+Enter follow [`TextInputMode`]. Ctrl+J (emacs newline,
    /// and the raw-mode encoding of a bare LF that many terminals send for
    /// Shift+Enter without the kitty keyboard protocol) always inserts a
    /// newline.
    ///
    /// Beyond the plain keys, an emacs-style keymap covers the readline bindings
    /// a terminal composer is expected to honor (so the widget matches what
    /// `ratatui-textarea` gave hosts before): `C-a`/`C-e` line start/end,
    /// `C-f`/`C-b` char move, `C-p`/`C-n` line move, `C-h`/`C-d` delete,
    /// `C-j` newline, `C-k`/`C-u` kill to line end/start, `C-w`/`M-Backspace`
    /// delete word back, `M-f`/`M-b` word move, `M-d` delete word forward.
    pub fn handle(&mut self, event: &Event) -> crate::InputOutcome {
        let before = (self.row, self.col, self.revision);
        match self.handle_event(event) {
            Some(TextInputEvent::Changed) if (self.row, self.col, self.revision) == before => {
                crate::InputOutcome::Consumed
            }
            Some(TextInputEvent::Changed) => crate::InputOutcome::Changed,
            Some(TextInputEvent::Submit) => crate::InputOutcome::Submitted,
            None => crate::InputOutcome::Ignored,
        }
    }

    fn handle_event(&mut self, event: &Event) -> Option<TextInputEvent> {
        match event {
            Event::Key(k) if k.ctrl && !k.alt => match k.code {
                KeyCode::Char('a') => {
                    self.move_home();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('e') => {
                    self.move_end();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('f') => {
                    self.move_right();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('b') => {
                    self.move_left();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('p') => {
                    self.move_up();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('n') => {
                    self.move_down();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('h') => {
                    self.backspace();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('d') => {
                    self.delete();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('j') => {
                    // Emacs newline; also raw-mode LF from terminals that map
                    // Shift+Enter to a bare `\n` without enhanced keyboard reporting.
                    self.newline();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('k') => {
                    self.kill_to_line_end();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('u') => {
                    self.kill_to_line_start();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('w') => {
                    self.delete_word_left();
                    Some(TextInputEvent::Changed)
                }
                _ => None,
            },
            Event::Key(k) if k.alt && !k.ctrl => match k.code {
                KeyCode::Char('f') => {
                    self.move_word_right();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('b') => {
                    self.move_word_left();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Char('d') => {
                    self.delete_word_right();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Backspace => {
                    self.delete_word_left();
                    Some(TextInputEvent::Changed)
                }
                _ => None,
            },
            Event::Key(k) if !k.ctrl && !k.alt => match k.code {
                KeyCode::Char(c) => {
                    self.insert_char(c);
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Enter => Some(self.enter_event(k.shift)),
                KeyCode::Backspace => {
                    self.backspace();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Delete => {
                    self.delete();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Left => {
                    self.move_left();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Right => {
                    self.move_right();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Up => {
                    self.move_up();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Down => {
                    self.move_down();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::Home => {
                    self.move_home();
                    Some(TextInputEvent::Changed)
                }
                KeyCode::End => {
                    self.move_end();
                    Some(TextInputEvent::Changed)
                }
                _ => None,
            },
            Event::Paste(text) => {
                self.insert_str(text);
                Some(TextInputEvent::Changed)
            }
            _ => None,
        }
    }

    /// Every token in the buffer matched by any of `triggers`, in reading order.
    ///
    /// Scanning is left to right and the first trigger that matches at a
    /// position wins, so overlapping declarations resolve by their order in
    /// `triggers`.
    pub fn tokens(&self, triggers: &[Trigger]) -> Vec<Token> {
        let mut out = Vec::new();
        for (row, line) in self.lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let mut col = 0;
            while col < chars.len() {
                let Some(trigger) = triggers
                    .iter()
                    .find(|t| t.start == chars[col] && t.anchored_at(&chars, row, col))
                else {
                    col += 1;
                    continue;
                };
                let mut end = col + 1;
                if trigger.stop_at_whitespace {
                    while end < chars.len() && !chars[end].is_whitespace() {
                        end += 1;
                    }
                } else {
                    end = chars.len();
                }
                out.push(Token {
                    trigger: trigger.start,
                    row,
                    start: col,
                    end,
                    text: chars[col..end].iter().collect(),
                });
                col = end.max(col + 1);
            }
        }
        out
    }

    /// The token the cursor is inside, if any — what a host opens a completion
    /// popup for.
    ///
    /// The cursor counts as inside from just after the trigger character
    /// through the token's end, so typing `@s` keeps the popup open and moving
    /// left onto the `@` itself closes it.
    pub fn active_token(&self, triggers: &[Trigger]) -> Option<Token> {
        self.tokens(triggers)
            .into_iter()
            .find(|t| t.row == self.row && self.col > t.start && self.col <= t.end)
    }

    /// Replace `token`'s range with `replacement`, leaving the cursor after it.
    ///
    /// This is completion: the host picked a row in its popup and hands back the
    /// text — `"@src/lib.rs "` for a file, `"/model "` for a command. The
    /// replacement is inserted verbatim, trailing space included or not, because
    /// only the host knows whether its token type wants one.
    pub fn replace_token(&mut self, token: &Token, replacement: &str) {
        let Some(line) = self.lines.get_mut(token.row) else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let start = token.start.min(chars.len());
        let end = token.end.min(chars.len()).max(start);
        let mut next: String = chars[..start].iter().collect();
        next.push_str(replacement);
        next.extend(chars[end..].iter());
        *line = next;
        self.row = token.row;
        self.col = start + replacement.chars().count();
    }

    /// Number of visual rows the text occupies at `width`.
    pub fn visual_height(&self, width: u16) -> u16 {
        wrap_visual_rows(&self.lines, width).len().max(1) as u16
    }

    /// The cursor's visual `(row, col)` in wrapped coordinates at `width`.
    fn visual_cursor(&self, width: u16) -> (u16, u16) {
        visual_cursor_at(&self.lines, self.row, self.col, width)
    }

    /// The visual-row scroll offset that keeps the cursor visible in a
    /// `height`-row viewport at `width`: once the text is taller than the
    /// viewport the cursor rests on the last visible row; otherwise 0. A bounded
    /// composer (see [`TextInput`]) renders and places its cursor through this.
    pub fn scroll_offset(&self, width: u16, height: u16) -> u16 {
        self.visual_cursor(width)
            .0
            .saturating_sub(height.saturating_sub(1))
    }

    /// The cursor cell in terminal coordinates, given the rendered `area`,
    /// accounting for scroll-to-cursor when the text is taller than the area.
    pub fn cursor_screen(&self, area: Rect) -> (u16, u16) {
        let (vrow, vcol) = self.visual_cursor(area.width);
        let offset = vrow.saturating_sub(area.height.saturating_sub(1));
        let x = area
            .x
            .saturating_add(vcol.min(area.width.saturating_sub(1)));
        let y = area
            .y
            .saturating_add((vrow - offset).min(area.height.saturating_sub(1)));
        (x, y)
    }
}

/// Char-index boundaries between grapheme clusters, including 0 and line end.
fn grapheme_boundaries(line: &str) -> Vec<usize> {
    let mut boundaries = Vec::with_capacity(line.graphemes(true).count() + 1);
    boundaries.push(0);
    let mut col = 0;
    for grapheme in line.graphemes(true) {
        col += grapheme.chars().count();
        boundaries.push(col);
    }
    boundaries
}

fn grapheme_boundary_at_or_before(line: &str, col: usize) -> usize {
    let col = col.min(line.chars().count());
    grapheme_boundaries(line)
        .into_iter()
        .take_while(|boundary| *boundary <= col)
        .last()
        .unwrap_or(0)
}

fn previous_grapheme_boundary(line: &str, col: usize) -> usize {
    grapheme_boundaries(line)
        .into_iter()
        .take_while(|boundary| *boundary < col)
        .last()
        .unwrap_or(0)
}

fn next_grapheme_boundary(line: &str, col: usize) -> usize {
    grapheme_boundaries(line)
        .into_iter()
        .find(|boundary| *boundary > col)
        .unwrap_or_else(|| line.chars().count())
}

/// One terminal grapheme cell and its logical char-index range.
#[derive(Clone, Copy)]
struct VisualCell<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    width: u16,
}

/// One wrapped visual row: its logical line, char-index range, grapheme cells,
/// and measured terminal-cell width.
struct VisualRow<'a> {
    logical: usize,
    start: usize,
    end: usize,
    cells: Vec<VisualCell<'a>>,
    width: u16,
}

/// The cursor's visual `(row, col)` for `lines` with the logical cursor at
/// `(row, col)`, word-soft-wrapped to `width`. Reuses [`wrap_visual_rows`] so the
/// rendered scroll offset and the placed cursor always agree with what's drawn.
fn visual_cursor_at(lines: &[String], row: usize, col: usize, width: u16) -> (u16, u16) {
    let rows = wrap_visual_rows(lines, width);
    let mut last_on_line: Option<(usize, &VisualRow)> = None;
    for (vi, vr) in rows.iter().enumerate() {
        if vr.logical > row {
            break;
        }
        if vr.logical != row {
            continue;
        }
        last_on_line = Some((vi, vr));
        // A col at this row's end belongs to the next row's start, so only claim
        // it here when it falls strictly inside — except the line's last row,
        // handled below.
        if col >= vr.start && col < vr.end {
            let visual_col = vr
                .cells
                .iter()
                .take_while(|cell| cell.end <= col)
                .map(|cell| cell.width)
                .fold(0, u16::saturating_add);
            return (vi as u16, visual_col);
        }
    }
    // Cursor at the end of the logical line: rest at the end of its last row
    // (which is an empty trailing row when the text filled the width exactly).
    if let Some((vi, vr)) = last_on_line {
        return (vi as u16, vr.width);
    }
    (rows.len().saturating_sub(1) as u16, 0)
}

/// Word-soft-wrap `lines` to `width`: each logical line breaks at the last space
/// that fits, falling back to a hard grapheme break for a word longer than
/// `width`. Width is measured in terminal cells, not Unicode scalar count.
/// A line whose final row fills the width exactly emits a trailing empty row so
/// the cursor can rest on a fresh line. Shared by [`visual_cursor_at`] (cursor
/// math) and [`TextInput`] (rendering) so both wrap identically.
fn wrap_visual_rows<'a>(lines: &'a [String], width: u16) -> Vec<VisualRow<'a>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (r, line) in lines.iter().enumerate() {
        let mut char_col = 0;
        let cells: Vec<VisualCell<'a>> = line
            .graphemes(true)
            .map(|grapheme| {
                let start = char_col;
                char_col += grapheme.chars().count();
                VisualCell {
                    text: grapheme,
                    start,
                    end: char_col,
                    width: grapheme_cols(grapheme),
                }
            })
            .collect();
        if cells.is_empty() {
            rows.push(VisualRow {
                logical: r,
                start: 0,
                end: 0,
                cells: Vec::new(),
                width: 0,
            });
            continue;
        }
        let mut start = 0;
        let mut last_filled = false;
        while start < cells.len() {
            let mut hard_end = start;
            let mut hard_width = 0u16;
            while hard_end < cells.len() {
                let next_width = hard_width.saturating_add(cells[hard_end].width);
                if hard_end > start && next_width > width {
                    break;
                }
                hard_width = next_width;
                hard_end += 1;
                if hard_width >= width {
                    break;
                }
            }

            let end = if hard_end == cells.len() {
                hard_end
            } else {
                (start + 1..hard_end)
                    .rev()
                    .find(|&i| cells[i].text.chars().all(char::is_whitespace))
                    .map(|i| i + 1)
                    .unwrap_or(hard_end)
            };
            let row_width = cells[start..end]
                .iter()
                .map(|cell| cell.width)
                .fold(0, u16::saturating_add);
            last_filled = row_width == width;
            rows.push(VisualRow {
                logical: r,
                start: cells[start].start,
                end: cells[end - 1].end,
                cells: cells[start..end].to_vec(),
                width: row_width,
            });
            start = end;
        }
        if last_filled {
            rows.push(VisualRow {
                logical: r,
                start: char_col,
                end: char_col,
                cells: Vec::new(),
                width: 0,
            });
        }
    }
    rows
}

/// Strictly single-line editing state for search fields and command bars.
///
/// Newline input from setters, paste, Ctrl+J, or direct character insertion is
/// normalized to one space. Enter always submits. Unlike
/// [`TextInputState::text`], [`text`](Self::text) returns a borrowed string.
#[derive(Clone, Debug, Default)]
pub struct SingleLineInputState {
    inner: TextInputState,
}

impl SingleLineInputState {
    /// Create an empty single-line input.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed from text, normalizing every line boundary to one space.
    pub fn from_text(text: &str) -> Self {
        let mut state = Self::new();
        state.set_text(text);
        state
    }

    /// Borrow the current text without allocating.
    pub fn text(&self) -> &str {
        &self.inner.lines[0]
    }

    /// Whether the input is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Cursor column as a logical character index.
    pub fn cursor(&self) -> usize {
        self.inner.col
    }

    /// Replace the text, normalizing line boundaries to spaces.
    pub fn set_text(&mut self, text: &str) {
        self.inner.set_text(&normalize_single_line(text));
    }

    /// Clear the input.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Insert one character, normalizing CR/LF to a space.
    pub fn insert_char(&mut self, ch: char) {
        self.inner
            .insert_char(if matches!(ch, '\r' | '\n') { ' ' } else { ch });
    }

    /// Insert text, normalizing every line boundary to one space.
    pub fn insert_str(&mut self, text: &str) {
        self.inner.insert_str(&normalize_single_line(text));
    }

    /// Access the underlying state for [`TextInput`] rendering.
    pub fn as_text_input(&self) -> &TextInputState {
        &self.inner
    }

    /// Handle editing input. Enter and Ctrl+J always submit.
    pub fn handle(&mut self, event: &Event) -> crate::InputOutcome {
        match event {
            Event::Paste(text) => {
                let before = (self.inner.row, self.inner.col, self.inner.revision);
                self.insert_str(text);
                if (self.inner.row, self.inner.col, self.inner.revision) == before {
                    crate::InputOutcome::Consumed
                } else {
                    crate::InputOutcome::Changed
                }
            }
            Event::Key(key)
                if key.plain()
                    && matches!(key.code, KeyCode::Enter | KeyCode::Char('\n' | '\r')) =>
            {
                crate::InputOutcome::Submitted
            }
            Event::Key(key)
                if key.ctrl && !key.alt && !key.shift && key.code == KeyCode::Char('j') =>
            {
                crate::InputOutcome::Submitted
            }
            _ => self.inner.handle(event),
        }
    }
}

fn normalize_single_line(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                normalized.push(' ');
            }
            '\n' => normalized.push(' '),
            _ => normalized.push(ch),
        }
    }
    normalized
}

/// Renders a snapshot of a [`TextInputState`]'s wrapped text.
///
/// Owns its lines (cloned from the state at construction, like [`Scroll`]) so it
/// is `'static` and composes into a [`view!`](crate::view!) tree. When the text
/// is taller than the render area it **scrolls to the cursor** (the cursor's
/// visual row stays on screen), so it backs a bounded composer without losing the
/// caret. The host places the terminal cursor through
/// [`TextInputState::cursor_screen`], which derives the same offset.
///
/// [`Scroll`]: crate::components::Scroll
///
/// ![textinput demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/textinput.gif)
pub struct TextInput {
    lines: Vec<String>,
    /// Logical cursor `(row, col)`, so a bounded render can scroll to it.
    cursor: (usize, usize),
    style: Style,
    /// Host-supplied styled ranges painted over `style`.
    highlights: Vec<TextSpan>,
    /// Shown, in its own style, while the buffer is empty.
    placeholder: Option<(String, Style)>,
}

impl TextInput {
    /// Snapshot `state`'s text and cursor for rendering.
    pub fn new(state: &TextInputState) -> Self {
        Self {
            lines: state.lines.clone(),
            cursor: (state.row, state.col),
            style: Style::default(),
            highlights: Vec::new(),
            placeholder: None,
        }
    }

    /// Set the text style applied to rendered glyphs.
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Paint `spans` over the base style — a mention in one color, a command in
    /// another, an unknown path struck through.
    ///
    /// The ranges are the host's to compute; [`Token::span`] turns a trigger
    /// match into one directly. Where spans overlap, the last one wins.
    ///
    /// ```
    /// use tuika::ui::{Color, Style};
    /// use tuika::prelude::*;
    ///
    /// let state = TextInputState::from_text("ship @docs/readme.md");
    /// let mention = Style::default().fg(Color::Cyan);
    /// let view = TextInput::new(&state).highlights(
    ///     state.tokens(&[Trigger::new('@')]).iter().map(|t| t.span(mention)).collect(),
    /// );
    /// # let _ = view;
    /// ```
    pub fn highlights(mut self, spans: Vec<TextSpan>) -> Self {
        self.highlights = spans;
        self
    }

    /// Text drawn in place of an empty buffer, in its own style.
    ///
    /// The cursor still sits at the start of the input, so the host places it
    /// through [`TextInputState::cursor_screen`] exactly as when there is text.
    pub fn placeholder(mut self, text: impl Into<String>, style: Style) -> Self {
        self.placeholder = Some((text.into(), style));
        self
    }

    /// Style for the char at `(row, col)`: the base, patched by every covering
    /// highlight in order.
    fn style_at(&self, row: usize, col: usize) -> Style {
        self.highlights
            .iter()
            .filter(|span| span.covers(row, col))
            .fold(self.style, |style, span| style.patch(span.style))
    }

    /// Whether the buffer is a single empty line (so the placeholder shows).
    fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Visual-row offset that keeps the cursor on screen in `height` rows.
    fn scroll_offset(&self, width: u16, height: u16) -> u16 {
        visual_cursor_at(&self.lines, self.cursor.0, self.cursor.1, width)
            .0
            .saturating_sub(height.saturating_sub(1))
    }
}

impl View for TextInput {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let height = wrap_visual_rows(&self.lines, available.width).len().max(1) as u16;
        Size::new(available.width, height)
    }

    fn render(&self, area: Rect, surface: &mut Surface, _ctx: &RenderCtx) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        if let Some((text, style)) = &self.placeholder
            && self.is_empty()
        {
            surface.set_string(area.x, area.y, text, *style);
            return;
        }
        let offset = self.scroll_offset(area.width, area.height) as usize;
        for (i, vr) in wrap_visual_rows(&self.lines, area.width)
            .into_iter()
            .enumerate()
            .skip(offset)
        {
            let y = area.y.saturating_add((i - offset) as u16);
            if y >= area.bottom() {
                break;
            }
            let mut x = area.x;
            for cell in vr.cells {
                if cell.width == 0 || x >= area.right() {
                    continue;
                }
                surface.set_string(x, y, cell.text, self.style_at(vr.logical, cell.start));
                x = x.saturating_add(cell.width);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, Key, KeyCode};
    use crate::geometry::Rect;
    use crate::style::Color;
    use crate::style::Theme;
    use crate::surface::Surface;
    use crate::tests::support::{buffer, render_el, render_view_rows};
    use crate::view::{RenderCtx, element};

    #[test]
    fn single_line_normalizes_setter_and_paste_newlines() {
        let mut state = SingleLineInputState::from_text("alpha\r\nbeta\ngamma\rdelta");
        assert_eq!(state.text(), "alpha beta gamma delta");
        assert_eq!(
            state.handle(&Event::Paste(" one\r\ntwo\nthree".into())),
            crate::InputOutcome::Changed
        );
        assert_eq!(state.text(), "alpha beta gamma delta one two three");
    }

    #[test]
    fn single_line_text_is_borrowed_and_cursor_stays_on_one_row() {
        let mut state = SingleLineInputState::new();
        state.insert_str("café");
        let borrowed: &str = state.text();
        assert_eq!(borrowed, "café");
        assert_eq!(state.cursor(), 4);
        assert_eq!(state.as_text_input().cursor(), (0, 4));
    }

    #[test]
    fn single_line_enter_and_ctrl_j_submit_without_mutating() {
        let mut state = SingleLineInputState::from_text("query");
        for event in [
            Event::Key(Key::new(KeyCode::Enter)),
            Event::Key(Key {
                code: KeyCode::Char('j'),
                ctrl: true,
                alt: false,
                shift: false,
            }),
        ] {
            assert_eq!(state.handle(&event), crate::InputOutcome::Submitted);
            assert_eq!(state.text(), "query");
        }
    }

    fn press(state: &mut TextInputState, code: KeyCode) -> bool {
        matches!(
            state.handle(&Event::Key(Key::new(code))),
            crate::InputOutcome::Changed
        )
    }

    fn press_shift(state: &mut TextInputState, code: KeyCode) -> bool {
        matches!(
            state.handle(&Event::Key(Key {
                code,
                ctrl: false,
                alt: false,
                shift: true,
            })),
            crate::InputOutcome::Changed
        )
    }

    fn press_ctrl(state: &mut TextInputState, code: KeyCode) -> bool {
        matches!(
            state.handle(&Event::Key(Key {
                code,
                ctrl: true,
                alt: false,
                shift: false,
            })),
            crate::InputOutcome::Changed
        )
    }

    fn press_alt(state: &mut TextInputState, code: KeyCode) -> bool {
        matches!(
            state.handle(&Event::Key(Key {
                code,
                ctrl: false,
                alt: true,
                shift: false,
            })),
            crate::InputOutcome::Changed
        )
    }

    fn type_str(state: &mut TextInputState, s: &str) {
        for ch in s.chars() {
            assert!(press(state, KeyCode::Char(ch)));
        }
    }

    #[test]
    fn text_input_starts_empty() {
        let state = TextInputState::new();
        assert!(state.is_empty());
        assert_eq!(state.text(), "");
        assert_eq!(state.cursor(), (0, 0));
        assert_eq!(state.line_count(), 1);
    }

    #[test]
    fn text_input_types_and_edits() {
        let mut state = TextInputState::new();
        type_str(&mut state, "helo");
        assert_eq!(state.text(), "helo");
        assert_eq!(state.cursor(), (0, 4));

        // Move back and insert the missing 'l' → "hello".
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Left);
        assert_eq!(state.cursor(), (0, 2));
        assert!(press(&mut state, KeyCode::Char('l')));
        assert_eq!(state.text(), "hello");
        assert_eq!(state.cursor(), (0, 3));
    }

    #[test]
    fn text_input_backspace_and_delete() {
        let mut state = TextInputState::from_text("abc");
        assert_eq!(state.cursor(), (0, 3));
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.text(), "ab");
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Delete);
        assert_eq!(state.text(), "b");
        assert_eq!(state.cursor(), (0, 0));
    }

    #[test]
    fn text_input_newline_splits_and_backspace_joins() {
        let mut state = TextInputState::from_text("abcd");
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Right);
        assert_eq!(state.cursor(), (0, 2));
        // Default mode submits on Enter; Shift+Enter inserts the newline.
        assert!(press_shift(&mut state, KeyCode::Enter));
        assert_eq!(state.text(), "ab\ncd");
        assert_eq!(state.line_count(), 2);
        assert_eq!(state.cursor(), (1, 0));

        // Backspace at column 0 rejoins the two logical lines.
        press(&mut state, KeyCode::Backspace);
        assert_eq!(state.text(), "abcd");
        assert_eq!(state.cursor(), (0, 2));
        assert_eq!(state.line_count(), 1);
    }

    #[test]
    fn text_input_vertical_movement_clamps_column() {
        let mut state = TextInputState::from_text("longline\nhi");
        // Cursor is at end of "hi" (row 1, col 2). Move up onto the longer line:
        // column is preserved (2), not clamped, because "longline" is longer.
        press(&mut state, KeyCode::Up);
        assert_eq!(state.cursor(), (0, 2));
        // From end of "longline" move down — clamps onto the shorter "hi".
        press(&mut state, KeyCode::End);
        assert_eq!(state.cursor(), (0, 8));
        press(&mut state, KeyCode::Down);
        assert_eq!(state.cursor(), (1, 2));
    }

    #[test]
    fn text_input_paste_inserts_multiline() {
        let mut state = TextInputState::new();
        assert_eq!(
            state.handle(&Event::Paste("one\ntwo".to_string())),
            crate::InputOutcome::Changed
        );
        assert_eq!(state.text(), "one\ntwo");
        assert_eq!(state.line_count(), 2);
        assert_eq!(state.cursor(), (1, 3));
    }

    #[test]
    fn text_input_unbound_ctrl_keys_ignored() {
        // A ctrl combo with no binding (C-z) is a no-op the host can repurpose.
        let mut state = TextInputState::from_text("x");
        assert!(!press_ctrl(&mut state, KeyCode::Char('z')));
        assert_eq!(state.text(), "x");
    }

    #[test]
    fn text_input_emacs_cursor_bindings() {
        let mut state = TextInputState::from_text("hello");
        // C-a → line start, C-e → line end, C-f/C-b → char right/left.
        assert!(press_ctrl(&mut state, KeyCode::Char('a')));
        assert_eq!(state.cursor(), (0, 0));
        assert!(press_ctrl(&mut state, KeyCode::Char('f')));
        assert_eq!(state.cursor(), (0, 1));
        assert!(press_ctrl(&mut state, KeyCode::Char('e')));
        assert_eq!(state.cursor(), (0, 5));
        assert!(press_ctrl(&mut state, KeyCode::Char('b')));
        assert_eq!(state.cursor(), (0, 4));

        // C-p / C-n move between logical lines.
        state = TextInputState::from_text("ab\ncd");
        press(&mut state, KeyCode::Home);
        assert!(press_ctrl(&mut state, KeyCode::Char('p')));
        assert_eq!(state.cursor(), (0, 0));
        assert!(press_ctrl(&mut state, KeyCode::Char('n')));
        assert_eq!(state.cursor(), (1, 0));
    }

    #[test]
    fn text_input_emacs_delete_bindings() {
        // C-h backspaces, C-d deletes forward.
        let mut state = TextInputState::from_text("abc");
        assert!(press_ctrl(&mut state, KeyCode::Char('h')));
        assert_eq!(state.text(), "ab");
        press(&mut state, KeyCode::Home);
        assert!(press_ctrl(&mut state, KeyCode::Char('d')));
        assert_eq!(state.text(), "b");
    }

    #[test]
    fn text_input_kill_to_line_end_and_start() {
        // C-k kills from the cursor to end of line.
        let mut state = TextInputState::from_text("hello world");
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Right); // cursor after "hello"
        assert!(press_ctrl(&mut state, KeyCode::Char('k')));
        assert_eq!(state.text(), "hello");
        assert_eq!(state.cursor(), (0, 5));

        // C-k at line end joins the next line.
        let mut state = TextInputState::from_text("ab\ncd");
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::End);
        assert!(press_ctrl(&mut state, KeyCode::Char('k')));
        assert_eq!(state.text(), "abcd");

        // C-u kills from line start to the cursor.
        let mut state = TextInputState::from_text("hello world");
        assert!(press_ctrl(&mut state, KeyCode::Char('u')));
        assert_eq!(state.text(), "");
        assert_eq!(state.cursor(), (0, 0));
    }

    #[test]
    fn text_input_word_move_and_delete() {
        // M-b / M-f jump by word; C-w / M-d delete a word back / forward.
        let mut state = TextInputState::from_text("foo bar baz");
        assert!(press_alt(&mut state, KeyCode::Char('b')));
        assert_eq!(state.cursor(), (0, 8)); // start of "baz"
        assert!(press_alt(&mut state, KeyCode::Char('b')));
        assert_eq!(state.cursor(), (0, 4)); // start of "bar"

        // C-w at the start of "bar" deletes the previous word "foo ".
        assert!(press_ctrl(&mut state, KeyCode::Char('w')));
        assert_eq!(state.text(), "bar baz");
        assert_eq!(state.cursor(), (0, 0));

        // M-f to the end of "bar", then M-d deletes the next word " baz".
        assert!(press_alt(&mut state, KeyCode::Char('f')));
        assert_eq!(state.cursor(), (0, 3)); // end of "bar"
        assert!(press_alt(&mut state, KeyCode::Char('d')));
        assert_eq!(state.text(), "bar");

        // M-Backspace deletes the previous word too.
        let mut state = TextInputState::from_text("alpha beta");
        assert!(press_alt(&mut state, KeyCode::Backspace));
        assert_eq!(state.text(), "alpha ");
    }

    #[test]
    fn text_input_scrolls_to_cursor_when_taller_than_area() {
        // 10 single-row lines; cursor at the end (line 9).
        let mut state = TextInputState::new();
        for i in 0..10 {
            type_str(&mut state, &format!("line{i}"));
            if i < 9 {
                state.newline();
            }
        }
        // A 3-row viewport shows the last three rows (containing the cursor), not the
        // top — the composer scrolls to the caret.
        let out = render_view_rows(&TextInput::new(&state), 10, 3);
        assert_eq!(out, vec!["line7", "line8", "line9"]);
        // The placed cursor sits on the last visible row, consistent with the scroll.
        let (_, y) = state.cursor_screen(Rect::new(0, 0, 10, 3));
        assert_eq!(y, 2);
        // Move to the top: the viewport follows the cursor back up.
        for _ in 0..9 {
            state.move_up();
        }
        let out = render_view_rows(&TextInput::new(&state), 10, 3);
        assert_eq!(out, vec!["line0", "line1", "line2"]);
        assert_eq!(state.cursor_screen(Rect::new(0, 0, 10, 3)).1, 0);
    }

    #[test]
    fn text_input_renders_wrapped_rows() {
        let mut state = TextInputState::new();
        type_str(&mut state, "abcdef");
        // Width 4 wraps "abcdef" onto two visual rows: "abcd" / "ef".
        assert_eq!(state.visual_height(4), 2);
        let out = render_view_rows(&TextInput::new(&state), 4, 2);
        assert_eq!(out[0], "abcd");
        assert_eq!(out[1], "ef");
    }

    #[test]
    fn text_input_wraps_and_places_the_cursor_by_terminal_cells() {
        let state = TextInputState::from_text("界界界");

        assert_eq!(state.visual_height(4), 2);
        assert_eq!(state.cursor_screen(Rect::new(0, 0, 4, 2)), (2, 1));
        assert_eq!(
            render_view_rows(&TextInput::new(&state), 4, 2),
            vec!["界 界", "界"]
        );

        let emoji = TextInputState::from_text("👩‍💻x");
        assert_eq!(
            render_view_rows(&TextInput::new(&emoji), 3, 2),
            vec!["👩‍💻 x", ""]
        );
    }

    #[test]
    fn text_input_moves_and_deletes_whole_graphemes() {
        let mut state = TextInputState::from_text("a\u{301}b");

        state.move_left();
        assert_eq!(state.cursor(), (0, 2));
        state.move_left();
        assert_eq!(state.cursor(), (0, 0));

        state.set_cursor(0, 2);
        state.backspace();
        assert_eq!(state.text(), "b");
        assert_eq!(state.cursor(), (0, 0));

        let mut state = TextInputState::from_text("👩‍💻x");
        state.move_left();
        assert_eq!(state.cursor(), (0, 3));
        state.backspace();
        assert_eq!(state.text(), "x");
        assert_eq!(state.cursor(), (0, 0));
    }

    #[test]
    fn text_input_word_wraps_at_spaces() {
        let mut state = TextInputState::new();
        type_str(&mut state, "hello world foo");
        // Width 8 breaks at the last space that fits, not mid-word: "hello " then
        // "world " then "foo".
        assert_eq!(state.visual_height(8), 3);
        let out = render_view_rows(&TextInput::new(&state), 8, 3);
        assert_eq!(out[0], "hello");
        assert_eq!(out[1], "world");
        assert_eq!(out[2], "foo");
    }

    #[test]
    fn text_input_hard_breaks_overlong_word() {
        let mut state = TextInputState::new();
        type_str(&mut state, "abcdefghij");
        // A single word longer than the width still hard-breaks so it fits.
        assert_eq!(state.visual_height(4), 3);
        let out = render_view_rows(&TextInput::new(&state), 4, 3);
        assert_eq!(out[0], "abcd");
        assert_eq!(out[1], "efgh");
        assert_eq!(out[2], "ij");
    }

    #[test]
    fn text_input_cursor_tracks_word_wrap() {
        let mut state = TextInputState::from_text("hello world");
        // Cursor at end (col 11). Width 8 → "hello " / "world"; cursor sits after
        // "world" on the second visual row at col 5.
        let area = Rect::new(0, 0, 8, 3);
        assert_eq!(state.cursor_screen(area), (5, 1));
        // Move to just after the space (col 6, start of "world") → row 1, col 0.
        state.move_home();
        for _ in 0..6 {
            state.move_right();
        }
        assert_eq!(state.cursor_screen(area), (0, 1));
    }

    #[test]
    fn text_input_cursor_screen_follows_wrap() {
        let mut state = TextInputState::new();
        type_str(&mut state, "abcd");
        // At width 4 the line fills exactly, so the cursor rests on a fresh row.
        assert_eq!(state.visual_height(4), 2);
        let area = Rect::new(2, 1, 4, 3);
        // Cursor after "abcd" → visual row 1, col 0, offset by area origin.
        assert_eq!(state.cursor_screen(area), (2, 2));
        // Move home → back to the first visual row.
        press(&mut state, KeyCode::Home);
        assert_eq!(state.cursor_screen(area), (2, 1));
    }

    #[test]
    fn text_input_set_and_clear() {
        let mut state = TextInputState::new();
        state.set_text("hello\nworld");
        assert_eq!(state.cursor(), (1, 5));
        assert_eq!(state.line_count(), 2);
        state.clear();
        assert!(state.is_empty());
        assert_eq!(state.cursor(), (0, 0));
    }

    #[test]
    fn text_input_set_cursor_clamps() {
        let mut state = TextInputState::from_text("hi\nthere");
        state.set_cursor(0, 1);
        assert_eq!(state.cursor(), (0, 1));
        // Row past the end clamps to the last line; col past its end clamps too.
        state.set_cursor(9, 9);
        assert_eq!(state.cursor(), (1, 5));
    }

    #[test]
    fn text_input_composes_into_view_tree() {
        // Owning its snapshot makes TextInput `'static`, so it splices into a
        // `view!` tree via `element(...)` — the property the fullscreen composer
        // relies on.
        let mut state = TextInputState::new();
        type_str(&mut state, "hi");
        let tree = element(TextInput::new(&state));
        let out = render_el(&tree, 4, 1);
        assert_eq!(out[0], "hi");
    }

    #[test]
    fn submit_on_enter_mode_uses_shift_enter_for_newline() {
        let mut state = TextInputState::new();
        assert_eq!(state.mode(), TextInputMode::SubmitOnEnter);
        assert_eq!(state.handle_enter(false), crate::InputOutcome::Submitted);
        assert_eq!(state.handle_enter(true), crate::InputOutcome::Changed);
        assert_eq!(state.text(), "\n");
    }

    #[test]
    fn submit_on_shift_enter_mode_reverses_enter_chords() {
        let mut state = TextInputState::new();
        state.set_mode(TextInputMode::SubmitOnShiftEnter);
        assert_eq!(state.handle_enter(false), crate::InputOutcome::Changed);
        assert_eq!(state.handle_enter(true), crate::InputOutcome::Submitted);
        assert_eq!(state.text(), "\n");
    }

    /// Reproduction: hosts that feed every key through [`TextInputState::handle`]
    /// (the natural component API) must still honor [`TextInputMode`]. Shift+Enter
    /// in the default `SubmitOnEnter` mode has to insert a newline — not submit,
    /// and not be ignored — without the host special-casing Enter.
    #[test]
    fn handle_honors_submit_on_enter_mode_for_shift_enter_newline() {
        let mut state = TextInputState::new();
        assert_eq!(state.mode(), TextInputMode::SubmitOnEnter);
        type_str(&mut state, "one");
        let shift_enter = Event::Key(Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: false,
            shift: true,
        });
        assert_eq!(
            state.handle(&shift_enter),
            crate::InputOutcome::Changed,
            "Shift+Enter must insert a newline under SubmitOnEnter"
        );
        assert_eq!(state.text(), "one\n");
        type_str(&mut state, "two");
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter))),
            crate::InputOutcome::Submitted,
            "plain Enter must submit under SubmitOnEnter, not insert another newline"
        );
        assert_eq!(
            state.text(),
            "one\ntwo",
            "submit must leave the draft text intact"
        );
    }

    #[test]
    fn handle_honors_submit_on_shift_enter_mode() {
        let mut state = TextInputState::new();
        state.set_mode(TextInputMode::SubmitOnShiftEnter);
        type_str(&mut state, "one");
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter))),
            crate::InputOutcome::Changed
        );
        assert_eq!(state.text(), "one\n");
        type_str(&mut state, "two");
        let shift_enter = Event::Key(Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: false,
            shift: true,
        });
        assert_eq!(state.handle(&shift_enter), crate::InputOutcome::Submitted);
        assert_eq!(state.text(), "one\ntwo");
    }

    #[test]
    fn clear_preserves_enter_mode() {
        let mut state = TextInputState::new();
        state.set_mode(TextInputMode::SubmitOnShiftEnter);
        type_str(&mut state, "draft");
        state.clear();
        assert!(state.is_empty());
        assert_eq!(state.mode(), TextInputMode::SubmitOnShiftEnter);
    }

    /// Many terminals (VS Code / Cursor integrated terminal, classic xterm with
    /// no kitty keyboard protocol) encode Shift+Enter as a bare LF byte. In raw
    /// mode crossterm surfaces that as Ctrl+J — emacs newline — which must insert
    /// a newline rather than being ignored.
    #[test]
    fn handle_ctrl_j_inserts_newline() {
        let mut state = TextInputState::new();
        type_str(&mut state, "ab");
        assert!(press_ctrl(&mut state, KeyCode::Char('j')));
        assert_eq!(state.text(), "ab\n");
        assert_eq!(state.cursor(), (1, 0));
    }

    // -- triggers, tokens, and highlighting ---------------------------------

    #[test]
    fn tokens_respect_each_trigger_anchor() {
        let state = TextInputState::from_text("/model gpt\nsee @src/lib.rs and me@example.com");
        let triggers = [
            Trigger::new('/').anchor(TriggerAnchor::BufferStart),
            Trigger::new('@'),
        ];
        let found = state.tokens(&triggers);
        // `/` counts only as the buffer's first char; `@` only at a word start,
        // so the address's `@` is not a mention.
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].trigger, found[0].text.as_str()), ('/', "/model"));
        assert_eq!(
            (found[1].trigger, found[1].text.as_str(), found[1].row),
            ('@', "@src/lib.rs", 1)
        );
    }

    #[test]
    fn a_trigger_can_span_a_query_with_spaces() {
        let state = TextInputState::from_text("/model gpt 5");
        let to_end = [Trigger::new('/')
            .anchor(TriggerAnchor::BufferStart)
            .to_line_end()];
        let tokens = state.tokens(&to_end);
        assert_eq!(tokens[0].text, "/model gpt 5");
        assert_eq!(tokens[0].query(), "model gpt 5");
    }

    #[test]
    fn active_token_follows_the_cursor() {
        let mut state = TextInputState::from_text("ship @doc");
        let triggers = [Trigger::new('@')];
        // Cursor at the end, inside the mention.
        assert_eq!(state.active_token(&triggers).unwrap().query(), "doc");

        // Moving onto the `@` itself leaves the token: a popup would close.
        state.set_cursor(0, 5);
        assert!(state.active_token(&triggers).is_none());

        // Just after the trigger, with nothing typed yet, is still inside it.
        state.set_cursor(0, 6);
        assert_eq!(state.active_token(&triggers).unwrap().query(), "doc");
    }

    #[test]
    fn replace_token_completes_in_place() {
        let mut state = TextInputState::from_text("ship @doc now");
        state.set_cursor(0, 9);
        let token = state.active_token(&[Trigger::new('@')]).unwrap();
        state.replace_token(&token, "@docs/readme.md");
        assert_eq!(state.text(), "ship @docs/readme.md now");
        // Cursor lands after the replacement, ready to keep typing.
        assert_eq!(state.cursor(), (0, 20));
    }

    #[test]
    fn highlights_style_only_their_range() {
        let state = TextInputState::from_text("hi @you");
        let mention = Style::default().fg(Color::Blue);
        let spans: Vec<TextSpan> = state
            .tokens(&[Trigger::new('@')])
            .iter()
            .map(|t| t.span(mention))
            .collect();
        let view = TextInput::new(&state)
            .style(Style::default().fg(Color::White))
            .highlights(spans);

        let theme = Theme::default();
        let mut buf = buffer(8, 1);
        let area = buf.area;
        let ctx = RenderCtx::new(&theme);
        view.render(area, &mut Surface::new(&mut buf, area), &ctx);
        assert_eq!(buf[(0, 0)].fg, Color::White); // 'h'
        assert_eq!(buf[(3, 0)].fg, Color::Blue); // '@'
        assert_eq!(buf[(6, 0)].fg, Color::Blue); // 'u'
    }

    #[test]
    fn placeholder_shows_only_while_empty() {
        let mut state = TextInputState::new();
        let dim = Style::default().fg(Color::DarkGray);
        let rows = render_view_rows(
            &TextInput::new(&state).placeholder("Ask me something", dim),
            18,
            1,
        );
        assert_eq!(rows[0].trim_end(), "Ask me something");

        state.insert_char('x');
        let rows = render_view_rows(
            &TextInput::new(&state).placeholder("Ask me something", dim),
            18,
            1,
        );
        assert_eq!(rows[0].trim_end(), "x");
    }
}
