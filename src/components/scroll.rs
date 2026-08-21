//! Scroll viewport with a scrollbar.
//!
//! This is the primitive that replaces native terminal scrollback in the
//! full-screen renderer: content taller than the viewport is windowed by a
//! persisted [`ScrollState`], and the state handles wheel/paging events. The
//! offset is measured in content rows from the top; a "stick to bottom" flag
//! keeps a live transcript pinned to the newest line until the user scrolls up.
//!
//! Beyond the built-in wheel/paging [`handle`](ScrollState::handle), the offset
//! is **host-drivable**: an app that owns its scroll position in its own model
//! mirrors it into the view with [`set_offset`](ScrollState::set_offset), the
//! vertical peer of [`SelectState::select`](crate::components::SelectState::select).
//!
//! Content wider than the pane — logs, unified diffs, wide tables, deep paths —
//! **pans horizontally**: [`set_x_offset`](ScrollState::set_x_offset) shifts the
//! view left by a number of display columns (bind it to `h`/`l` or `←`/`→`), and
//! [`clamp_x`](ScrollState::clamp_x) bounds it to the widest line. The pan is
//! width-aware — it skips whole grapheme clusters, so wide/CJK glyphs never
//! split. [`max_offset`](ScrollState::max_offset) and
//! [`max_x_offset`](ScrollState::max_x_offset) expose the in-range bounds for a
//! host that drives the offsets itself.
//!
//! [`Scroll::wrap`] reflows owned styled lines at the assigned width before
//! windowing, for prose that must both wrap and scroll.

use crate::geometry::Rect;
use crate::text::Line;

use crate::event::{Event, InputOutcome, KeyCode, MouseKind};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

use super::text::wrap_lines;
use super::{Scrollbar, VirtualWindow};

/// Persisted scroll position for one scroll region.
///
/// Dimensions are `usize` on purpose: a transcript can wrap to far more than
/// `u16::MAX` rows in a long session. Measuring the offset and content height in
/// `u16` let a 65,536-row transcript wrap to ~0, which collapsed
/// stick-to-bottom's `max_offset` to 0 and silently snapped the view to the top.
#[derive(Clone, Copy, Debug)]
pub struct ScrollState {
    /// Top visible content row.
    offset: usize,
    /// Leftmost visible display column (horizontal pan). Zero unless the host
    /// pans; used by wide viewers — logs, diffs, wide tables, deep paths.
    x_offset: usize,
    /// When true, `clamp` snaps to the bottom on content growth so new
    /// transcript output stays visible.
    stick_to_bottom: bool,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    /// A fresh state at the top with bottom-stick armed.
    pub fn new() -> Self {
        Self {
            offset: 0,
            x_offset: 0,
            stick_to_bottom: true,
        }
    }

