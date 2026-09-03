//! Screen modes: which part of the terminal a `tuika` frame owns.
//!
//! A full-screen renderer is not the only useful shape. [`ScreenMode`] names the
//! two `tuika` supports:
//!
//! - [`ScreenMode::Alternate`] — the terminal's alternate buffer. The frame owns
//!   the whole window and the user's scrollback is restored untouched on exit.
//!   This is the default; native terminal mouse handling stays active so OSC 8
//!   links, selection, and scrolling keep their emulator behavior.
//! - [`ScreenMode::SplitFooter`] — a reserved region pinned to the bottom of the
//!   *main* screen. The frame owns only those rows; everything above stays the
//!   terminal's own scrollback, so ordinary program output, the shell prompt
//!   that launched the app, and the user's selection and scroll-wheel history
//!   all survive.
//!
//! The split footer is how a long-running tool keeps a live composer, status
//! line, or progress panel on screen while its actual output accumulates above
//! it as plain terminal text the user can scroll back to, copy, and pipe.
//!
//! # Publishing to the scrollback
//!
//! In split-footer mode, a host must not write to stdout directly — the footer
//! owns the cursor, and a stray `println!` lands wherever the cursor happens to
//! be. [`Scrollback`] is the supported path: a cheap, cloneable, `Send + Sync`
//! handle that queues *views*, which the runner renders and commits above the
//! footer, one block at a time.
//!
//! ```no_run
//! use std::time::Duration;
//! use tuika::prelude::*;
//!
//! let runner = Runner::new(RunnerConfig {
//!     tick_rate: Duration::from_millis(100),
//!     screen_mode: ScreenMode::split_footer(6),
//! });
//! let scrollback = runner.scrollback();
//!
//! // From anywhere, including another thread: published above the footer on
//! // the next loop iteration.
//! std::thread::spawn(move || {
//!     scrollback.write(|_width| element(Text::raw("build finished in 12ms")));
//! });
//! ```
//!
//! Blocks are painted with no background fill, so unstyled cells keep the
//! terminal's own colors and published output blends with the surrounding shell
//! session rather than looking like a pasted panel.
//!
//! # Hosts driving their own loop
//!
//! [`Runner`](crate::Runner) and `AsyncRunner` do all of this already. A host
//! with its own event loop composes the same pieces: enter with
//! [`TerminalSession::enter_with`](crate::TerminalSession::enter_with), call
//! [`pin_footer`] before each frame (it re-pins after a resize), publish, and
//! call [`close_footer`] before the session is dropped.
//!
//! Such a host can also skip the queue: [`publish_block`] commits one view
//! immediately, with no `Send` bound, so a block may own frame state that could
//! never cross a thread — a transcript entry holding a streaming-markdown
//! cache, say. [`Scrollback`] is the path *into* that loop from elsewhere;
//! [`publish_block`] is the path from inside it. The
//! [`codex`](https://github.com/everruns/tuika/tree/main/examples/codex)
//! example publishes its finished transcript entries that way.

use std::sync::{Arc, Mutex};

use crate::buffer::Buffer;
use crate::term::terminal::{Terminal, Viewport};
use crate::term::traits::Backend;
use crate::text::Line;

use crate::components::Text;
use crate::geometry::Size;
use crate::style::Theme;
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, View, element};

/// Rows a split footer reserves when the host does not pick a height.
pub const DEFAULT_FOOTER_HEIGHT: u16 = 12;

/// Which part of the terminal the renderer owns.
///
/// See the [module documentation](self) for the model and the split-footer
/// contract.
///
/// <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/split-footer.gif" width="880" alt="A terminal running the split_footer example: a bordered status box pinned to the last rows while published build lines accumulate above it as ordinary scrollback; after the example exits the lines remain and the box's rows are gone.">
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenMode {
    /// Own the whole window on the terminal's alternate buffer, restoring the
    /// previous screen and scrollback on exit. Leaves mouse handling to the
    /// terminal so native OSC 8 links and selection keep working.
    #[default]
    Alternate,
    /// Alternate-screen mode with terminal mouse reporting enabled.
    ///
    /// Prefer constructing this through [`with_mouse_capture`](Self::with_mouse_capture).
    /// Capture gives the application pointer and wheel events, but prevents the
    /// terminal from providing native OSC 8 activation, selection, and scrolling.
    AlternateWithMouseCapture,
    /// Own `height` rows pinned to the bottom of the main screen, leaving the
    /// rows above as ordinary terminal scrollback.
    SplitFooter {
        /// Rows reserved for the footer, at least 1 and clamped to the terminal
        /// height by the viewport.
        height: u16,
        /// Whether to capture the mouse. Off by default: on the main screen,
        /// capture takes the wheel and drag-selection away from the terminal,
        /// which is exactly the scrollback interaction a split footer exists to
        /// preserve. Opt in with
        /// [`with_mouse_capture`](ScreenMode::with_mouse_capture) when the
        /// footer itself needs mouse input.
        mouse_capture: bool,
    },
}

