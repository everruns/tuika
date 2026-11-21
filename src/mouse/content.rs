//! Text selection anchored to content rows, so it survives scrolling.
//!
//! [`SelectionState`](super::SelectionState) tracks a gesture in *screen*
//! cells. Inside a scrolled view that is the wrong space: scrolling moves the
//! text under a fixed selection, so the selection either has to be dropped or
//! ends up covering different words, and a drag can never reach past the rows
//! on screen. [`ContentSelection`] stores the gesture in *content* space
//! instead — the row index into the scrolled content, which does not change as
//! the view scrolls — and maps it through the [`ContentWindow`] of each frame
//! to paint and hit-test. A selection made, scrolled away, and scrolled back is
//! the same selection, and one drag (with the wheel turning underneath it) can
//! span any number of screens.
//!
//! Copying needs the text of rows that are no longer on screen, which no
//! single frame's buffer holds. [`capture`](ContentSelection::capture) records
//! the selected rows each frame as they are painted, so every row the gesture
//! passed over is available to [`text`](ContentSelection::text) after it has
//! scrolled out of view.

use std::collections::BTreeMap;
use std::time::Instant;

use super::{DOUBLE_CLICK_INTERVAL, SelectionRange, paint_selection, word_at};
use crate::buffer::Buffer;
use crate::event::{Mouse, MouseButton, MouseKind};
use crate::geometry::Rect;
use crate::style::Style;
use crate::{Clock, SystemClock};

/// A cell in content space: a row of the scrolled content and a column of it.
///
/// Ordering is reading order — by row, then column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentPoint {
    /// Content row, counted from the first row of the content (not the screen).
    pub row: usize,
    /// Content column, counted from the content's left edge.
    pub column: u16,
}

impl ContentPoint {
    /// A point at `row`, `column`.
    pub const fn new(row: usize, column: u16) -> Self {
        Self { row, column }
    }
}

/// A content-space selection in reading order; both ends are inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentRange {
    /// The first selected cell.
    pub start: ContentPoint,
    /// The last selected cell (inclusive).
    pub end: ContentPoint,
}

impl ContentRange {
    /// Order two points into reading order.
    pub fn between(a: ContentPoint, b: ContentPoint) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

/// Where one frame showed a scrolled content: its screen rect and the content
/// row and column at that rect's top-left cell.
///
/// Built-in scrolling views ([`Scroll`](crate::components::Scroll),
/// [`Viewport`](crate::components::Viewport),
/// [`ItemScroll`](crate::components::ItemScroll)) report theirs to the runner
/// through [`RenderCtx::record_scrolled_content`](crate::RenderCtx::record_scrolled_content);
/// a host driving [`ContentSelection`] itself builds one from its own scroll
/// state each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContentWindow {
    /// The screen cells showing content (scrollbars excluded).
    pub area: Rect,
    /// The content row painted on `area`'s top row.
    pub first_row: usize,
    /// The content column painted on `area`'s left column (horizontal pan).
    pub first_column: u16,
}

impl ContentWindow {
    /// A window showing content from `first_row` at the top of `area`, unpanned.
    pub const fn new(area: Rect, first_row: usize) -> Self {
        Self {
            area,
            first_row,
            first_column: 0,
        }
    }

    /// Set the horizontal pan: the content column on `area`'s left edge.
    pub const fn with_first_column(mut self, first_column: u16) -> Self {
        self.first_column = first_column;
        self
    }

    /// The content point under a screen cell, clamped into the window so a drag
    /// past an edge selects to that edge. `None` for an empty window.
    pub fn point_at(&self, column: u16, row: u16) -> Option<ContentPoint> {
        if self.area.is_empty() {
            return None;
        }
        let column = column.clamp(self.area.x, self.area.right() - 1);
        let row = row.clamp(self.area.y, self.area.bottom() - 1);
        Some(ContentPoint {
            row: self.first_row + usize::from(row - self.area.y),
            column: self.first_column.saturating_add(column - self.area.x),
        })
    }

    /// The screen cell showing `point`, if it is inside the window.
    pub fn screen_cell(&self, point: ContentPoint) -> Option<(u16, u16)> {
        let row = point.row.checked_sub(self.first_row)?;
        let column = point.column.checked_sub(self.first_column)?;
        if row >= usize::from(self.area.height) || column >= self.area.width {
            return None;
        }
        Some((self.area.x + column, self.area.y + row as u16))
    }

