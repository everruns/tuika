//! The render loop: [`Terminal`], [`Viewport`], and [`Frame`].
//!
//! `Terminal` owns two [`Buffer`]s and alternates between them. Each
//! [`draw`](Terminal::draw) paints the whole frame into the back buffer, diffs
//! it against the front one, and hands only the changed cells to the
//! [`Backend`]. That diff is the *only* reconciliation in tuika — it is what
//! makes rebuilding every `View` from application state each frame affordable,
//! and why there is no retained widget tree above it.
//!
//! # Viewports
//!
//! [`Viewport::Fullscreen`] owns the whole screen and follows resizes.
//! [`Viewport::Fixed`] owns a rectangle and never moves on its own.
//!
//! [`Viewport::Inline`] is the interesting one and the basis of
//! [`ScreenMode::SplitFooter`](crate::ScreenMode): the viewport is anchored to
//! the cursor row where it was created and spans the full width, with ordinary
//! terminal output continuing to scroll *above* it.
//! [`insert_before`](Terminal::insert_before) publishes a block into that
//! region, which is how a footer stays pinned while finished output becomes
//! real scrollback.

use crate::buffer::Buffer;
use crate::geometry::{Position, Rect, Size};
use crate::term::traits::{Backend, ClearType};

/// What part of the screen a [`Terminal`] draws into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Viewport {
    /// The whole terminal. Resizes automatically.
    #[default]
    Fullscreen,
    /// A full-width strip of `height` rows, anchored to the cursor row it was
    /// created at, with normal output scrolling above it.
    Inline(u16),
    /// A fixed rectangle. Never resized automatically; the host calls
    /// [`Terminal::resize`] if the region should change.
    Fixed(Rect),
}

/// How to construct a [`Terminal`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TerminalOptions {
    /// The viewport to draw into.
    pub viewport: Viewport,
}

/// One frame being composed.
///
/// Handed to the [`draw`](Terminal::draw) callback; everything painted through
/// it lands in the back buffer and is diffed when the callback returns.
pub struct Frame<'a> {
    area: Rect,
    buffer: &'a mut Buffer,
    cursor_position: Option<Position>,
    count: usize,
}

impl Frame<'_> {
    /// The region this frame covers. For an inline viewport the origin is the
    /// viewport's real screen row, not `(0, 0)`.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The cell grid to paint into.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }

    /// Show the cursor at `position` after this frame is flushed.
    ///
    /// Left unset the cursor stays hidden — which is what a full-screen app
    /// wants; a text input calls this to put the caret where it belongs.
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor_position = Some(position.into());
    }

    /// How many frames have been drawn, for animation and for tests that need a
    /// deterministic tick.
    pub fn count(&self) -> usize {
        self.count
    }
}

/// The render loop over a [`Backend`].
#[derive(Debug)]
pub struct Terminal<B: Backend> {
    backend: B,
    /// Front and back; `current` selects which is being painted.
    buffers: [Buffer; 2],
    current: usize,
    hidden_cursor: bool,
    viewport: Viewport,
    viewport_area: Rect,
    /// The screen size as of the last resize check, so `autoresize` can notice
    /// a change without re-laying out every frame.
    last_known_area: Rect,
    /// Where the cursor was left, which an inline viewport needs in order to
    /// re-anchor itself across a resize.
    last_known_cursor_pos: Position,
    frame_count: usize,
}