impl ScreenMode {
    /// A footer of `height` rows (clamped to at least 1), without mouse capture.
    pub fn split_footer(height: u16) -> Self {
        Self::SplitFooter {
            height: height.max(1),
            mouse_capture: false,
        }
    }

    /// A footer of [`DEFAULT_FOOTER_HEIGHT`] rows.
    pub fn default_split_footer() -> Self {
        Self::split_footer(DEFAULT_FOOTER_HEIGHT)
    }

    /// Capture the mouse in this mode.
    ///
    /// This is opt-in because terminal mouse reporting takes native OSC 8 link
    /// activation, selection, and scrolling away from the terminal emulator.
    pub fn with_mouse_capture(self) -> Self {
        match self {
            Self::Alternate | Self::AlternateWithMouseCapture => Self::AlternateWithMouseCapture,
            Self::SplitFooter { height, .. } => Self::SplitFooter {
                height,
                mouse_capture: true,
            },
        }
    }

    /// Whether this mode owns the alternate screen.
    pub fn is_alternate(self) -> bool {
        matches!(self, Self::Alternate | Self::AlternateWithMouseCapture)
    }

    /// The reserved footer height, or `None` in [`Alternate`](Self::Alternate).
    pub fn footer_height(self) -> Option<u16> {
        match self {
            Self::Alternate | Self::AlternateWithMouseCapture => None,
            Self::SplitFooter { height, .. } => Some(height),
        }
    }

    /// Whether the host should enable mouse capture for this mode.
    pub fn captures_mouse(self) -> bool {
        match self {
            Self::Alternate => false,
            Self::AlternateWithMouseCapture => true,
            Self::SplitFooter { mouse_capture, .. } => mouse_capture,
        }
    }

    /// The ratatui [`Viewport`] this mode renders into, for a host building its
    /// own [`Terminal`].
    pub fn viewport(self) -> Viewport {
        match self {
            Self::Alternate | Self::AlternateWithMouseCapture => Viewport::Fullscreen,
            Self::SplitFooter { height, .. } => Viewport::Inline(height),
        }
    }
}

/// A queued block: builds the view for a render width known only at flush time.
type Build = Box<dyn FnOnce(u16) -> Element + Send>;

struct Entry {
    /// Rows the caller pinned, or `None` to measure the built view.
    rows: Option<u16>,
    build: Build,
}

/// A handle for publishing content into the terminal scrollback above a split
/// footer.
///
/// Cloneable and `Send + Sync`: background producers keep a clone and publish
/// from their own thread or task. Queued blocks are rendered and committed by
/// the runner on its next loop iteration — at most one
/// [`tick_rate`](crate::RunnerConfig::tick_rate) away — and each block is
/// committed whole, so a block never interleaves with another producer's.
///
/// A block is *not* a frame: it is written into the terminal's scrollback once
/// and never repainted. Views that animate, scroll, or depend on later state
/// belong in the footer.
///
/// In [`ScreenMode::Alternate`] there is no scrollback to write to; the runners
/// discard queued blocks rather than let the queue grow without bound.
#[derive(Clone, Default)]
pub struct Scrollback {
    queue: Arc<Mutex<Vec<Entry>>>,
}

impl Scrollback {
    /// Rows a single block may occupy. A measured or requested height past this
    /// is clamped, bounding the scratch buffer one block can allocate; publish
    /// genuinely long output as several blocks.
    pub const MAX_BLOCK_ROWS: u16 = 4096;

    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a view, built at the render width and sized by its own `measure`.
    pub fn write(&self, build: impl FnOnce(u16) -> Element + Send + 'static) {
        self.push(Entry {
            rows: None,
            build: Box::new(build),
        });
    }

    /// Queue a view that occupies exactly `rows` rows, skipping measurement.
    /// Content past `rows` is clipped, as it would be in any other area.
    pub fn write_rows(&self, rows: u16, build: impl FnOnce(u16) -> Element + Send + 'static) {
        self.push(Entry {
            rows: Some(rows),
            build: Box::new(build),
        });
    }

    /// Queue pre-styled lines — the common case for logs and transcripts.
    pub fn write_lines(&self, lines: Vec<Line<'static>>) {
        self.write(move |_width| element(Text::new(lines)));
    }

