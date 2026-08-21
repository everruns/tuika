//! The [`Backend`] boundary: what [`Terminal`](crate::term::terminal::Terminal)
//! needs from whatever is actually driving the screen.
//!
//! Two implementations ship with tuika:
//! [`CrosstermBackend`](super::backend::CrosstermBackend) writes real escape
//! sequences, and [`TestBackend`](super::testbackend::TestBackend) keeps a cell
//! grid in memory so the suite can assert on rendered output without a
//! terminal. A host can add its own — the trait is public precisely so
//! `Runner::run_on` can drive one.
//!
//! The trait is deliberately small. Everything above it — layout, overlays,
//! diffing — is tuika's; everything below is escape sequences.

use crate::buffer::Cell;
use crate::geometry::{Position, Size};

/// How much of the screen [`Backend::clear_region`] should erase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClearType {
    /// Every cell of the visible area.
    All,
    /// From the cursor (inclusive) to the end of the screen.
    AfterCursor,
    /// From the start of the screen to the cursor (inclusive).
    BeforeCursor,
    /// The cursor's whole line.
    CurrentLine,
    /// From the cursor (inclusive) to the end of its line.
    UntilNewLine,
}

/// The terminal's size in both cells and pixels.
///
/// Both come from one syscall, and the pixel size is what
/// [`term::image`](crate::term::image) needs to scale a picture to a cell box —
/// so they are reported together rather than through two calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowSize {
    /// Size in character cells.
    pub columns_rows: Size,
    /// Size in pixels. Terminals that do not report this return `0×0`.
    pub pixels: Size,
}

/// Something that can paint cells onto a screen.
///
/// Implementors own escape sequences and cursor state; they are handed only the
/// cells that changed, already diffed.
pub trait Backend {
    /// How this backend fails. `io::Error` for anything writing to a terminal;
    /// [`Infallible`](core::convert::Infallible) for an in-memory one.
    type Error: core::error::Error;

    /// Paint `content` — the `(x, y, cell)` triples a diff produced.
    ///
    /// Coordinates are absolute screen positions and arrive in row-major order,
    /// which is what lets an implementation elide a cursor move for a
    /// contiguous run.
    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>;

    /// Scroll `n` blank lines in at the bottom, pushing the top into
    /// scrollback. This is how an inline viewport publishes a block.
    fn append_lines(&mut self, _n: u16) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Hide the cursor.
    fn hide_cursor(&mut self) -> Result<(), Self::Error>;

    /// Show the cursor.
    fn show_cursor(&mut self) -> Result<(), Self::Error>;

    /// Where the cursor currently is.
    ///
    /// On a real terminal this is a *query* — it writes a request and blocks on
    /// the reply — which is how an inline viewport learns the row it must
    /// anchor itself to.
    fn get_cursor_position(&mut self) -> Result<Position, Self::Error>;

    /// Move the cursor.
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error>;

    /// Erase the visible area, leaving the cursor where it is.
    fn clear(&mut self) -> Result<(), Self::Error>;

    /// Erase the part of the screen `clear_type` names.
    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error>;

    /// The screen size in cells.
    fn size(&self) -> Result<Size, Self::Error>;

    /// The screen size in cells and pixels.
    fn window_size(&mut self) -> Result<WindowSize, Self::Error>;

    /// Push anything buffered out to the screen.
    ///
    /// Distinct from [`Terminal::flush`](crate::term::terminal::Terminal::flush),
    /// which computes the diff; this one only empties the write buffer.
    fn flush(&mut self) -> Result<(), Self::Error>;

    /// Scroll rows `region` up by `line_count`, pushing rows off the top into
    /// scrollback when the region starts at row 0.
    ///
    /// Optional: the default reports that the backend does not implement it, and
    /// callers fall back to a portable repaint. See the `scrolling-regions`
    /// note in [`ScreenMode`](crate::ScreenMode) for why the fallback is the
    /// right default under a split footer.
    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_up(
        &mut self,
        region: core::ops::Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error>;

    /// Scroll rows `region` down by `line_count`. Rows pushed off the bottom are
    /// discarded — terminals do not put them in scrollback.
    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_down(
        &mut self,
        region: core::ops::Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error>;
}