    /// Top visible content row (0-based).
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Set the top visible content row explicitly, detaching bottom-stick.
    ///
    /// The vertical counterpart to [`SelectState::select`](crate::components::SelectState::select):
    /// an event-loop app that already owns a scroll position in its own model
    /// mirrors it into the view each frame with this, instead of only nudging
    /// via [`handle`](Self::handle). A following [`clamp`](Self::clamp) still
    /// bounds it to the content, so an out-of-range value snaps into range
    /// rather than scrolling past the end.
    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
        self.stick_to_bottom = false;
    }

    /// Leftmost visible display column (0-based). Zero unless the view is panned.
    pub fn x_offset(&self) -> usize {
        self.x_offset
    }

    /// Set the leftmost visible display column (horizontal pan) — the twin of
    /// [`set_offset`](Self::set_offset). Panning is measured in display columns,
    /// not bytes or `char`s, so wide/CJK glyphs stay whole. Bind it to `h`/`l`
    /// or `←`/`→`, or mirror an app-owned column. Follow with
    /// [`clamp_x`](Self::clamp_x) to keep it within the widest line.
    pub fn set_x_offset(&mut self, cols: usize) {
        self.x_offset = cols;
    }

    /// Whether the view is pinned to the newest content.
    pub fn is_stuck_to_bottom(&self) -> bool {
        self.stick_to_bottom
    }

    /// The largest in-range vertical offset for the given content and viewport
    /// heights (`content_h - viewport_h`, floored at 0).
    ///
    /// Exposed so a host driving the offset from its own model (see
    /// [`set_offset`](Self::set_offset)) can bound its own key handling with the
    /// same arithmetic the view uses, rather than reimplementing it.
    pub fn max_offset(content_h: usize, viewport_h: usize) -> usize {
        VirtualWindow::max_start_for(content_h, viewport_h)
    }

    /// The largest in-range horizontal pan for the given content and viewport
    /// widths (`content_w - viewport_w`, floored at 0), where `content_w` is the
    /// widest line's display width. The horizontal peer of
    /// [`max_offset`](Self::max_offset).
    pub fn max_x_offset(content_w: usize, viewport_w: usize) -> usize {
        VirtualWindow::max_start_for(content_w, viewport_w)
    }

    /// Reconcile the offset with current content/viewport dimensions, honoring
    /// the stick-to-bottom flag. Call once per frame before rendering.
    pub fn clamp(&mut self, content_h: usize, viewport_h: usize) {
        let max = Self::max_offset(content_h, viewport_h);
        if self.stick_to_bottom {
            self.offset = max;
        } else {
            self.offset = self.offset.min(max);
        }
    }

    /// Bound the horizontal pan to the widest line: `x_offset` is clamped to
    /// `content_w - viewport_w`, so panning can't scroll past the end of the
    /// longest line. `content_w` is that line's display width; `viewport_w` is
    /// the visible column count. The horizontal peer of [`clamp`](Self::clamp).
    pub fn clamp_x(&mut self, content_w: usize, viewport_w: usize) {
        self.x_offset = self.x_offset.min(Self::max_x_offset(content_w, viewport_w));
    }

    fn scroll_up(&mut self, lines: usize) -> InputOutcome {
        let changed = self.offset != 0 || self.stick_to_bottom;
        self.offset = self.offset.saturating_sub(lines);
        self.stick_to_bottom = false;
        if changed {
            InputOutcome::Changed
        } else {
            InputOutcome::Consumed
        }
    }

    fn scroll_down(&mut self, lines: usize, content_h: usize, viewport_h: usize) -> InputOutcome {
        let max = Self::max_offset(content_h, viewport_h);
        let changed = self.offset != max || !self.stick_to_bottom;
        self.offset = self.offset.saturating_add(lines).min(max);
        // Re-arm bottom-stick once the user scrolls back to the end.
        self.stick_to_bottom = self.offset >= max;
        if changed {
            InputOutcome::Changed
        } else {
            InputOutcome::Consumed
        }
    }

    /// Jump to the newest content and re-enable bottom-stick.
    pub fn jump_to_bottom(&mut self, content_h: usize, viewport_h: usize) {
        self.offset = Self::max_offset(content_h, viewport_h);
        self.stick_to_bottom = true;
    }

    /// Jump to the top.
    pub fn jump_to_top(&mut self) {
        self.offset = 0;
        self.stick_to_bottom = false;
    }

    /// Handle a scroll/paging event against the given dimensions.
    pub fn handle(&mut self, event: &Event, content_h: usize, viewport_h: usize) -> InputOutcome {
        let page = viewport_h.saturating_sub(1).max(1);
        match event {
            Event::Mouse(m) => match m.kind {
                MouseKind::ScrollUp => self.scroll_up(3),
                MouseKind::ScrollDown => self.scroll_down(3, content_h, viewport_h),
                _ => InputOutcome::Ignored,
            },
            Event::Key(k) if k.plain() => match k.code {
                KeyCode::PageUp => self.scroll_up(page),
                KeyCode::PageDown => self.scroll_down(page, content_h, viewport_h),
                KeyCode::Home => {
                    let changed = self.offset != 0 || self.stick_to_bottom;
                    self.jump_to_top();
                    if changed {
                        InputOutcome::Changed
                    } else {
                        InputOutcome::Consumed
                    }
                }
                KeyCode::End => {
                    let max = Self::max_offset(content_h, viewport_h);
                    let changed = self.offset != max || !self.stick_to_bottom;
                    self.jump_to_bottom(content_h, viewport_h);
                    if changed {
                        InputOutcome::Changed
                    } else {
                        InputOutcome::Consumed
                    }
                }
                _ => InputOutcome::Ignored,
            },
            _ => InputOutcome::Ignored,
        }
    }
}