    /// Whether anything is waiting to be published.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Drop every queued block without rendering it.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Render and commit every queued block above `terminal`'s inline viewport,
    /// returning whether anything was written.
    ///
    /// Committing scrolls the terminal, and without the `scrolling-regions`
    /// feature it also clears the viewport, so a `true` return means the caller
    /// must repaint the footer before waiting for input again.
    ///
    /// Only an inline viewport has a scrollback to commit into. Against any
    /// other viewport the queue is still drained but the blocks go nowhere,
    /// which is why the runners call [`clear`](Self::clear) instead of this in
    /// [`ScreenMode::Alternate`].
    pub fn flush<B: Backend>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
    ) -> Result<bool, B::Error> {
        let entries = std::mem::take(&mut *self.lock());
        if entries.is_empty() {
            return Ok(false);
        }
        // A screen with no columns has nowhere to paint, and one with no rows
        // is worse than useless: ratatui's portable insert-before makes no
        // forward progress when it cannot draw a single row, so publishing into
        // a zero-row terminal would spin. A terminal that reports 0×N or N×0 is
        // degenerate (a detached or resizing pty), not an error — drop the
        // blocks rather than block the frame loop.
        let width = terminal.get_frame().area().width;
        if width == 0 || terminal.size()?.height == 0 {
            return Ok(false);
        }
        let ctx = RenderCtx::new(theme);
        for entry in entries {
            let view = (entry.build)(width);
            commit(terminal, view.as_ref(), entry.rows, &ctx)?;
        }
        Ok(true)
    }

    fn push(&self, entry: Entry) {
        self.lock().push(entry);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        // A producer that panicked mid-publish leaves the queue usable; the
        // renderer must keep running rather than poison the whole session.
        self.queue.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// Publish one view into the terminal scrollback above the footer, now.
///
/// The synchronous counterpart to [`Scrollback`], for a host already on its own
/// render loop: it renders `view` and commits it immediately, with no queue and
/// no `Send` bound — so a block may borrow or own frame state that could never
/// cross a thread, such as a transcript entry holding a
/// [`MarkdownState`](crate::components::MarkdownState) cache. `ctx` carries the theme and
/// stylesheet, exactly as it would for a normal render, so a host with a custom
/// [`StyleSheet`](crate::StyleSheet) publishes in its own styling.
///
/// The block occupies the rows `view` measures itself as needing (at least one,
/// at most [`Scrollback::MAX_BLOCK_ROWS`]). As with a queued block, committing
/// scrolls the terminal and — without the `scrolling-regions` feature — clears
/// the viewport, so repaint the footer before waiting for input again.
pub fn publish_block<B: Backend>(
    terminal: &mut Terminal<B>,
    view: &dyn View,
    ctx: &RenderCtx,
) -> Result<(), B::Error> {
    commit(terminal, view, None, ctx)
}

/// Render `view` and hand it to the terminal, `rows` tall or self-measured.
fn commit<B: Backend>(
    terminal: &mut Terminal<B>,
    view: &dyn View,
    rows: Option<u16>,
    ctx: &RenderCtx,
) -> Result<(), B::Error> {
    let width = terminal.get_frame().area().width;
    // Same degenerate-screen guard as `Scrollback::flush`: nowhere to paint, and
    // a zero-row screen makes ratatui's portable insert-before spin.
    if width == 0 || terminal.size()?.height == 0 {
        return Ok(());
    }
    let rows = block_rows(rows, view, width, ctx);
    terminal.insert_before(rows, |buffer| paint_block(buffer, view, ctx))
}

/// Rows one block occupies: what the caller pinned, else what the view measures.
///
/// Always at least one row (a zero-row block would be a queue entry that
/// publishes nothing) and never more than [`Scrollback::MAX_BLOCK_ROWS`], which
/// bounds the scratch buffer a single block can allocate — a view is free to
/// measure itself as `u16::MAX` tall.
fn block_rows(requested: Option<u16>, view: &dyn View, width: u16, ctx: &RenderCtx) -> u16 {
    requested
        .unwrap_or_else(|| {
            view.measure(Size::new(width, Scrollback::MAX_BLOCK_ROWS), ctx)
                .height
        })
        .clamp(1, Scrollback::MAX_BLOCK_ROWS)
}

/// Paint one scrollback block into the buffer `insert_before` handed us.
///
/// Deliberately *not* [`paint`](crate::paint): there is no background fill, so
/// cells the view does not touch keep the terminal's own colors and the block
/// reads as part of the surrounding session instead of a pasted panel.
fn paint_block(buffer: &mut Buffer, view: &dyn View, ctx: &RenderCtx) {
    let area = buffer.area;
    let mut surface = Surface::new(buffer, area);
    view.render(area, &mut surface, ctx);
}

/// Push the inline viewport down until it sits on the last rows of the screen.
///
/// ratatui anchors an inline viewport to the cursor row it was created at, which
/// leaves the footer floating mid-screen when the terminal had room below —
/// on a fresh prompt, most of it. Inserting the gap as blank rows scrolls the
/// existing output up and pins the footer to the bottom, which is what makes it
/// a *footer* rather than an inline panel.
///
/// Cheap and idempotent once pinned, so callers run it before every frame: it is
/// also what re-pins the footer after a terminal resize. Call
/// [`Terminal::autoresize`] first so the viewport is recomputed for the new size.
/// A no-op for a non-inline viewport.
pub fn pin_footer<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    let screen = terminal.size()?.height;
    let viewport = terminal.get_frame().area();
    let gap = screen.saturating_sub(viewport.bottom());
    if gap > 0 {
        terminal.insert_before(gap, |_| {})?;
    }
    Ok(())
}