impl<B: Backend> Terminal<B> {
    /// A full-screen terminal over `backend`.
    ///
    /// # Errors
    ///
    /// Returns the backend's error if the screen size cannot be read.
    pub fn new(backend: B) -> Result<Self, B::Error> {
        Self::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fullscreen,
            },
        )
    }

    /// A terminal with an explicit viewport.
    ///
    /// An [`Inline`](Viewport::Inline) viewport is anchored here: the backend is
    /// asked where the cursor is, and lines are appended if there is not enough
    /// room below it for the requested height.
    ///
    /// # Errors
    ///
    /// Returns the backend's error if the screen size or cursor position cannot
    /// be read.
    pub fn with_options(mut backend: B, options: TerminalOptions) -> Result<Self, B::Error> {
        let area: Rect = match options.viewport {
            Viewport::Fullscreen | Viewport::Inline(_) => {
                let size = backend.size()?;
                Rect::new(0, 0, size.width, size.height)
            }
            Viewport::Fixed(area) => area,
        };
        let (viewport_area, cursor_pos) = match options.viewport {
            Viewport::Fullscreen => (area, Position::ORIGIN),
            Viewport::Inline(height) => {
                compute_inline_size(&mut backend, height, area.as_size(), 0)?
            }
            Viewport::Fixed(area) => (area, area.as_position()),
        };
        Ok(Self {
            backend,
            buffers: [Buffer::empty(viewport_area), Buffer::empty(viewport_area)],
            current: 0,
            hidden_cursor: false,
            viewport: options.viewport,
            viewport_area,
            last_known_area: area,
            last_known_cursor_pos: cursor_pos,
            frame_count: 0,
        })
    }

    /// The backend, for a caller that needs to emit escapes tuika does not
    /// model — an image transmission, an OSC progress update.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Mutable access to the backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// The viewport this terminal draws into.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// A [`Frame`] over the back buffer without drawing it.
    ///
    /// Useful for reading the area a frame *would* have — the split-footer code
    /// uses it to measure before publishing.
    pub fn get_frame(&mut self) -> Frame<'_> {
        let count = self.frame_count;
        Frame {
            area: self.viewport_area,
            buffer: &mut self.buffers[self.current],
            cursor_position: None,
            count,
        }
    }

    /// The current back buffer.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// The screen size in cells.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn size(&self) -> Result<Size, B::Error> {
        self.backend.size()
    }

    /// Compose one frame and flush it.
    ///
    /// Resizes first if the terminal changed size, runs `render`, diffs the
    /// result against the previous frame, writes only what changed, and then
    /// positions the cursor — hiding it unless `render` asked for it.
    ///
    /// # Errors
    ///
    /// Returns the backend's error if any stage fails.
    pub fn draw<F>(&mut self, render: F) -> Result<(), B::Error>
    where
        F: FnOnce(&mut Frame<'_>),
    {
        self.autoresize()?;

        let mut frame = self.get_frame();
        render(&mut frame);
        let cursor_position = frame.cursor_position;

        self.flush()?;

        match cursor_position {
            None => self.hide_cursor()?,
            Some(position) => {
                self.show_cursor()?;
                self.set_cursor_position(position)?;
            }
        }

        self.swap_buffers();
        self.backend.flush()?;
        self.frame_count = self.frame_count.wrapping_add(1);
        Ok(())
    }

    /// Diff the back buffer against the front one and write the difference.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn flush(&mut self) -> Result<(), B::Error> {
        let previous = &self.buffers[1 - self.current];
        let current = &self.buffers[self.current];
        let updates = previous.diff(current);
        if let Some((x, y, _)) = updates.last() {
            self.last_known_cursor_pos = Position { x: *x, y: *y };
        }
        self.backend.draw(updates.into_iter())
    }

    /// Make the back buffer current and blank the one just shown.
    fn swap_buffers(&mut self) {
        self.buffers[1 - self.current].reset();
        self.current = 1 - self.current;
    }

    /// Move the viewport to `area`, discarding both buffers.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn resize(&mut self, area: Rect) -> Result<(), B::Error> {
        let (mut next_area, cursor_to_restore) = match self.viewport {
            Viewport::Inline(height) => {
                let offset = self
                    .last_known_cursor_pos
                    .y
                    .saturating_sub(self.viewport_area.top());
                let (next_area, cursor) =
                    compute_inline_size(&mut self.backend, height, area.as_size(), offset)?;
                (next_area, Some(cursor))
            }
            Viewport::Fixed(_) | Viewport::Fullscreen => (area, None),
        };

        // A narrower screen re-wraps everything already on it, so the old rows
        // cannot be trusted as a diff baseline — start from a cleared screen.
        if next_area.width < self.viewport_area.width {
            next_area.y = 0;
            self.backend.clear_region(ClearType::All)?;
        }

        self.set_viewport_area(next_area);
        self.clear()?;
        if let Some(cursor) = cursor_to_restore {
            self.backend.set_cursor_position(cursor)?;
        }
        self.last_known_area = area;
        Ok(())
    }

    /// Resize if the terminal changed size since the last check.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn autoresize(&mut self) -> Result<(), B::Error> {
        if matches!(self.viewport, Viewport::Fullscreen | Viewport::Inline(_)) {
            let size = self.backend.size()?;
            let area = Rect::new(0, 0, size.width, size.height);
            if area != self.last_known_area {
                self.resize(area)?;
            }
        }
        Ok(())
    }

    fn set_viewport_area(&mut self, area: Rect) {
        self.buffers[self.current].resize(area);
        self.buffers[1 - self.current].resize(area);
        self.viewport_area = area;
    }

    /// Blank *this terminal's own region* and drop the diff baseline, so the
    /// next frame repaints in full.
    ///
    /// Scoping the erase to the viewport is load-bearing for an inline
    /// viewport: everything above it is the user's scrollback, including blocks
    /// [`insert_before`](Self::insert_before) just published, and a full-screen
    /// erase would wipe them.
    ///
    /// Resetting the previous buffer matters just as much — without it the next
    /// diff would compare against rows no longer on screen and elide them as
    /// unchanged, leaving the viewport blank.
    pub fn clear(&mut self) -> Result<(), B::Error> {
        match self.viewport {
            Viewport::Fullscreen => self.backend.clear_region(ClearType::All)?,
            Viewport::Inline(_) => {
                self.backend
                    .set_cursor_position(self.viewport_area.as_position())?;
                self.backend.clear_region(ClearType::AfterCursor)?;
            }
            Viewport::Fixed(area) => {
                // No escape erases an arbitrary rectangle, so blank it by
                // painting spaces over exactly the cells it owns.
                let blank = Buffer::empty(area);
                let cells: Vec<_> = blank
                    .content
                    .iter()
                    .enumerate()
                    .map(|(i, cell)| {
                        let width = area.width.max(1) as usize;
                        (
                            area.x + (i % width) as u16,
                            area.y + (i / width) as u16,
                            cell,
                        )
                    })
                    .collect();
                self.backend.draw(cells.into_iter())?;
            }
        }
        self.buffers[1 - self.current].reset();
        Ok(())
    }

    /// Hide the cursor.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn hide_cursor(&mut self) -> Result<(), B::Error> {
        self.backend.hide_cursor()?;
        self.hidden_cursor = true;
        Ok(())
    }

    /// Show the cursor.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn show_cursor(&mut self) -> Result<(), B::Error> {
        self.backend.show_cursor()?;
        self.hidden_cursor = false;
        Ok(())
    }

    /// Where the cursor is.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn get_cursor_position(&mut self) -> Result<Position, B::Error> {
        self.backend.get_cursor_position()
    }

    /// Move the cursor.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), B::Error> {
        let position = position.into();
        self.backend.set_cursor_position(position)?;
        self.last_known_cursor_pos = position;
        Ok(())
    }

    /// Publish `height` rows of content directly above an inline viewport,
    /// pushing earlier output into scrollback.
    ///
    /// A no-op for any other viewport: there is nothing "above" a full-screen
    /// app. This is the mechanism behind
    /// [`Scrollback`](crate::Scrollback) — finished output becomes real
    /// terminal scrollback the user can select and scroll, while the footer
    /// stays pinned below it.
    ///
    /// The implementation is deliberately the *portable* one: draw the block,
    /// scroll only as far as needed, then repaint the viewport. A DECSTBM
    /// scrolling region would avoid the repaint, but rows leaving such a region
    /// are discarded by the terminal rather than entering scrollback — which
    /// would defeat the whole point. See the `scrolling-regions` note on
    /// [`ScreenMode`](crate::ScreenMode).
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub fn insert_before<F>(&mut self, height: u16, draw: F) -> Result<(), B::Error>
    where
        F: FnOnce(&mut Buffer),
    {
        if !matches!(self.viewport, Viewport::Inline(_)) {
            return Ok(());
        }
        #[cfg(feature = "scrolling-regions")]
        return self.insert_before_scrolling_regions(height, draw);
        #[cfg(not(feature = "scrolling-regions"))]
        self.insert_before_portable(height, draw)
    }

    /// Publish by drawing the block and repainting the viewport beneath it.
    ///
    /// Portable to any terminal, and the only path that puts the displaced rows
    /// into scrollback.
    #[cfg_attr(feature = "scrolling-regions", allow(dead_code))]
    fn insert_before_portable<F>(&mut self, height: u16, draw: F) -> Result<(), B::Error>
    where
        F: FnOnce(&mut Buffer),
    {
        let width = self.viewport_area.width;
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        draw(&mut buffer);
        let mut cells = buffer.content.as_slice();

        // Signed arithmetic throughout: the intermediate sums genuinely go
        // negative, and clamping them at zero as u16 would silently mis-scroll.
        let mut drawn_height = i32::from(self.viewport_area.top());
        let mut remaining = i32::from(height);
        let viewport_height = i32::from(self.viewport_area.height);
        let screen_height = i32::from(self.last_known_area.height);

        // Draw a screenful at a time until what is left of the block plus the
        // viewport fits, so the tail can go out in one final write.
        while remaining + viewport_height > screen_height {
            let to_draw = remaining.min(screen_height);
            let scroll_up = 0.max(drawn_height + to_draw - screen_height);
            self.scroll_up(scroll_up as u16)?;
            cells = self.draw_lines((drawn_height - scroll_up) as u16, to_draw as u16, cells)?;
            drawn_height += to_draw - scroll_up;
            remaining -= to_draw;
        }

        // Scroll just enough that the viewport ends up back at the bottom.
        let scroll_up = 0.max(drawn_height + remaining + viewport_height - screen_height);
        self.scroll_up(scroll_up as u16)?;
        self.draw_lines((drawn_height - scroll_up) as u16, remaining as u16, cells)?;
        drawn_height += remaining - scroll_up;

        self.set_viewport_area(Rect {
            y: drawn_height as u16,
            ..self.viewport_area
        });

        // Cleared only now: the block's cells are dense, so they already
        // overwrote the screen, and clearing before scrolling pushes garbage
        // into tmux's scrollback.
        self.clear()?;
        Ok(())
    }

    /// Publish using DECSTBM scrolling regions, which move only the rows above
    /// the viewport and so leave the footer painted.
    ///
    /// Faster than the portable path — no viewport repaint — but the rows it
    /// displaces are *discarded* by the terminal rather than entering
    /// scrollback. That trade is why this is behind a feature; see the
    /// `scrolling-regions` note on [`ScreenMode`](crate::ScreenMode).
    #[cfg(feature = "scrolling-regions")]
    fn insert_before_scrolling_regions<F>(
        &mut self,
        mut height: u16,
        draw: F,
    ) -> Result<(), B::Error>
    where
        F: FnOnce(&mut Buffer),
    {
        let width = self.viewport_area.width;
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        draw(&mut buffer);
        let mut cells = buffer.content.as_slice();

        // A viewport filling the screen leaves no region to scroll into, so
        // borrow its top row: draw one line there, scroll that single row into
        // scrollback, repeat, then repaint the row from the front buffer.
        if self.viewport_area.height == self.last_known_area.height {
            let mut first = true;
            while !cells.is_empty() {
                cells = if first {
                    self.draw_lines(0, 1, cells)?
                } else {
                    self.draw_lines_over_cleared(0, 1, cells)?
                };
                first = false;
                self.backend.scroll_region_up(0..1, 1)?;
            }
            let width = self.viewport_area.width as usize;
            let top = self.buffers[1 - self.current].content[0..width].to_vec();
            self.draw_lines_over_cleared(0, 1, &top)?;
            return Ok(());
        }

        // While there is room below, push the viewport down and fill the gap
        // rather than scrolling anything off the top.
        {
            let top = self.viewport_area.top();
            let bottom = self.viewport_area.bottom();
            let screen_bottom = self.last_known_area.bottom();
            if bottom < screen_bottom {
                let to_draw = height.min(screen_bottom - bottom);
                self.backend
                    .scroll_region_down(top..bottom + to_draw, to_draw)?;
                cells = self.draw_lines_over_cleared(top, to_draw, cells)?;
                self.set_viewport_area(Rect {
                    y: top + to_draw,
                    ..self.viewport_area
                });
                height -= to_draw;
            }
        }

        // The rest scrolls the region above the viewport up.
        let top = self.viewport_area.top();
        while height > 0 && top > 0 {
            let to_draw = height.min(top);
            self.backend.scroll_region_up(0..top, to_draw)?;
            cells = self.draw_lines_over_cleared(top - to_draw, to_draw, cells)?;
            height -= to_draw;
        }
        Ok(())
    }

    /// Like [`draw_lines`](Self::draw_lines) but diffing against blank cells, so
    /// a row the terminal just cleared is written in full.
    #[cfg(feature = "scrolling-regions")]
    fn draw_lines_over_cleared<'a>(
        &mut self,
        y: u16,
        count: u16,
        cells: &'a [crate::buffer::Cell],
    ) -> Result<&'a [crate::buffer::Cell], B::Error> {
        let width = self.last_known_area.width as usize;
        let split = (width * count as usize).min(cells.len());
        let (to_draw, remainder) = cells.split_at(split);
        if count > 0 && width > 0 {
            let area = Rect::new(0, y, width as u16, count);
            let blank = Buffer::empty(area);
            let next = Buffer {
                area,
                content: to_draw.to_vec(),
            };
            let updates = blank.diff(&next);
            self.backend.draw(updates.into_iter())?;
            self.backend.flush()?;
        }
        Ok(remainder)
    }

    /// Write `count` rows of `cells` starting at screen row `y`.
    fn draw_lines<'a>(
        &mut self,
        y: u16,
        count: u16,
        cells: &'a [crate::buffer::Cell],
    ) -> Result<&'a [crate::buffer::Cell], B::Error> {
        let width = self.last_known_area.width as usize;
        let split = (width * count as usize).min(cells.len());
        let (to_draw, remainder) = cells.split_at(split);
        if count > 0 && width > 0 {
            let updates = to_draw
                .iter()
                .enumerate()
                .map(|(i, cell)| ((i % width) as u16, y + (i / width) as u16, cell));
            self.backend.draw(updates)?;
            self.backend.flush()?;
        }
        Ok(remainder)
    }

    /// Scroll the screen up by `lines`, pushing the top into scrollback.
    fn scroll_up(&mut self, lines: u16) -> Result<(), B::Error> {
        if lines > 0 {
            self.backend.set_cursor_position(Position::new(
                0,
                self.last_known_area.height.saturating_sub(1),
            ))?;
            self.backend.append_lines(lines)?;
        }
        Ok(())
    }
}