/// A windowed view of content, showing the slice at `offset` and a scrollbar
/// when content overflows.
///
/// Two constructors trade off who holds the rows. [`new`](Scroll::new) owns the
/// *whole* content and paints the visible slice out of it — simplest, and right
/// for short lists. [`windowed`](Scroll::windowed) is handed *only* the visible
/// slice plus the true content height; for very long content that turns a frame
/// from O(content) (clone every row in, drop it out) into O(viewport). Both draw
/// identically; only the ownership differs.
///
/// ![scroll demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/scroll.gif)
pub struct Scroll {
    /// The rows this view holds: the whole content in [`new`](Scroll::new), or
    /// just `content[window_start..]` in [`windowed`](Scroll::windowed).
    lines: Vec<Line<'static>>,
    /// Absolute content-row index of `lines[0]`. Zero for `new`; `offset` for
    /// `windowed`, so `render` maps a content row to a `lines` index the same
    /// way in both modes.
    window_start: usize,
    /// Total content height in rows, even when `lines` holds only a window.
    content_height: usize,
    /// Top visible content row.
    offset: usize,
    /// Leftmost visible display column; each line is drawn skipping this many
    /// columns from its left. Zero (the default) is the flush-left fast path.
    x_offset: usize,
    stick_to_bottom: bool,
    scrollbar: bool,
    wrap: bool,
    windowed: bool,
}

impl Scroll {
    /// Build a viewport over the whole `lines`, painting the slice at `state`'s
    /// offset. The view owns every row; for content far taller than the viewport
    /// prefer [`windowed`](Scroll::windowed).
    pub fn new(lines: Vec<Line<'static>>, state: &ScrollState) -> Self {
        Self {
            content_height: lines.len(),
            lines,
            window_start: 0,
            offset: state.offset(),
            x_offset: state.x_offset(),
            stick_to_bottom: state.is_stuck_to_bottom(),
            scrollbar: true,
            wrap: false,
            windowed: false,
        }
    }

    /// Build a viewport that already holds only the visible window —
    /// `content[offset .. offset + viewport_height]`, where `offset` is
    /// `state`'s offset — instead of the whole content. `content_height` is the
    /// full row count, so the scrollbar and [`measure`](View::measure) still
    /// reflect the entire content.
    ///
    /// This is the O(viewport) path for very long content: the caller slices its
    /// own cache once per frame rather than handing over — and dropping — every
    /// row. The window may be shorter than the viewport near the bottom;
    /// `render` simply stops at its end.
    pub fn windowed(
        window: Vec<Line<'static>>,
        content_height: usize,
        state: &ScrollState,
    ) -> Self {
        let offset = state.offset();
        Self {
            lines: window,
            window_start: offset,
            content_height,
            offset,
            x_offset: state.x_offset(),
            stick_to_bottom: state.is_stuck_to_bottom(),
            scrollbar: true,
            wrap: false,
            windowed: true,
        }
    }

    /// Toggle the scrollbar (shown by default when content overflows).
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    /// Word-wrap owned lines to the assigned width before scrolling.
    ///
    /// Wrapping and overflow are resolved at render time, when width is known.
    /// Horizontal panning is disabled while wrapping. A [`windowed`](Self::windowed)
    /// view already represents host-prepared rows, so its rows must be wrapped
    /// by the host and this setting has no effect there.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Total content height in rows (one per line), even in windowed mode.
    pub fn content_height(&self) -> usize {
        self.content_height
    }
}

impl View for Scroll {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        // `Size` is a terminal-cell extent (`u16`); a transcript can be taller
        // than that. Saturate — the intrinsic hint only matters when the scroll
        // is not a flex `grow` child, and a viewport is never `u16::MAX` tall.
        let content_height = if self.wrap && !self.windowed {
            wrap_lines(&self.lines, available.width).len()
        } else {
            self.content_height()
        };
        let intrinsic_h = content_height.min(u16::MAX as usize) as u16;
        Size::new(available.width, intrinsic_h)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let should_wrap = self.wrap && !self.windowed;
        let mut wrapped = if should_wrap {
            wrap_lines(&self.lines, area.width)
        } else {
            Vec::new()
        };
        let mut content_h = if should_wrap {
            wrapped.len()
        } else {
            self.content_height()
        };
        let mut overflow = content_h > area.height as usize;
        let text_width = if overflow && self.scrollbar {
            area.width.saturating_sub(1)
        } else {
            area.width
        };
        if should_wrap && text_width < area.width {
            wrapped = wrap_lines(&self.lines, text_width);
            content_h = wrapped.len();
            overflow = content_h > area.height as usize;
        }
        let lines = if should_wrap { &wrapped } else { &self.lines };
        let window_start = if should_wrap { 0 } else { self.window_start };
        let max_offset = ScrollState::max_offset(content_h, area.height as usize);
        let offset = if should_wrap && self.stick_to_bottom {
            max_offset
        } else if should_wrap {
            self.offset.min(max_offset)
        } else {
            self.offset
        };