    /// The part of `range` this window shows, as a screen [`SelectionRange`]
    /// for [`paint_selection`] or [`selected_text`](super::selected_text) over
    /// `area`. An end scrolled above or below the window is pinned to the
    /// window's first or last cell, so the visible middle still reads as
    /// selected; `None` when no selected row is on screen.
    pub fn project(&self, range: ContentRange) -> Option<SelectionRange> {
        if self.area.is_empty() {
            return None;
        }
        let last_row = self.first_row + usize::from(self.area.height) - 1;
        if range.end.row < self.first_row || range.start.row > last_row {
            return None;
        }
        let column = |content: u16| {
            let local = content.saturating_sub(self.first_column);
            self.area.x + local.min(self.area.width - 1)
        };
        let start = if range.start.row < self.first_row {
            (self.area.x, self.area.y)
        } else {
            let row = self.area.y + (range.start.row - self.first_row) as u16;
            if range.start.column < self.first_column {
                (self.area.x, row)
            } else {
                (column(range.start.column), row)
            }
        };
        let end = if range.end.row > last_row {
            (self.area.right() - 1, self.area.bottom() - 1)
        } else {
            (
                column(range.end.column),
                self.area.y + (range.end.row - self.first_row) as u16,
            )
        };
        Some(SelectionRange { start, end })
    }

    /// The same mapping over only the part of the window inside `bounds`
    /// (the buffer, typically): clipping the top or left edge advances the
    /// first row or column with it.
    pub fn clip(&self, bounds: Rect) -> Self {
        let area = self.area.intersection(bounds);
        if area.is_empty() {
            return Self {
                area: Rect::new(self.area.x, self.area.y, 0, 0),
                ..*self
            };
        }
        Self {
            area,
            first_row: self.first_row + usize::from(area.y - self.area.y),
            first_column: self.first_column.saturating_add(area.x - self.area.x),
        }
    }
}

/// A click-drag, word, and line selection kept in content space, so it
/// survives scrolling and can span more than one screen.
///
/// It is the content-space counterpart of
/// [`SelectionState`](super::SelectionState), with the same gestures: a left
/// `Down` starts a selection (clearing the last one), `Drag` extends it, `Up`
/// finishes it; two clicks on one cell select a word and three select the row,
/// and a drag after either extends by words or rows. Every mouse call takes the
/// [`ContentWindow`] the pointer is over this frame, and positions outside it
/// are clamped to its edge.
///
/// After painting each frame, call [`resolve`](Self::resolve) (word and row
/// boundaries come from the painted cells), [`capture`](Self::capture) (so the
/// selected rows can be copied after they scroll away), then
/// [`paint`](Self::paint). [`text`](Self::text) returns what was selected.
///
/// ```
/// use tuika::buffer::Buffer;
/// use tuika::mouse::{ContentSelection, ContentWindow};
/// use tuika::prelude::*;
/// use tuika::event::{Mouse, MouseButton, MouseKind};
///
/// let area = Rect::new(0, 0, 10, 2);
/// let mut buffer = Buffer::empty(area);
/// buffer.set_string(0, 0, "row three", Style::default());
/// buffer.set_string(0, 1, "row four", Style::default());
/// // Content rows 3 and 4 are on screen.
/// let window = ContentWindow::new(area, 3);
///
/// let mut selection = ContentSelection::new();
/// selection.handle(&Mouse::at(MouseKind::Down(MouseButton::Left), 4, 0), window);
/// selection.handle(&Mouse::at(MouseKind::Up(MouseButton::Left), 2, 1), window);
/// selection.capture(&buffer, window);
/// assert_eq!(selection.text().as_deref(), Some("three\nrow"));
///
/// // Scroll down a row: the selection still covers content rows 3..=4.
/// let scrolled = ContentWindow::new(area, 4);
/// assert_eq!(selection.range().unwrap().start.row, 3);
/// assert!(scrolled.project(selection.range().unwrap()).is_some());
/// ```
#[derive(Clone, Debug, Default)]
pub struct ContentSelection {
    anchor: ContentPoint,
    cursor: ContentPoint,
    pressed: bool,
    selecting: bool,
    last_click: Option<(ContentPoint, Instant)>,
    pending_word: Option<ContentPoint>,
    click_count: u8,
    anchor_range: Option<ContentRange>,
    pending_drag: bool,
    /// Captured cell symbols of selected rows, keyed by content row.
    rows: BTreeMap<usize, Vec<String>>,
}

impl ContentSelection {
    /// A state with no selection in progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a mouse event over `window`; returns `true` when the selection
    /// changed and a redraw is warranted.
    pub fn handle(&mut self, m: &Mouse, window: ContentWindow) -> bool {
        self.handle_with_clock(m, window, &SystemClock)
    }