/// Where an inline viewport of `height` rows should sit, given the cursor.
///
/// Appends lines when the cursor is too near the bottom for the viewport to
/// fit, and reports the cursor position from *before* that scroll so the caller
/// can put it back. `offset_in_previous_viewport` keeps a resize anchored to the
/// same relative row rather than jumping.
fn compute_inline_size<B: Backend>(
    backend: &mut B,
    height: u16,
    size: Size,
    offset_in_previous_viewport: u16,
) -> Result<(Rect, Position), B::Error> {
    let position = backend.get_cursor_position()?;
    let mut row = position.y;
    let max_height = size.height.min(height);

    let lines_after_cursor = height
        .saturating_sub(offset_in_previous_viewport)
        .saturating_sub(1);
    backend.append_lines(lines_after_cursor)?;

    let available = size.height.saturating_sub(row).saturating_sub(1);
    let missing = lines_after_cursor.saturating_sub(available);
    if missing > 0 {
        row = row.saturating_sub(missing);
    }
    row = row.saturating_sub(offset_in_previous_viewport);

    Ok((
        Rect {
            x: 0,
            y: row,
            width: size.width,
            height: max_height,
        },
        position,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Style;
    use crate::term::testbackend::TestBackend;

    fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(width, height)).unwrap()
    }

    #[test]
    fn a_new_terminal_covers_the_whole_screen() {
        let terminal = terminal(10, 4);
        assert_eq!(terminal.viewport_area, Rect::new(0, 0, 10, 4));
        assert_eq!(terminal.viewport(), Viewport::Fullscreen);
    }

    #[test]
    fn draw_paints_into_the_backend() {
        let mut terminal = terminal(5, 1);
        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.buffer_mut().set_string(0, 0, "hi", Style::default());
                assert_eq!(area, Rect::new(0, 0, 5, 1));
            })
            .unwrap();
        assert_eq!(terminal.backend().rows(), vec!["hi   ".to_string()]);
    }

    /// The property the whole immediate-mode model rests on: painting the same
    /// content twice writes nothing the second time.
    #[test]
    fn an_unchanged_frame_writes_nothing() {
        let mut terminal = terminal(4, 1);
        let paint = |frame: &mut Frame<'_>| {
            frame.buffer_mut().set_string(0, 0, "ab", Style::default());
        };
        terminal.draw(paint).unwrap();

        // Paint the same content into the back buffer but stop short of
        // flushing, so the two buffers can be compared the way `flush` would.
        {
            let mut frame = terminal.get_frame();
            paint(&mut frame);
        }
        let previous = &terminal.buffers[1 - terminal.current];
        let current = &terminal.buffers[terminal.current];
        assert!(
            previous.diff(current).is_empty(),
            "an identical frame must produce no writes"
        );
    }

    #[test]
    fn a_changed_cell_is_written() {
        let mut terminal = terminal(3, 1);
        terminal
            .draw(|f| {
                f.buffer_mut().set_string(0, 0, "ab", Style::default());
            })
            .unwrap();
        terminal
            .draw(|f| {
                f.buffer_mut().set_string(0, 0, "xb", Style::default());
            })
            .unwrap();
        assert_eq!(terminal.backend().rows(), vec!["xb ".to_string()]);
    }

    #[test]
    fn the_cursor_is_hidden_unless_the_frame_asks_for_it() {
        let mut terminal = terminal(3, 1);
        terminal.draw(|_| {}).unwrap();
        assert!(!terminal.backend().cursor_visible());

        terminal
            .draw(|frame| frame.set_cursor_position((1, 0)))
            .unwrap();
        assert!(terminal.backend().cursor_visible());
        assert_eq!(
            terminal.backend_mut().get_cursor_position().unwrap(),
            Position::new(1, 0)
        );
    }

    #[test]
    fn frame_count_advances() {
        let mut terminal = terminal(2, 1);
        assert_eq!(terminal.get_frame().count(), 0);
        terminal.draw(|_| {}).unwrap();
        assert_eq!(terminal.get_frame().count(), 1);
    }

    #[test]
    fn autoresize_follows_the_backend() {
        let mut terminal = terminal(4, 2);
        terminal.backend_mut().resize(6, 3);
        terminal.autoresize().unwrap();
        assert_eq!(terminal.viewport_area, Rect::new(0, 0, 6, 3));
    }

    #[test]
    fn a_fixed_viewport_does_not_autoresize() {
        let backend = TestBackend::new(10, 10);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(2, 2, 3, 3)),
            },
        )
        .unwrap();
        terminal.backend_mut().resize(20, 20);
        terminal.autoresize().unwrap();
        assert_eq!(terminal.viewport_area, Rect::new(2, 2, 3, 3));
    }

    /// An inline viewport starts at the cursor row when there is room below it.
    #[test]
    fn an_inline_viewport_anchors_to_the_cursor_row() {
        let mut backend = TestBackend::new(10, 10);
        backend.set_cursor_position((0, 3)).unwrap();
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(4),
            },
        )
        .unwrap();
        assert_eq!(terminal.viewport_area, Rect::new(0, 3, 10, 4));
    }

    /// Near the bottom there is not enough room, so the screen scrolls and the
    /// viewport lands higher than the cursor row it was asked for.
    #[test]
    fn an_inline_viewport_scrolls_to_make_room() {
        let mut backend = TestBackend::new(10, 6);
        backend.set_cursor_position((0, 5)).unwrap();
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(3),
            },
        )
        .unwrap();
        assert_eq!(terminal.viewport_area, Rect::new(0, 3, 10, 3));
    }

    #[test]
    fn an_inline_viewport_is_clamped_to_the_screen_height() {
        let backend = TestBackend::new(4, 2);
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(9),
            },
        )
        .unwrap();
        assert_eq!(terminal.viewport_area.height, 2);
    }

    /// The split-footer contract: the published block ends up in scrollback and
    /// the viewport stays the same height, below it.
    #[test]
    fn insert_before_publishes_above_an_inline_viewport() {
        let mut backend = TestBackend::new(6, 6);
        backend.set_cursor_position((0, 3)).unwrap();
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(2),
            },
        )
        .unwrap();
        terminal
            .draw(|f| {
                f.buffer_mut().set_string(0, 0, "FOOT", Style::default());
            })
            .unwrap();

        terminal
            .insert_before(1, |buffer| {
                buffer.set_string(0, 0, "BLOCK", Style::default());
            })
            .unwrap();

        assert_eq!(terminal.viewport_area.height, 2, "footer keeps its height");
        let published = terminal
            .backend()
            .rows()
            .iter()
            .any(|row| row.starts_with("BLOCK"))
            || terminal
                .backend()
                .scrollback()
                .iter()
                .any(|row| row.first().is_some_and(|c| c.symbol() == "B"));
        assert!(published, "the block must reach the screen or scrollback");
    }

    #[test]
    fn insert_before_is_a_no_op_for_a_fullscreen_viewport() {
        let mut terminal = terminal(4, 2);
        let before = terminal.viewport_area;
        terminal
            .insert_before(1, |buffer| {
                buffer.set_string(0, 0, "x", Style::default());
            })
            .unwrap();
        assert_eq!(terminal.viewport_area, before);
    }

    #[test]
    fn clear_drops_the_diff_baseline_so_the_next_frame_repaints() {
        let mut terminal = terminal(3, 1);
        terminal
            .draw(|f| {
                f.buffer_mut().set_string(0, 0, "ab", Style::default());
            })
            .unwrap();
        terminal.clear().unwrap();
        assert!(
            terminal.buffers[1 - terminal.current]
                .content
                .iter()
                .all(|cell| cell.symbol() == " "),
            "the previous buffer must be blank after clear"
        );
    }

    #[test]
    fn a_zero_size_screen_does_not_panic() {
        let mut terminal = Terminal::new(TestBackend::new(0, 0)).unwrap();
        terminal.draw(|_| {}).unwrap();
        assert_eq!(terminal.viewport_area, Rect::new(0, 0, 0, 0));
    }
}