/// Give the footer's rows back to the terminal.
///
/// Clears from the footer's origin down and parks the cursor there, so the shell
/// prompt resumes immediately below the published scrollback instead of after —
/// or on top of — the last frame. Call this before the
/// [`TerminalSession`](crate::TerminalSession) is dropped; restoring raw mode
/// and cursor visibility is the session's job. Against a full-screen viewport
/// the origin is the top-left, so this degrades to a plain
/// [`Terminal::clear`].
pub fn close_footer<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    let area = terminal.get_frame().area();
    // A viewport with no cells reserved nothing, so there is nothing to hand
    // back — and clearing "from its origin" on a degenerate screen would ask
    // the backend to address a cell that does not exist.
    if area.is_empty() {
        return Ok(());
    }
    terminal.clear()?;
    terminal.set_cursor_position(area.as_position())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Position, Rect};
    use crate::style::StyleSheet;
    use crate::style::{Color, Style};
    use crate::term::terminal::TerminalOptions;
    use crate::term::testbackend::TestBackend;
    use crate::tests::support::rainbow_theme;
    use crate::text::Span;
    use crate::view::element;

    /// An inline terminal whose viewport is anchored at `cursor_row`, i.e. what
    /// a host gets when it starts a footer partway down a used screen.
    fn footer_terminal(
        width: u16,
        height: u16,
        footer: u16,
        cursor_row: u16,
    ) -> Terminal<TestBackend> {
        let mut backend = TestBackend::new(width, height);
        backend
            .set_cursor_position(Position::new(0, cursor_row))
            .expect("place cursor");
        Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: ScreenMode::split_footer(footer).viewport(),
            },
        )
        .expect("inline terminal")
    }

    /// A pinned footer terminal — the state every host is in after startup.
    fn pinned(width: u16, height: u16, footer: u16) -> Terminal<TestBackend> {
        let mut terminal = footer_terminal(width, height, footer, 0);
        pin_footer(&mut terminal).expect("pin");
        terminal
    }

    /// Paint `text` on every footer row so the footer's rows are identifiable.
    fn draw_footer(terminal: &mut Terminal<TestBackend>, text: &str) {
        terminal
            .draw(|frame| {
                let area = frame.area();
                let theme = Theme::default();
                let lines = vec![Line::from(text.to_string()); area.height as usize];
                let view = element(Text::new(lines));
                crate::paint(frame.buffer_mut(), area, &theme, view.as_ref(), &[]);
            })
            .expect("draw footer");
    }

    fn screen_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (buffer.area.top()..buffer.area.bottom())
            .map(|y| crate::tests::support::row(buffer, y))
            .collect()
    }

    /// Queue one line of text.
    fn write_text(scrollback: &Scrollback, text: &'static str) {
        scrollback.write(move |_width| element(Text::raw(text)));
    }

    // ---- ScreenMode ------------------------------------------------------

    #[test]
    fn split_footer_clamps_zero_height_and_defaults_off_mouse_capture() {
        let mode = ScreenMode::split_footer(0);
        assert_eq!(mode.footer_height(), Some(1));
        assert!(!mode.captures_mouse());
        assert!(!mode.is_alternate());
    }

    #[test]
    fn mouse_capture_is_opt_in_for_a_footer_and_keeps_its_height() {
        let mode = ScreenMode::split_footer(4).with_mouse_capture();
        assert!(mode.captures_mouse());
        assert_eq!(mode.footer_height(), Some(4));
        // Idempotent, and it does not turn a footer into an alternate screen.
        assert_eq!(mode.with_mouse_capture(), mode);
        assert!(!mode.is_alternate());
    }

    #[test]
    fn alternate_defaults_to_native_terminal_mouse_handling() {
        let mode = ScreenMode::default();
        assert_eq!(mode, ScreenMode::Alternate);
        assert_eq!(mode.viewport(), Viewport::Fullscreen);
        assert_eq!(mode.footer_height(), None);
        assert!(!mode.captures_mouse());
        assert!(mode.is_alternate());

        let captured = mode.with_mouse_capture();
        assert!(captured.captures_mouse());
        assert!(captured.is_alternate());
        assert_eq!(captured.viewport(), Viewport::Fullscreen);
        assert_eq!(captured.with_mouse_capture(), captured);
    }

    #[test]
    fn split_footer_renders_into_an_inline_viewport_of_its_height() {
        assert_eq!(ScreenMode::split_footer(6).viewport(), Viewport::Inline(6));
        assert_eq!(
            ScreenMode::default_split_footer().footer_height(),
            Some(DEFAULT_FOOTER_HEIGHT)
        );
        assert_eq!(
            ScreenMode::default_split_footer().viewport(),
            Viewport::Inline(DEFAULT_FOOTER_HEIGHT)
        );
    }

    // ---- Block sizing ----------------------------------------------------

    #[test]
    fn a_measured_block_takes_the_rows_its_view_asks_for() {
        let view = element(Text::new(vec![Line::from("a"), Line::from("b")]));
        let theme = Theme::default();
        assert_eq!(
            block_rows(None, view.as_ref(), 10, &RenderCtx::new(&theme)),
            2
        );
    }

    #[test]
    fn a_zero_row_block_is_clamped_to_one_row() {
        // A block that publishes nothing is a queue entry that did nothing;
        // clamping keeps `write_rows(0)` meaning "one row", not "silently drop".
        let view = element(Text::raw("x"));
        let theme = Theme::default();
        assert_eq!(
            block_rows(Some(0), view.as_ref(), 10, &RenderCtx::new(&theme)),
            1
        );
    }

    #[test]
    fn an_oversized_block_is_clamped_to_the_row_ceiling() {
        struct Tall;
        impl View for Tall {
            fn measure(&self, _available: Size, _ctx: &RenderCtx) -> Size {
                Size::new(1, u16::MAX)
            }
            fn render(&self, _area: Rect, _surface: &mut Surface, _ctx: &RenderCtx) {}
        }
        // Both a view that measures itself absurdly tall and a caller that asks
        // for it are bounded, so one block cannot allocate an unbounded buffer.
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        assert_eq!(
            block_rows(None, &Tall, 10, &ctx),
            Scrollback::MAX_BLOCK_ROWS,
            "a measured height is clamped"
        );
        assert_eq!(
            block_rows(Some(u16::MAX), &Tall, 10, &ctx),
            Scrollback::MAX_BLOCK_ROWS,
            "a requested height is clamped"
        );
    }

    // ---- Pinning ---------------------------------------------------------

    #[test]
    fn pin_footer_moves_the_viewport_to_the_bottom_and_stays_there() {
        let mut terminal = footer_terminal(12, 10, 3, 0);
        assert_eq!(terminal.get_frame().area(), Rect::new(0, 0, 12, 3));

        pin_footer(&mut terminal).expect("pin");

        assert_eq!(
            terminal.get_frame().area(),
            Rect::new(0, 7, 12, 3),
            "the footer occupies the last three rows"
        );
        // Idempotent: a second pass has no gap left to close.
        pin_footer(&mut terminal).expect("re-pin");
        assert_eq!(terminal.get_frame().area(), Rect::new(0, 7, 12, 3));
    }

    #[test]
    fn pin_footer_repins_to_the_new_bottom_after_a_resize() {
        let mut terminal = pinned(12, 10, 3);
        assert_eq!(terminal.get_frame().area(), Rect::new(0, 7, 12, 3));

        // A taller terminal: ratatui re-anchors the viewport to the cursor row,
        // which leaves a gap below it that the next pin closes.
        terminal.backend_mut().resize(12, 16);
        terminal.autoresize().expect("autoresize");
        pin_footer(&mut terminal).expect("re-pin");
        assert_eq!(terminal.get_frame().area().bottom(), 16);
        assert_eq!(terminal.get_frame().area().height, 3);

        // And a shorter one.
        terminal.backend_mut().resize(12, 8);
        terminal.autoresize().expect("autoresize");
        pin_footer(&mut terminal).expect("re-pin");
        assert_eq!(terminal.get_frame().area().bottom(), 8);
        assert_eq!(terminal.get_frame().area().height, 3);
    }

    #[test]
    fn a_footer_taller_than_the_screen_is_clamped_to_it() {
        let theme = Theme::default();
        let mut terminal = footer_terminal(8, 3, 40, 0);
        assert_eq!(terminal.get_frame().area(), Rect::new(0, 0, 8, 3));
        pin_footer(&mut terminal).expect("pin");
        assert_eq!(terminal.get_frame().area(), Rect::new(0, 0, 8, 3));

        // With no room above, a published block still reaches the scrollback
        // rather than panicking or clipping the footer.
        let scrollback = Scrollback::new();
        write_text(&scrollback, "note");
        assert!(scrollback.flush(&mut terminal, &theme).expect("flush"));
    }

    // ---- Publishing ------------------------------------------------------

    #[test]
    fn flushed_blocks_land_above_the_footer_in_order() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 8, 2);
        draw_footer(&mut terminal, "FOOTER");

        let scrollback = Scrollback::new();
        write_text(&scrollback, "first");
        scrollback.write_lines(vec![Line::from("second"), Line::from("third")]);

        assert!(
            scrollback
                .flush(&mut terminal, &theme)
                .expect("flush blocks"),
            "flushing published blocks reports that the footer needs a repaint"
        );
        assert!(scrollback.is_empty(), "the queue is drained by a flush");
        // Committing scrolls the viewport, so repaint as a real host would.
        draw_footer(&mut terminal, "FOOTER");

        let lines = screen_lines(&terminal);
        assert_eq!(
            &lines[3..],
            &["first", "second", "third", "FOOTER", "FOOTER"],
            "blocks stack above the footer in publication order: {lines:?}"
        );
    }

    #[test]
    fn a_pinned_row_count_wins_over_the_measurement() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 8, 2);

        fn lines(texts: [&str; 3]) -> Vec<Line<'static>> {
            texts.map(|text| Line::from(text.to_string())).to_vec()
        }
        let scrollback = Scrollback::new();
        // Three lines measure to three rows; the pinned block takes exactly one
        // and clips the rest.
        scrollback.write(|_width| element(Text::new(lines(["a", "b", "c"]))));
        scrollback.write_rows(1, |_width| element(Text::new(lines(["x", "y", "z"]))));
        scrollback.flush(&mut terminal, &theme).expect("flush");
        draw_footer(&mut terminal, "F");

        let lines = screen_lines(&terminal);
        assert_eq!(&lines[2..6], &["a", "b", "c", "x"], "{lines:?}");
    }

    #[test]
    fn a_block_is_built_at_the_terminal_width_and_clipped_to_it() {
        let theme = Theme::default();
        let mut terminal = pinned(10, 6, 2);

        let scrollback = Scrollback::new();
        // The builder sees the width it will be painted at...
        scrollback.write(|width| element(Text::raw(format!("w={width}"))));
        // ...and content past that width is clipped, not wrapped onto a row the
        // block never reserved.
        scrollback.write(|_width| element(Text::raw("0123456789ABCDEF")));
        scrollback.flush(&mut terminal, &theme).expect("flush");

        let lines = screen_lines(&terminal);
        assert_eq!(lines[2], "w=10", "{lines:?}");
        assert_eq!(lines[3], "0123456789", "{lines:?}");
    }

    #[test]
    fn a_block_taller_than_the_screen_still_publishes_its_tail() {
        let theme = Theme::default();
        let mut terminal = pinned(8, 6, 2);

        let scrollback = Scrollback::new();
        scrollback.write(|_width| {
            element(Text::new(
                (0..10)
                    .map(|i| Line::from(format!("r{i}")))
                    .collect::<Vec<_>>(),
            ))
        });
        assert!(scrollback.flush(&mut terminal, &theme).expect("flush"));
        draw_footer(&mut terminal, "F");

        // The screen holds the last rows of the block; the rest scrolled into
        // the terminal's own scrollback buffer.
        let lines = screen_lines(&terminal);
        assert_eq!(&lines[..4], &["r6", "r7", "r8", "r9"], "{lines:?}");
        assert_eq!(&lines[4..], &["F", "F"], "{lines:?}");
    }

    #[test]
    fn published_style_survives_into_the_scrollback() {
        let theme = rainbow_theme();
        let mut terminal = pinned(12, 6, 2);

        let scrollback = Scrollback::new();
        let accent = theme.accent;
        scrollback.write(move |_width| {
            element(Text::new(vec![Line::from(Span::styled(
                "hi",
                Style::default().fg(accent),
            ))]))
        });
        scrollback.flush(&mut terminal, &theme).expect("flush");

        let buffer = terminal.backend().buffer();
        let row = buffer.area.bottom() - 3;
        assert_eq!(
            buffer[(0, row)].fg,
            accent,
            "styled spans reach the terminal"
        );
    }

    #[test]
    fn blocks_keep_the_terminal_background_outside_painted_cells() {
        let theme = rainbow_theme();
        let mut terminal = pinned(12, 6, 2);

        let scrollback = Scrollback::new();
        write_text(&scrollback, "hi");
        scrollback.flush(&mut terminal, &theme).expect("flush");

        let buffer = terminal.backend().buffer();
        let row = buffer.area.bottom() - 3;
        assert_eq!(
            buffer[(11, row)].bg,
            Color::Reset,
            "unpainted cells in a block keep the terminal's own background"
        );
        assert_ne!(
            theme.background,
            Color::Reset,
            "the theme would have painted a background if the block used one"
        );
    }

    #[test]
    fn flushing_an_empty_queue_writes_nothing() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 6, 2);
        draw_footer(&mut terminal, "FOOTER");
        let before = screen_lines(&terminal);

        let scrollback = Scrollback::new();
        assert!(scrollback.is_empty());
        assert!(!scrollback.flush(&mut terminal, &theme).expect("flush"));
        assert_eq!(screen_lines(&terminal), before);
    }

    // A zero-column screen: the guard must hold *before* anything reaches the
    // backend. (`TestBackend` cannot model one — its buffer indexing panics on
    // a zero-width area — so this drives `flush` on an unpinned terminal, which
    // is exactly the path the guard protects.)
    #[test]
    fn a_zero_width_terminal_publishes_nothing() {
        let theme = Theme::default();
        let mut terminal = footer_terminal(0, 4, 2, 0);

        let scrollback = Scrollback::new();
        write_text(&scrollback, "nowhere");
        assert!(
            !scrollback.flush(&mut terminal, &theme).expect("flush"),
            "there is no column to publish into"
        );
    }

    #[test]
    fn cleared_blocks_are_never_published() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 6, 2);

        let scrollback = Scrollback::new();
        write_text(&scrollback, "dropped");
        assert!(!scrollback.is_empty());
        scrollback.clear();
        assert!(scrollback.is_empty());

        assert!(!scrollback.flush(&mut terminal, &theme).expect("flush"));
        assert!(
            screen_lines(&terminal).iter().all(|line| line.is_empty()),
            "a discarded block never reaches the terminal"
        );
    }

    // ---- Publishing without the queue ------------------------------------

    #[test]
    fn publish_block_commits_immediately_and_in_order() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 8, 2);
        draw_footer(&mut terminal, "FOOTER");
        let ctx = RenderCtx::new(&theme);

        // No queue, no flush: each call is on screen when it returns.
        publish_block(&mut terminal, &Text::raw("one"), &ctx).expect("publish");
        publish_block(&mut terminal, &Text::raw("two"), &ctx).expect("publish");
        draw_footer(&mut terminal, "FOOTER");

        let lines = screen_lines(&terminal);
        assert_eq!(
            &lines[4..],
            &["one", "two", "FOOTER", "FOOTER"],
            "{lines:?}"
        );
    }

    #[test]
    fn publish_block_uses_the_hosts_own_stylesheet() {
        // A host with a custom `StyleSheet` publishes in its own styling, the
        // same as it renders: `publish_block` takes the render context, not just
        // a theme.
        let theme = rainbow_theme();
        let mut sheet = StyleSheet::from_theme(&theme);
        sheet.panel = sheet.panel.fg(Color::Indexed(200));
        let ctx = RenderCtx::new(&theme).with_sheet(sheet);
        let mut terminal = pinned(12, 6, 2);

        publish_block(
            &mut terminal,
            &crate::components::Boxed::new(element(Text::raw("x"))),
            &ctx,
        )
        .expect("publish");

        let buffer = terminal.backend().buffer();
        // The box border resolves through the sheet the host installed, not the
        // theme's default one.
        let row = buffer.area.bottom() - 5;
        assert_eq!(buffer[(0, row)].symbol(), "╭");
        assert_eq!(
            buffer[(0, row)].fg,
            Color::Indexed(200),
            "the host's panel style, not the theme's border color"
        );
        assert_ne!(Color::Indexed(200), theme.border);
    }

    #[test]
    fn publish_block_may_own_state_that_cannot_cross_a_thread() {
        // The reason this exists next to `Scrollback`: a transcript entry holds
        // a `MarkdownState` cache, which is not `Send`, so it could never be
        // queued — but it can be published from the loop that owns it.
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let mut terminal = pinned(20, 8, 2);

        let mut state = crate::components::MarkdownState::new();
        state.set("**done** in 12ms");
        let lines = state
            .lines(
                20,
                &theme,
                &ctx.sheet,
                crate::highlight::CodeHighlighter::Plain,
            )
            .to_vec();
        publish_block(&mut terminal, &Text::new(lines), &ctx).expect("publish");

        let lines = screen_lines(&terminal);
        assert!(
            lines.iter().any(|line| line.contains("done in 12ms")),
            "rendered markdown reached the scrollback: {lines:?}"
        );
    }

    #[test]
    fn publish_block_is_a_no_op_on_a_degenerate_screen() {
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        // No columns to paint into, and (separately) no rows: neither may reach
        // the backend, and neither may spin.
        let mut narrow = footer_terminal(0, 4, 2, 0);
        publish_block(&mut narrow, &Text::raw("nowhere"), &ctx).expect("publish");
        let mut flat = footer_terminal(10, 0, 2, 0);
        publish_block(&mut flat, &Text::raw("nowhere"), &ctx).expect("publish");
    }

    // ---- Concurrency -----------------------------------------------------

    #[test]
    fn a_scrollback_handle_is_shared_and_thread_safe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Scrollback>();

        let theme = Theme::default();
        let mut terminal = pinned(12, 8, 2);

        let scrollback = Scrollback::new();
        let handles: Vec<_> = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|name| {
                let producer = scrollback.clone();
                std::thread::spawn(move || {
                    // Two rows per producer: the block must arrive whole, never
                    // split by another producer's rows.
                    producer.write_lines(vec![
                        Line::from(format!("{name}-1")),
                        Line::from(format!("{name}-2")),
                    ]);
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("producer thread");
        }

        assert!(scrollback.flush(&mut terminal, &theme).expect("flush"));
        let lines = screen_lines(&terminal);
        for name in ["alpha", "beta", "gamma"] {
            let first = lines
                .iter()
                .position(|line| line == &format!("{name}-1"))
                .unwrap_or_else(|| panic!("{name} published: {lines:?}"));
            assert_eq!(
                lines[first + 1],
                format!("{name}-2"),
                "a block is committed whole: {lines:?}"
            );
        }
    }

    #[test]
    fn a_producer_panicking_mid_publish_leaves_the_queue_usable() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 6, 2);

        let scrollback = Scrollback::new();
        let poisoner = scrollback.clone();
        let panicked = std::thread::spawn(move || {
            let _guard = poisoner.queue.lock().expect("lock");
            panic!("producer died holding the queue");
        })
        .join();
        assert!(panicked.is_err(), "the producer really did panic");

        // The renderer must keep running: a poisoned queue is recovered rather
        // than propagated into the frame loop.
        write_text(&scrollback, "after");
        assert!(scrollback.flush(&mut terminal, &theme).expect("flush"));
        assert!(screen_lines(&terminal).contains(&"after".to_string()));
    }

    // ---- Teardown --------------------------------------------------------

    #[test]
    fn close_footer_clears_the_region_and_parks_the_cursor() {
        let mut terminal = pinned(12, 6, 2);
        draw_footer(&mut terminal, "FOOTER");

        close_footer(&mut terminal).expect("close");

        let lines = screen_lines(&terminal);
        assert!(
            lines[4..].iter().all(|line| line.is_empty()),
            "the footer rows are given back blank: {lines:?}"
        );
        assert_eq!(
            terminal.backend().cursor_position(),
            Position::new(0, 4),
            "the prompt resumes where the footer started"
        );
    }

    #[test]
    fn close_footer_leaves_the_published_scrollback_alone() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 6, 2);
        let scrollback = Scrollback::new();
        write_text(&scrollback, "kept");
        scrollback.flush(&mut terminal, &theme).expect("flush");
        draw_footer(&mut terminal, "FOOTER");

        close_footer(&mut terminal).expect("close");

        assert!(
            screen_lines(&terminal).contains(&"kept".to_string()),
            "published output is the user's, not ours to clear"
        );
    }

    #[test]
    fn close_footer_on_a_full_screen_viewport_clears_the_screen() {
        let mut terminal = Terminal::new(TestBackend::new(6, 3)).expect("terminal");
        terminal
            .draw(|frame| {
                frame
                    .buffer_mut()
                    .set_string(0, 0, "hello", Style::default());
            })
            .expect("draw");

        close_footer(&mut terminal).expect("close");

        assert!(
            screen_lines(&terminal).iter().all(|line| line.is_empty()),
            "with the viewport at the origin this is a plain clear"
        );
        assert_eq!(terminal.backend().cursor_position(), Position::new(0, 0));
    }

    // ---- Degenerate geometry ---------------------------------------------

    // The whole lifecycle — pin, publish, paint, close — across sizes down to
    // one cell, including a screen with no rows at all: not passing (no panic,
    // no hang, no out-of-bounds write) is the assertion. Width starts at 1
    // because `TestBackend` panics indexing a zero-width buffer; the
    // zero-column guard is asserted above instead.
    #[test]
    fn the_footer_lifecycle_survives_degenerate_screens() {
        let theme = Theme::default();
        for &width in &[1u16, 2, 5, 40] {
            for &height in &[0u16, 1, 2, 3, 9] {
                for &footer in &[1u16, 2, 12] {
                    let mut terminal = footer_terminal(width, height, footer, 0);
                    pin_footer(&mut terminal).expect("pin");
                    let scrollback = Scrollback::new();
                    write_text(&scrollback, "block");
                    scrollback.flush(&mut terminal, &theme).expect("flush");
                    draw_footer(&mut terminal, "F");
                    close_footer(&mut terminal).expect("close");
                    let area = terminal.get_frame().area();
                    assert!(
                        area.bottom() <= height && area.right() <= width,
                        "viewport {area:?} escaped a {width}x{height} screen"
                    );
                }
            }
        }
    }

    // ---- The `scrolling-regions` trade-off -------------------------------

    // The two publish paths differ in exactly one visible way, and it is the
    // reason the feature exists: whether the footer survives a commit or has to
    // be repainted. Pin both so the feature cannot change behavior unnoticed.
    #[test]
    #[cfg(feature = "scrolling-regions")]
    fn publishing_leaves_the_painted_footer_on_screen() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 6, 2);
        draw_footer(&mut terminal, "FOOTER");

        let scrollback = Scrollback::new();
        write_text(&scrollback, "block");
        scrollback.flush(&mut terminal, &theme).expect("flush");

        let lines = screen_lines(&terminal);
        assert_eq!(
            &lines[4..],
            &["FOOTER", "FOOTER"],
            "scrolling regions move only the rows above the footer: {lines:?}"
        );
    }

    #[test]
    #[cfg(not(feature = "scrolling-regions"))]
    fn publishing_clears_the_footer_for_the_next_repaint() {
        let theme = Theme::default();
        let mut terminal = pinned(12, 6, 2);
        draw_footer(&mut terminal, "FOOTER");

        let scrollback = Scrollback::new();
        write_text(&scrollback, "block");
        scrollback.flush(&mut terminal, &theme).expect("flush");

        let lines = screen_lines(&terminal);
        assert!(
            lines[4..].iter().all(|line| line.is_empty()),
            "the portable path clears the viewport, which is why `flush` \
             reporting `true` obliges the caller to repaint: {lines:?}"
        );
        draw_footer(&mut terminal, "FOOTER");
        assert_eq!(&screen_lines(&terminal)[4..], &["FOOTER", "FOOTER"]);
    }
}