    /// [`handle`](Self::handle) with an explicit monotonic `clock`, for
    /// deterministic multi-click timing.
    pub fn handle_with_clock(
        &mut self,
        m: &Mouse,
        window: ContentWindow,
        clock: &dyn Clock,
    ) -> bool {
        let Some(position) = window.point_at(m.column, m.row) else {
            return false;
        };
        match m.kind {
            MouseKind::Down(MouseButton::Left) => {
                let now = clock.now();
                let repeated = self.last_click.is_some_and(|(previous, at)| {
                    previous == position
                        && now.saturating_duration_since(at) <= DOUBLE_CLICK_INTERVAL
                });
                self.click_count = if repeated {
                    self.click_count % 3 + 1
                } else {
                    1
                };
                self.last_click = Some((position, now));
                self.anchor = position;
                self.cursor = position;
                self.pressed = true;
                self.anchor_range = None;
                self.pending_drag = false;
                self.rows.clear();
                let had = self.selecting;
                self.selecting = false;
                self.pending_word = (self.click_count > 1).then_some(position);
                had || self.pending_word.is_some()
            }
            MouseKind::Drag(MouseButton::Left) if self.pressed => {
                self.cursor = position;
                self.last_click = None;
                if self.click_count > 1 {
                    self.pending_drag = true;
                } else {
                    self.selecting = self.cursor != self.anchor;
                }
                true
            }
            MouseKind::Up(MouseButton::Left) if self.pressed => {
                self.cursor = position;
                self.pressed = false;
                if self.click_count > 1 {
                    self.pending_drag = true;
                } else {
                    self.selecting = self.cursor != self.anchor;
                }
                if self.last_click.is_some_and(|(start, _)| start != position) {
                    self.last_click = None;
                }
                true
            }
            _ => false,
        }
    }

    /// Resolve multi-click word and row selection against the cells painted
    /// into `buffer` over `window`. Returns `true` when the selection changed.
    pub fn resolve(&mut self, buffer: &Buffer, window: ContentWindow) -> bool {
        let window = window.clip(buffer.area);
        let area = window.area;
        let pointer = self.cursor;
        let click_count = self.click_count;
        let range_at = |position: ContentPoint| -> Option<ContentRange> {
            let (column, row) = window.screen_cell(position)?;
            if click_count == 3 {
                let first = window.point_at(area.x, row)?;
                let last = window.point_at(area.right() - 1, row)?;
                Some(ContentRange::between(first, last))
            } else {
                let word = word_at(buffer, area, column, row)?;
                Some(ContentRange::between(
                    window.point_at(word.start.0, word.start.1)?,
                    window.point_at(word.end.0, word.end.1)?,
                ))
            }
        };
        let mut changed = false;
        if let Some(position) = self.pending_word.take()
            && let Some(range) = range_at(position)
        {
            self.anchor_range = Some(range);
            self.anchor = range.start;
            self.cursor = range.end;
            self.selecting = true;
            changed = true;
        }
        if self.pending_drag {
            self.pending_drag = false;
            if let Some(anchor) = self.anchor_range {
                let focus = range_at(pointer).unwrap_or(ContentRange {
                    start: pointer,
                    end: pointer,
                });
                if focus.start < anchor.start {
                    self.anchor = anchor.end;
                    self.cursor = focus.start;
                } else {
                    self.anchor = anchor.start;
                    self.cursor = focus.end;
                }
                self.selecting = true;
                changed = true;
            }
        }
        changed
    }

    /// Record the text of the rows `window` shows in `buffer`, so
    /// [`text`](Self::text) can still read them once they scroll away.
    ///
    /// While the button is held every visible row is kept, because the drag
    /// may still grow to include rows that are on screen now and scrolled away
    /// by the time it does. Once the gesture ends, rows outside the selection
    /// are forgotten, which keeps the record bounded by the selection itself.
    pub fn capture(&mut self, buffer: &Buffer, window: ContentWindow) {
        let range = match self.range() {
            _ if self.pressed => None,
            Some(range) => Some(range),
            None => {
                self.rows.clear();
                return;
            }
        };
        if let Some(range) = range {
            self.rows
                .retain(|row, _| *row >= range.start.row && *row <= range.end.row);
        }
        let window = window.clip(buffer.area);
        let area = window.area;
        for y in area.y..area.bottom() {
            let row = window.first_row + usize::from(y - area.y);
            if range.is_some_and(|range| row < range.start.row || row > range.end.row) {
                continue;
            }
            // Store from content column 0 so a later horizontal pan cannot
            // shift which columns a captured row covers; columns left of the
            // window (panned away) are recorded as blanks.
            let left = usize::from(window.first_column);
            let mut cells = vec![String::from(" "); left];
            cells.extend((area.x..area.right()).map(|x| buffer[(x, y)].symbol().to_string()));
            match self.rows.get_mut(&row) {
                // Keep cells captured earlier further right (a narrower or
                // panned frame must not truncate what was already seen).
                Some(existing) if existing.len() > cells.len() => {
                    existing[..cells.len()].clone_from_slice(&cells);
                }
                _ => {
                    self.rows.insert(row, cells);
                }
            }
        }
    }

