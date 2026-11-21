//! The runner side of screen-reader mode: paint off screen, speak the changes.
//!
//! When [`RunnerConfig::screen_reader`](super::RunnerConfig::screen_reader) is
//! active, the loop is unchanged — the same ticks, events, redraw scheduling,
//! and `update` contract — but its terminal is a [`SinkBackend`] that keeps the
//! cells in memory, and each frame's [`Document`] is narrated to stdout as
//! plain lines. Keeping the loop shared is the point: an application behaves
//! identically in both modes, and only what reaches the terminal differs.

use std::cell::RefCell;
use std::io::{self, Write};

use crate::access::{AccessFrame, Document, Narrator};
use crate::buffer::{Buffer, Cell};
use crate::geometry::{Position, Size};
use crate::host::{MouseCapture, TerminalSessionConfig};
use crate::screen::ScreenMode;
use crate::term::traits::{Backend, ClearType, WindowSize};
use crate::view::RenderCtx;

/// The session a screen-reader run enters: raw input for keys, and nothing a
/// screen reader would trip over — no alternate screen (its review buffer
/// stays the real scrollback), no mouse capture, and a visible cursor.
pub(crate) fn session_config() -> TerminalSessionConfig {
    TerminalSessionConfig {
        // The split-footer mode is the one that leaves the main screen alone;
        // no footer is ever pinned, because the frame is never painted there.
        screen_mode: ScreenMode::split_footer(1),
        raw_mode: true,
        enhanced_keyboard: true,
        mouse_capture: MouseCapture::Disabled,
        hide_cursor: false,
    }
}

/// Per-run screen-reader state: the collector a frame records into, the
/// narrator that turns documents into speech, and where speech goes.
///
/// The loop that drives it is generic over its backend's error type, so a
/// write failure cannot travel up through it. The first one is kept instead,
/// later writes are skipped, and the entry point returns it once the loop
/// ends — a closed stdout ends the run as an error either way.
pub(crate) struct ScreenReaderSink {
    frame: AccessFrame,
    narrator: RefCell<Narrator>,
    out: RefCell<Box<dyn Write>>,
    error: RefCell<Option<io::Error>>,
}

impl ScreenReaderSink {
    pub(crate) fn new(out: impl Write + 'static) -> Self {
        Self {
            frame: AccessFrame::new(),
            narrator: RefCell::new(Narrator::new()),
            out: RefCell::new(Box::new(out)),
            error: RefCell::new(None),
        }
    }

    /// Clear the collector and install it on `ctx` for the coming paint.
    pub(crate) fn install<'a>(&'a self, ctx: RenderCtx<'a>) -> RenderCtx<'a> {
        self.frame.clear();
        ctx.with_access(&self.frame)
    }

    /// Read the painted frame back and speak what changed.
    pub(crate) fn capture(&self, buffer: &Buffer) {
        let document = self.frame.document(buffer);
        self.write(|narrator, out| narrator.narrate(&document, out));
    }

    /// Speak blocks published to the scrollback, whole and in order.
    pub(crate) fn announce(&self, documents: &[Document]) {
        for document in documents {
            self.write(|narrator, out| narrator.announce(document, out));
        }
    }

    /// Commit any open input line before the shell prompt returns, and report
    /// the first write failure of the run.
    pub(crate) fn finish(&self) -> io::Result<()> {
        self.write(|narrator, out| narrator.finish(out));
        self.error.borrow_mut().take().map_or(Ok(()), Err)
    }

    fn write(&self, speak: impl FnOnce(&mut Narrator, &mut dyn Write) -> io::Result<()>) {
        if self.error.borrow().is_some() {
            return;
        }
        let mut out = self.out.borrow_mut();
        if let Err(error) = speak(&mut self.narrator.borrow_mut(), &mut **out) {
            *self.error.borrow_mut() = Some(error);
        }
    }
}

/// A backend that draws nowhere and reports the real terminal's size.
///
/// The [`Terminal`](crate::term::terminal::Terminal) above it still diffs and
/// keeps the frame buffer, so layout sees the true geometry and a resize is
/// picked up the ordinary way; nothing is written, because the narrator owns
/// stdout in this mode.
#[derive(Debug, Default)]
pub(crate) struct SinkBackend;

impl Backend for SinkBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, _content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(Position::new(0, 0))
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, _position: P) -> io::Result<()> {
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> io::Result<()> {
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        // Without a tty (piped output, CI) there is no size to read; a
        // conventional 80×24 keeps layout meaningful instead of failing.
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        Ok(Size::new(width.max(1), height.max(1)))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size()?,
            pixels: Size::new(0, 0),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_up(&mut self, _region: core::ops::Range<u16>, _lines: u16) -> io::Result<()> {
        Ok(())
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_down(
        &mut self,
        _region: core::ops::Range<u16>,
        _lines: u16,
    ) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{Node, Role};
    use crate::geometry::Rect;
    use crate::style::Theme;

    #[test]
    fn the_session_leaves_the_main_screen_mouse_and_cursor_alone() {
        let config = session_config();
        assert!(!config.screen_mode.is_alternate());
        assert!(!config.captures_mouse());
        assert!(!config.hide_cursor);
        assert!(config.raw_mode);
    }

    #[derive(Clone, Default)]
    struct Shared(std::rc::Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_sink_speaks_each_change_once() {
        let theme = Theme::default();
        let out = Shared::default();
        let sink = ScreenReaderSink::new(out.clone());
        let buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        for _ in 0..2 {
            let ctx = sink.install(RenderCtx::new(&theme));
            ctx.record_access(|| Node::new(Role::Button, "OK").area(Rect::new(0, 0, 2, 1)));
            sink.capture(&buffer);
        }
        sink.finish().unwrap();
        assert_eq!(out.0.borrow().as_slice(), b"OK, button\r\n");
    }

    #[test]
    fn a_write_failure_is_kept_and_reported_at_the_end() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let theme = Theme::default();
        let sink = ScreenReaderSink::new(Closed);
        let buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        let ctx = sink.install(RenderCtx::new(&theme));
        ctx.record_access(|| Node::new(Role::Button, "OK").area(Rect::new(0, 0, 2, 1)));
        sink.capture(&buffer);
        let error = sink.finish().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