        for row in 0..area.height {
            // Map the content row (offset + row) to an index into `lines`, which
            // begins at `window_start` (0 in full mode, `offset` when windowed).
            let Some(idx) = (offset + row as usize).checked_sub(window_start) else {
                break;
            };
            let Some(line) = lines.get(idx) else {
                break;
            };
            let y = area.y + row;
            let mut clip = surface.child(Rect::new(area.x, y, text_width, 1));
            // Skip `x_offset` display columns from the left of each line (the
            // horizontal pan), carried across the line's spans. A zero offset is
            // exactly the flush-left `set_string` path.
            let mut skip = if should_wrap {
                0
            } else {
                self.x_offset.min(u16::MAX as usize) as u16
            };
            let mut x = area.x;
            for span in &line.spans {
                if x >= area.x + text_width {
                    break;
                }
                x = clip.set_string_skip(
                    x,
                    y,
                    span.content.as_ref(),
                    line.style.patch(span.style),
                    &mut skip,
                );
            }
        }

        if overflow && self.scrollbar && text_width < area.width {
            Scrollbar::vertical(VirtualWindow::new(
                content_h,
                usize::from(area.height),
                offset,
            ))
            .render(
                Rect::new(area.right() - 1, area.y, 1, area.height),
                surface,
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Surface;
    use crate::event::{Event, InputOutcome, Key, KeyCode, Mouse, MouseKind};
    use crate::style::Color;
    use crate::style::Style;
    use crate::style::Theme;
    use crate::tests::support::{buffer, rainbow_theme, row};
    use crate::text::{Line, Span};
    use crate::view::{RenderCtx, View};

    #[test]
    fn scroll_sticks_to_bottom_until_scrolled_up() {
        let mut s = ScrollState::new();
        // content 100 rows, viewport 10 => bottom offset 90.
        s.clamp(100, 10);
        assert_eq!(s.offset(), 90);
        assert!(s.is_stuck_to_bottom());

        // Wheel up unsticks and moves up by 3.
        let up = Event::Mouse(Mouse::at(MouseKind::ScrollUp, 0, 0));
        assert_eq!(s.handle(&up, 100, 10), InputOutcome::Changed);
        assert!(!s.is_stuck_to_bottom());
        assert_eq!(s.offset(), 87);

        // Growing content no longer drags the view down while unstuck.
        s.clamp(200, 10);
        assert_eq!(s.offset(), 87);
    }

    #[test]
    fn scroll_offset_survives_content_taller_than_u16() {
        // Regression: a transcript taller than u16::MAX must not wrap the
        // content height back near 0 and collapse stick-to-bottom to the top.
        let content_h = u16::MAX as usize + 5_000; // 70,535 rows
        let viewport_h = 40;
        let mut s = ScrollState::new();
        s.clamp(content_h, viewport_h);
        assert_eq!(s.offset(), content_h - viewport_h);
        assert!(s.is_stuck_to_bottom());

        // Paging up from the bottom detaches and moves by a page, still far from
        // the top rather than snapping there.
        let up = Event::Key(Key::new(KeyCode::PageUp));
        assert_eq!(s.handle(&up, content_h, viewport_h), InputOutcome::Changed);
        assert!(!s.is_stuck_to_bottom());
        assert_eq!(s.offset(), content_h - viewport_h - (viewport_h - 1));
    }

    #[test]
    fn set_offset_positions_view_and_detaches_stick() {
        let mut s = ScrollState::new();
        s.clamp(100, 10);
        assert!(s.is_stuck_to_bottom(), "starts stuck to bottom");
        // Host mirrors its own scroll row in; stick detaches and clamp honors it.
        s.set_offset(40);
        assert!(!s.is_stuck_to_bottom());
        s.clamp(100, 10);
        assert_eq!(s.offset(), 40);
        // An out-of-range value snaps into range on the next clamp rather than
        // scrolling past the end.
        s.set_offset(500);
        s.clamp(100, 10);
        assert_eq!(s.offset(), 90, "clamped to content height - viewport");
    }

    #[test]
    fn scroll_wraps_at_render_width_before_windowing() {
        let mut state = ScrollState::new();
        state.jump_to_top();
        let scroll = Scroll::new(
            vec![Line::styled(
                "the quick brown fox",
                Style::default().fg(Color::Blue),
            )],
            &state,
        )
        .wrap(true)
        .scrollbar(false);
        let buf = crate::testing::render(&scroll, 9, 3, &Theme::default());
        assert_eq!(
            crate::testing::grid(&buf),
            "the quick\nbrown fox\n         "
        );
        assert_eq!(buf[(0, 1)].fg, Color::Blue);
    }

    #[test]
    fn horizontal_pan_offset_clamp_and_max() {
        let mut s = ScrollState::new();
        assert_eq!(s.x_offset(), 0, "no pan by default");
        s.set_x_offset(34);
        assert_eq!(s.x_offset(), 34);
        // clamp_x bounds the pan to the widest line minus the viewport (30-10).
        s.clamp_x(30, 10);
        assert_eq!(s.x_offset(), 20);
        // A pan already within bounds is untouched.
        s.clamp_x(30, 10);
        assert_eq!(s.x_offset(), 20);
        // The static bounds helpers expose the arithmetic a host would otherwise
        // reimplement.
        assert_eq!(ScrollState::max_offset(100, 10), 90);
        assert_eq!(ScrollState::max_offset(5, 10), 0, "floors at zero");
        assert_eq!(ScrollState::max_x_offset(30, 10), 20);
    }

    #[test]
    fn scroll_view_pans_long_lines_horizontally() {
        // One line "0123456789" in a 5-wide viewport panned right by 3 shows the
        // slice starting at display column 3: "34567".
        let mut state = ScrollState::new();
        state.set_x_offset(3);
        let scroll = Scroll::new(vec![Line::from("0123456789")], &state);
        let mut buf = buffer(5, 1);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        scroll.render(area, &mut surface, &ctx);
        assert_eq!(row(&buf, 0), "34567");
    }

    #[test]
    fn horizontal_pan_is_width_aware_across_spans() {
        // "aa你好bb" columns: a(0) a(1) 你(2-3) 好(4-5) b(6) b(7). Panning to
        // column 3 straddles 你 (spans 2-3): it is dropped, and the pan carries
        // across the two spans so 好 and the trailing b's still show.
        let mut state = ScrollState::new();
        state.set_x_offset(3);
        let line = Line::from(vec![
            Span::raw("aa你"), // first span ends mid-way through the pan
            Span::raw("好bb"),
        ]);
        let scroll = Scroll::new(vec![line], &state);
        let mut buf = buffer(6, 1);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        scroll.render(area, &mut surface, &ctx);
        let rendered = row(&buf, 0);
        assert!(
            rendered.contains("好"),
            "resumes at the next whole glyph: {rendered:?}"
        );
        assert!(
            rendered.contains("bb"),
            "pan carried across spans: {rendered:?}"
        );
        assert!(
            !rendered.contains("你"),
            "straddling wide glyph dropped: {rendered:?}"
        );
    }

    /// The streaming host's loop: follow the newest content, hold position when
    /// the reader scrolls back, and resume following when they page down to the
    /// end again. `End` is the shortcut for the last step
    /// ([`scroll_end_key_rearms_bottom_stick`]); this covers arriving there by
    /// scrolling, which is how a reader normally does it.
    #[test]
    fn scrolling_back_to_the_bottom_resumes_following() {
        let mut s = ScrollState::new();
        s.clamp(100, 10);
        assert_eq!(s.offset(), 90);

        // Read back a couple of screens.
        let up = Event::Mouse(Mouse::at(MouseKind::ScrollUp, 0, 0));
        let _ = s.handle(&up, 100, 10);
        let _ = s.handle(&up, 100, 10);
        assert_eq!(s.offset(), 84);
        assert!(!s.is_stuck_to_bottom());

        // Deltas keep arriving while the reader is back there: the view must not
        // move under them.
        s.clamp(130, 10);
        assert_eq!(s.offset(), 84, "content growth must not yank a reader back");

        // Page down until the end is reached; arriving there re-arms the stick.
        let down = Event::Key(Key::new(KeyCode::PageDown));
        for _ in 0..8 {
            if s.is_stuck_to_bottom() {
                break;
            }
            let _ = s.handle(&down, 130, 10);
        }
        assert!(
            s.is_stuck_to_bottom(),
            "reaching the bottom resumes following"
        );
        assert_eq!(s.offset(), 120);

        // ...so the next delta scrolls itself into view again.
        s.clamp(160, 10);
        assert_eq!(s.offset(), 150);
    }

    #[test]
    fn scroll_end_key_rearms_bottom_stick() {
        let mut s = ScrollState::new();
        s.clamp(100, 10);
        s.jump_to_top();
        assert_eq!(s.offset(), 0);
        assert!(!s.is_stuck_to_bottom());
        let end = Event::Key(Key::new(KeyCode::End));
        assert_eq!(s.handle(&end, 100, 10), InputOutcome::Changed);
        assert_eq!(s.offset(), 90);
        assert!(s.is_stuck_to_bottom());
    }

    #[test]
    fn scroll_view_windows_content_and_draws_scrollbar() {
        let lines: Vec<Line<'static>> = (0..20).map(|i| Line::from(format!("line{i}"))).collect();
        let mut state = ScrollState::new();
        state.clamp(20, 5); // stuck to bottom => offset 15
        let scroll = Scroll::new(lines, &state);
        let mut buf = buffer(10, 5);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        scroll.render(area, &mut surface, &ctx);
        // Bottom-stuck: shows the last five lines (15..20).
        assert!(row(&buf, 0).starts_with("line15"));
        assert!(row(&buf, 4).starts_with("line19"));
        // Scrollbar drawn in the last column somewhere.
        let has_bar = (0..5).any(|y| {
            let c = buf[(9, y)].symbol().to_string();
            c == "█" || c == "│"
        });
        assert!(has_bar, "expected a scrollbar in the right column");
    }

    #[test]
    fn scroll_windowed_matches_full_render() {
        // `windowed` holds only the visible slice but must paint byte-for-byte
        // identically to `new` (which owns the whole content), scrollbar and all.
        let content: Vec<Line<'static>> =
            (0..1000).map(|i| Line::from(format!("line{i}"))).collect();
        let (width, height) = (12u16, 6u16);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);

        // A mid-content offset (not top, not bottom) exercises the window origin.
        let mut state = ScrollState::new();
        state.clamp(content.len(), height as usize);
        state.jump_to_top();
        let end = Event::Key(Key::new(KeyCode::PageDown));
        let _ = state.handle(&end, content.len(), height as usize); // one page down
        let offset = state.offset();
        assert!(offset > 0 && offset < content.len() - height as usize);

        let render = |scroll: Scroll| {
            let mut buf = buffer(width, height);
            let area = buf.area;
            let mut surface = Surface::new(&mut buf, area);
            scroll.render(area, &mut surface, &ctx);
            buf
        };

        let full = render(Scroll::new(content.clone(), &state));
        // The window is exactly what `windowed`'s caller would slice.
        let window = content[offset..offset + height as usize].to_vec();
        let windowed = render(Scroll::windowed(window, content.len(), &state));

        assert_eq!(
            full.content, windowed.content,
            "windowed render diverged from the full render"
        );
    }

    #[test]
    fn scrollbar_thumb_and_track_use_theme() {
        let t = rainbow_theme();
        let lines: Vec<Line<'static>> = (0..30).map(|i| Line::from(format!("l{i}"))).collect();
        let mut state = ScrollState::new();
        state.clamp(30, 5);
        let scroll = Scroll::new(lines, &state);
        let mut buf = buffer(10, 5);
        let area = buf.area;
        let ctx = RenderCtx::new(&t);
        let mut surface = Surface::new(&mut buf, area);
        scroll.render(area, &mut surface, &ctx);
        let col = 9; // scrollbar column
        let fgs: Vec<Color> = (0..5).map(|y| buf[(col, y)].fg).collect();
        assert!(fgs.contains(&t.muted), "thumb uses theme.muted: {fgs:?}");
        assert!(fgs.contains(&t.dim), "track uses theme.dim: {fgs:?}");
    }
}