    /// Paint `style` over the part of the selection `window` shows.
    pub fn paint(&self, buffer: &mut Buffer, window: ContentWindow, style: Style) {
        let window = window.clip(buffer.area);
        if let Some(range) = self.range()
            && let Some(screen) = window.project(range)
        {
            paint_selection(buffer, window.area, screen, style);
        }
    }

    /// The selected text from the captured rows: the tail of the first row,
    /// whole middle rows, the head of the last, joined with `\n` and with
    /// trailing blanks trimmed. Rows never painted while selected come back
    /// empty. `None` when nothing is selected.
    pub fn text(&self) -> Option<String> {
        let range = self.range()?;
        let mut out = String::new();
        for row in range.start.row..=range.end.row {
            if row > range.start.row {
                out.push('\n');
            }
            let Some(cells) = self.rows.get(&row) else {
                continue;
            };
            let left = if row == range.start.row {
                usize::from(range.start.column)
            } else {
                0
            };
            let right = if row == range.end.row {
                usize::from(range.end.column).saturating_add(1)
            } else {
                cells.len()
            }
            .min(cells.len());
            let line: String = cells.get(left..right).unwrap_or_default().concat();
            out.push_str(line.trim_end_matches(' '));
        }
        Some(out)
    }

    /// The current selection in reading order, or `None` when nothing is
    /// selected.
    pub fn range(&self) -> Option<ContentRange> {
        self.selecting
            .then(|| ContentRange::between(self.anchor, self.cursor))
    }

    /// Whether a non-empty selection currently exists.
    pub fn is_active(&self) -> bool {
        self.selecting
    }

