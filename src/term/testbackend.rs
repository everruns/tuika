//! An in-memory [`Backend`] for the hermetic test suite.
//!
//! Everything tuika renders is asserted by painting into a [`TestBackend`] and
//! reading cells back, so the whole suite runs without a terminal — see
//! [`testing`](crate::testing) for the consumer-facing wrappers around it.
//!
//! It is a real backend, not a stub: it applies the same cell updates a
//! terminal would, so a diffing bug shows up here rather than only under the
//! PTY smoke tests.

use std::convert::Infallible;

use crate::buffer::{Buffer, Cell};
use crate::geometry::{Position, Rect, Size};
use crate::term::traits::{Backend, ClearType, WindowSize};

/// A backend that paints into a [`Buffer`] instead of a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestBackend {
    buffer: Buffer,
    cursor: Position,
    cursor_visible: bool,
    /// Rows scrolled off the top by [`append_lines`](Backend::append_lines),
    /// oldest first — the in-memory stand-in for terminal scrollback, and what
    /// lets a split-footer test assert what was published.
    scrollback: Vec<Vec<Cell>>,
}

impl TestBackend {
    /// A blank screen `width × height` cells.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            buffer: Buffer::empty(Rect::new(0, 0, width, height)),
            cursor: Position::ORIGIN,
            cursor_visible: true,
            scrollback: Vec::new(),
        }
    }

    /// The current screen contents.
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Mutable access, for a test that needs to seed the screen.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    /// Where the cursor is, without the `&mut` a [`Backend`] query needs.
    pub fn cursor_position(&self) -> Position {
        self.cursor
    }

    /// Whether the cursor is currently shown.
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Rows that have scrolled off the top, oldest first.
    pub fn scrollback(&self) -> &[Vec<Cell>] {
        &self.scrollback
    }

    /// Resize the screen, discarding contents the way a real resize does.
    ///
    /// The cursor is clamped into the new bounds. A real terminal does this, and
    /// without it a shrink would leave the cursor off-screen — which an inline
    /// viewport then re-anchors itself against, landing the footer below the
    /// bottom row.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.buffer.resize(Rect::new(0, 0, width, height));
        self.cursor.x = self.cursor.x.min(width.saturating_sub(1));
        self.cursor.y = self.cursor.y.min(height.saturating_sub(1));
    }

    /// The screen as one string per row, trailing spaces intact.
    pub fn rows(&self) -> Vec<String> {
        (0..self.buffer.area.height)
            .map(|y| {
                (0..self.buffer.area.width)
                    .map(|x| self.buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }
}

impl Backend for TestBackend {
    /// In-memory writes cannot fail, and saying so in the type removes the
    /// error handling a test would otherwise carry.
    type Error = Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            if let Some(target) = self.buffer.cell_mut((x, y)) {
                *target = cell.clone();
            }
        }
        Ok(())
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        let height = self.buffer.area.height;
        let width = self.buffer.area.width;
        if height == 0 || n == 0 {
            return Ok(());
        }
        let shift = n.min(height);

        for y in 0..shift {
            let row: Vec<Cell> = (0..width).map(|x| self.buffer[(x, y)].clone()).collect();
            self.scrollback.push(row);
        }
        for y in 0..height.saturating_sub(shift) {
            for x in 0..width {
                let source = self.buffer[(x, y + shift)].clone();
                self.buffer[(x, y)] = source;
            }
        }
        for y in height.saturating_sub(shift)..height {
            for x in 0..width {
                self.buffer[(x, y)].reset();
            }
        }
        // A real terminal leaves the cursor on the last row after scrolling.
        self.cursor.y = height.saturating_sub(1);
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.cursor_visible = false;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.cursor_visible = true;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        let area = self.buffer.area;
        let Position { x: cx, y: cy } = self.cursor;
        let mut reset = |x: u16, y: u16| {
            if let Some(cell) = self.buffer.cell_mut((x, y)) {
                cell.reset();
            }
        };
        match clear_type {
            ClearType::All => {
                for y in 0..area.height {
                    for x in 0..area.width {
                        reset(x, y);
                    }
                }
            }
            ClearType::AfterCursor => {
                for x in cx..area.width {
                    reset(x, cy);
                }
                for y in cy.saturating_add(1)..area.height {
                    for x in 0..area.width {
                        reset(x, y);
                    }
                }
            }
            ClearType::BeforeCursor => {
                for y in 0..cy {
                    for x in 0..area.width {
                        reset(x, y);
                    }
                }
                for x in 0..=cx.min(area.width.saturating_sub(1)) {
                    reset(x, cy);
                }
            }
            ClearType::CurrentLine => {
                for x in 0..area.width {
                    reset(x, cy);
                }
            }
            ClearType::UntilNewLine => {
                for x in cx..area.width {
                    reset(x, cy);
                }
            }
        }
        Ok(())
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(self.buffer.area.as_size())
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: self.buffer.area.as_size(),
            // A cell is assumed 8×16 px, the common default. Tests that care
            // about pixel geometry state their own expectation against this.
            pixels: Size::new(
                self.buffer.area.width.saturating_mul(8),
                self.buffer.area.height.saturating_mul(16),
            ),
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_up(
        &mut self,
        region: core::ops::Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error> {
        let width = self.buffer.area.width;
        let end = region.end.min(self.buffer.area.height);
        let start = region.start.min(end);
        let rows = end - start;
        let shift = line_count.min(rows);

        // Rows leaving a region anchored at the top enter scrollback; rows
        // leaving a region below it are discarded, which is what terminals do.
        if start == 0 {
            for y in start..start + shift {
                let row: Vec<Cell> = (0..width).map(|x| self.buffer[(x, y)].clone()).collect();
                self.scrollback.push(row);
            }
        }
        for y in start..end.saturating_sub(shift) {
            for x in 0..width {
                let source = self.buffer[(x, y + shift)].clone();
                self.buffer[(x, y)] = source;
            }
        }
        for y in end.saturating_sub(shift)..end {
            for x in 0..width {
                self.buffer[(x, y)].reset();
            }
        }
        Ok(())
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_down(
        &mut self,
        region: core::ops::Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error> {
        let width = self.buffer.area.width;
        let end = region.end.min(self.buffer.area.height);
        let start = region.start.min(end);
        let rows = end - start;
        let shift = line_count.min(rows);

        for y in (start + shift..end).rev() {
            for x in 0..width {
                let source = self.buffer[(x, y - shift)].clone();
                self.buffer[(x, y)] = source;
            }
        }
        for y in start..(start + shift).min(end) {
            for x in 0..width {
                self.buffer[(x, y)].reset();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Style;

    fn seeded(width: u16, height: u16) -> TestBackend {
        let mut backend = TestBackend::new(width, height);
        for y in 0..height {
            let row = format!("{y}").repeat(width as usize);
            backend
                .buffer_mut()
                .set_string(0, y, &row, Style::default());
        }
        backend
    }

    #[test]
    fn draw_applies_cells_at_absolute_positions() {
        let mut backend = TestBackend::new(3, 2);
        let cell = Cell::new("x");
        backend.draw([(1u16, 1u16, &cell)].into_iter()).unwrap();
        assert_eq!(backend.rows(), vec!["   ".to_string(), " x ".to_string()]);
    }

    #[test]
    fn draw_ignores_cells_outside_the_screen() {
        let mut backend = TestBackend::new(2, 1);
        let cell = Cell::new("x");
        backend.draw([(9u16, 9u16, &cell)].into_iter()).unwrap();
        assert_eq!(backend.rows(), vec!["  ".to_string()]);
    }

    /// The scrollback path a split footer depends on: rows leaving the top are
    /// kept, the rest shift up, and the freed rows come back blank.
    #[test]
    fn append_lines_scrolls_the_top_into_scrollback() {
        let mut backend = seeded(2, 3);
        backend.append_lines(1).unwrap();
        assert_eq!(
            backend.rows(),
            vec!["11".to_string(), "22".to_string(), "  ".to_string()]
        );
        assert_eq!(backend.scrollback().len(), 1);
        assert_eq!(backend.scrollback()[0][0].symbol(), "0");
    }

    #[test]
    fn append_lines_past_the_screen_height_clears_it() {
        let mut backend = seeded(2, 2);
        backend.append_lines(9).unwrap();
        assert_eq!(backend.rows(), vec!["  ".to_string(), "  ".to_string()]);
        assert_eq!(backend.scrollback().len(), 2);
    }

    #[test]
    fn append_lines_of_zero_is_a_no_op() {
        let mut backend = seeded(2, 2);
        backend.append_lines(0).unwrap();
        assert_eq!(backend.rows(), vec!["00".to_string(), "11".to_string()]);
        assert!(backend.scrollback().is_empty());
    }

    #[test]
    fn clear_region_after_cursor_leaves_earlier_rows() {
        let mut backend = seeded(2, 3);
        backend.set_cursor_position((1, 1)).unwrap();
        backend.clear_region(ClearType::AfterCursor).unwrap();
        assert_eq!(
            backend.rows(),
            vec!["00".to_string(), "1 ".to_string(), "  ".to_string()]
        );
    }

    #[test]
    fn clear_region_before_cursor_leaves_later_rows() {
        let mut backend = seeded(2, 3);
        backend.set_cursor_position((0, 1)).unwrap();
        backend.clear_region(ClearType::BeforeCursor).unwrap();
        assert_eq!(
            backend.rows(),
            vec!["  ".to_string(), " 1".to_string(), "22".to_string()]
        );
    }

    #[test]
    fn clear_region_current_line_clears_only_that_row() {
        let mut backend = seeded(2, 2);
        backend.set_cursor_position((1, 0)).unwrap();
        backend.clear_region(ClearType::CurrentLine).unwrap();
        assert_eq!(backend.rows(), vec!["  ".to_string(), "11".to_string()]);
    }

    #[test]
    fn cursor_visibility_is_tracked() {
        let mut backend = TestBackend::new(1, 1);
        assert!(backend.cursor_visible());
        backend.hide_cursor().unwrap();
        assert!(!backend.cursor_visible());
        backend.show_cursor().unwrap();
        assert!(backend.cursor_visible());
    }

    #[test]
    fn resize_reports_the_new_size() {
        let mut backend = TestBackend::new(2, 2);
        backend.resize(5, 3);
        assert_eq!(backend.size().unwrap(), Size::new(5, 3));
    }

    /// A shrink must pull the cursor back on screen, or an inline viewport
    /// re-anchors against a row that no longer exists.
    #[test]
    fn resize_clamps_the_cursor_into_the_new_bounds() {
        let mut backend = TestBackend::new(10, 10);
        backend.set_cursor_position((9, 9)).unwrap();
        backend.resize(4, 3);
        assert_eq!(backend.cursor_position(), Position::new(3, 2));
    }

    #[test]
    fn resize_to_zero_does_not_underflow_the_cursor() {
        let mut backend = TestBackend::new(4, 4);
        backend.set_cursor_position((3, 3)).unwrap();
        backend.resize(0, 0);
        assert_eq!(backend.cursor_position(), Position::ORIGIN);
    }
}