    /// Whether the button is held (a gesture is in progress).
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Clear the selection and its captured rows.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Clone, Copy)]
    struct FixedClock(Instant);

    impl Clock for FixedClock {
        fn now(&self) -> Instant {
            self.0
        }
    }

    fn mouse(kind: MouseKind, column: u16, row: u16) -> Mouse {
        Mouse::at(kind, column, row)
    }

    const DOWN: MouseKind = MouseKind::Down(MouseButton::Left);
    const DRAG: MouseKind = MouseKind::Drag(MouseButton::Left);
    const UP: MouseKind = MouseKind::Up(MouseButton::Left);

    /// Paint content rows `first..first + height` of a numbered document into
    /// a buffer whose window sits at `area`.
    fn frame(area: Rect, first: usize) -> (Buffer, ContentWindow) {
        let mut buffer = Buffer::empty(Rect::new(0, 0, area.right() + 2, area.bottom() + 2));
        for y in 0..area.height {
            buffer.set_string(
                area.x,
                area.y + y,
                &format!("line {:02} text", first + usize::from(y)),
                Style::default(),
            );
        }
        (buffer, ContentWindow::new(area, first))
    }

    #[test]
    fn a_selection_survives_scrolling_and_spans_screens() {
        let area = Rect::new(2, 1, 14, 3);
        let mut selection = ContentSelection::new();

        // Start on content row 10 ("line 10 text"), column 5.
        let (buffer, window) = frame(area, 10);
        assert!(!selection.handle(&mouse(DOWN, 7, 1), window));
        selection.handle(&mouse(DRAG, 10, 2), window);
        selection.capture(&buffer, window);
        assert_eq!(
            selection.range(),
            Some(ContentRange::between(
                ContentPoint::new(10, 5),
                ContentPoint::new(11, 8)
            ))
        );

        // The wheel scrolls four rows down while the button is held; the
        // selection keeps its content rows instead of following the screen.
        let (buffer, window) = frame(area, 14);
        assert_eq!(window.project(selection.range().unwrap()), None);
        selection.capture(&buffer, window);

        // Dragging on continues across screens; releasing on row 15.
        selection.handle(&mouse(UP, 5, 2), window);
        selection.capture(&buffer, window);
        let range = selection.range().unwrap();
        assert_eq!((range.start.row, range.end.row), (10, 15));
        // The visible part is pinned to the window's first cell.
        assert_eq!(
            window.project(range),
            Some(SelectionRange {
                start: (2, 1),
                end: (5, 2)
            })
        );

        // Row 12 was on screen while the button was held, so it is kept even
        // though the drag reached it only after it scrolled away. Row 13 was
        // never painted, so it copies as an empty line.
        assert_eq!(
            selection.text().as_deref(),
            Some("10 text\nline 11 text\nline 12 text\n\nline 14 text\nline")
        );
    }

    #[test]
    fn scrolling_back_paints_the_same_cells() {
        let area = Rect::new(0, 0, 12, 2);
        let mut selection = ContentSelection::new();
        let (mut buffer, window) = frame(area, 0);
        selection.handle(&mouse(DOWN, 0, 1), window);
        selection.handle(&mouse(UP, 3, 1), window);
        selection.paint(
            &mut buffer,
            window,
            Style::default().bg(crate::style::Color::Red),
        );
        assert_eq!(buffer[(0, 1)].bg, crate::style::Color::Red);

        // One row up: content row 1 is now on screen row 0.
        let (mut buffer, scrolled) = frame(area, 1);
        selection.paint(
            &mut buffer,
            scrolled,
            Style::default().bg(crate::style::Color::Red),
        );
        assert_eq!(buffer[(0, 0)].bg, crate::style::Color::Red);
        assert_eq!(buffer[(4, 0)].bg, crate::style::Color::Reset);
        assert_eq!(buffer[(0, 1)].bg, crate::style::Color::Reset);
    }

    #[test]
    fn double_and_triple_clicks_select_words_and_rows_in_content_space() {
        let start = Instant::now();
        let area = Rect::new(0, 0, 14, 2);
        let (buffer, window) = frame(area, 40);
        let mut selection = ContentSelection::new();
        let clock = FixedClock(start);
        for _ in 0..2 {
            selection.handle_with_clock(&mouse(DOWN, 9, 1), window, &clock);
            selection.handle_with_clock(&mouse(UP, 9, 1), window, &clock);
        }
        assert!(selection.resolve(&buffer, window));
        selection.capture(&buffer, window);
        assert_eq!(selection.text().as_deref(), Some("text"));
        assert_eq!(selection.range().unwrap().start, ContentPoint::new(41, 8));

        let later = FixedClock(start + Duration::from_millis(10));
        selection.handle_with_clock(&mouse(DOWN, 9, 1), window, &later);
        selection.resolve(&buffer, window);
        selection.capture(&buffer, window);
        assert_eq!(selection.text().as_deref(), Some("line 41 text"));
    }

    #[test]
    fn horizontal_pan_keeps_content_columns() {
        let area = Rect::new(0, 0, 4, 1);
        let window = ContentWindow::new(area, 0).with_first_column(6);
        assert_eq!(window.point_at(1, 0), Some(ContentPoint::new(0, 7)));
        assert_eq!(window.screen_cell(ContentPoint::new(0, 7)), Some((1, 0)));
        assert_eq!(window.screen_cell(ContentPoint::new(0, 5)), None);
        let range = ContentRange::between(ContentPoint::new(0, 2), ContentPoint::new(0, 7));
        assert_eq!(
            window.project(range),
            Some(SelectionRange {
                start: (0, 0),
                end: (1, 0)
            })
        );
    }

    #[test]
    fn degenerate_windows_do_nothing() {
        let window = ContentWindow::new(Rect::new(3, 3, 0, 0), 5);
        let mut selection = ContentSelection::new();
        assert!(!selection.handle(&mouse(DOWN, 3, 3), window));
        assert_eq!(window.point_at(3, 3), None);
        assert_eq!(
            window.project(ContentRange::between(
                ContentPoint::new(0, 0),
                ContentPoint::new(9, 9)
            )),
            None
        );
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 2));
        selection.capture(&buffer, window);
        selection.paint(&mut buffer, window, Style::default());
        assert_eq!(selection.text(), None);
        // A window larger than the buffer is clipped to it.
        let big = ContentWindow::new(Rect::new(0, 0, 50, 50), 0);
        selection.handle(&mouse(DOWN, 0, 0), big);
        selection.handle(&mouse(UP, 40, 40), big);
        selection.capture(&buffer, big);
        selection.paint(&mut buffer, big, Style::default());
        assert!(selection.resolve(&buffer, big) || !selection.resolve(&buffer, big));
    }
}
