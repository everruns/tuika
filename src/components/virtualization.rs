//! Shared window and scrollbar primitives for virtualized collections.

use std::ops::Range;

use crate::geometry::Rect;
use crate::style::Style;

use crate::geometry::Size;
use crate::style::StyleRole;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

/// A clamped contiguous window into a larger logical collection.
///
/// `VirtualWindow` contains no data and owns no persistent state. A host keeps
/// its offset or selection, creates a window for the current frame, and uses
/// [`range`](Self::range) to fetch only the visible records. Built-in lists,
/// tables, line scrolling, item scrolling, and viewports use the same math.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualWindow {
    total: usize,
    start: usize,
    len: usize,
}

impl VirtualWindow {
    /// Create a window over `total` entries, showing at most `visible` entries
    /// from `start`. Both the length and start are clamped to the collection.
    pub fn new(total: usize, visible: usize, start: usize) -> Self {
        let len = visible.min(total);
        let start = start.min(Self::max_start_for(total, len));
        Self { total, start, len }
    }

    /// Largest valid start for `visible` entries in a collection of `total`.
    ///
    /// This is the allocation-free bound used by input handlers before a full
    /// frame window exists.
    pub const fn max_start_for(total: usize, visible: usize) -> usize {
        total.saturating_sub(visible)
    }

    /// Create a window centered on `anchor` where possible.
    ///
    /// A missing anchor starts at the beginning. An out-of-range anchor is
    /// clamped to the last entry, so stale host selection cannot overflow the
    /// range calculation.
    pub fn around(total: usize, visible: usize, anchor: Option<usize>) -> Self {
        if total == 0 {
            return Self::default();
        }
        let len = visible.min(total);
        let anchor = anchor.unwrap_or(0).min(total - 1);
        Self::new(total, len, anchor.saturating_sub(len / 2))
    }

    /// Create the window that keeps `anchor` visible with the smallest possible
    /// adjustment of `start`.
    ///
    /// `start` is the caller's current top entry — a host's persistent offset,
    /// or `0` for a stateless caller. The window does not move while the anchor
    /// is inside it; when the anchor leaves, it trails the anchor by exactly one
    /// edge. This is the complement of [`around`](Self::around), which recenters
    /// on every move.
    ///
    /// A missing anchor leaves `start` alone (clamped to the collection); an
    /// out-of-range anchor is clamped to the last entry.
    pub fn keeping(total: usize, visible: usize, start: usize, anchor: Option<usize>) -> Self {
        if total == 0 {
            return Self::default();
        }
        let len = visible.min(total);
        let mut start = start.min(Self::max_start_for(total, len));
        if len > 0
            && let Some(anchor) = anchor
        {
            let anchor = anchor.min(total - 1);
            if anchor < start {
                start = anchor;
            } else if anchor >= start.saturating_add(len) {
                start = anchor.saturating_add(1).saturating_sub(len);
            }
        }
        Self::new(total, len, start)
    }

    /// Total entries in the logical collection.
    pub const fn total(self) -> usize {
        self.total
    }

    /// Absolute index of the first visible entry.
    pub const fn start(self) -> usize {
        self.start
    }

    /// Number of visible entries.
    pub const fn len(self) -> usize {
        self.len
    }

    /// Whether the visible window is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// One-past-the-end absolute index of the visible entries.
    pub fn end(self) -> usize {
        self.start.saturating_add(self.len).min(self.total)
    }

    /// Absolute index range of the visible entries.
    pub fn range(self) -> Range<usize> {
        self.start..self.end()
    }

    /// Largest valid start for a window of this length.
    pub fn max_start(self) -> usize {
        Self::max_start_for(self.total, self.len)
    }

    /// Whether some entries lie outside the window.
    pub const fn overflows(self) -> bool {
        self.total > self.len
    }

    /// Whether `index` is visible in this window.
    pub fn contains(self, index: usize) -> bool {
        self.range().contains(&index)
    }
}

/// How a stateless component places its window around the selection.
///
/// Only consulted when a component resolves its own window — a host that
/// supplies an explicit window (`visible_window`, or a `source_window` it
/// already sliced) has already made this decision itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionAnchor {
    /// Keep the selection mid-window, recentering on every move
    /// ([`VirtualWindow::around`]). The default.
    #[default]
    Center,
    /// Move the window only when the selection would leave it
    /// ([`VirtualWindow::keeping`]).
    ///
    /// A stateless component starts at the top of the collection, so the
    /// selection rides the bottom edge moving down and the top edge moving up —
    /// the familiar list-scrolling policy.
    Edge,
}

impl SelectionAnchor {
    /// Resolve the stateless window for `selected` under this policy.
    pub(crate) fn window(
        self,
        total: usize,
        visible: usize,
        selected: Option<usize>,
    ) -> VirtualWindow {
        match self {
            Self::Center => VirtualWindow::around(total, visible, selected),
            Self::Edge => VirtualWindow::keeping(total, visible, 0, selected),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Orientation {
    Vertical,
    Horizontal,
}

/// A scrollbar for a [`VirtualWindow`].
///
/// The same component draws vertical and horizontal bars, uses the active
/// theme by default, and permits local glyph/style overrides. It draws nothing
/// when the whole collection fits in the window.
///
/// ![scrollbar demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/scrollbar.png)
pub struct Scrollbar {
    window: VirtualWindow,
    orientation: Orientation,
    track_glyph: char,
    thumb_glyph: char,
    track_style: Option<Style>,
    thumb_style: Option<Style>,
}

impl Scrollbar {
    /// Create a vertical scrollbar for `window`.
    pub fn vertical(window: VirtualWindow) -> Self {
        Self {
            window,
            orientation: Orientation::Vertical,
            track_glyph: '│',
            thumb_glyph: '█',
            track_style: None,
            thumb_style: None,
        }
    }

    /// Create a horizontal scrollbar for `window`.
    pub fn horizontal(window: VirtualWindow) -> Self {
        Self {
            window,
            orientation: Orientation::Horizontal,
            track_glyph: '─',
            thumb_glyph: '━',
            track_style: None,
            thumb_style: None,
        }
    }

    /// Replace the track glyph.
    pub fn track_glyph(mut self, glyph: char) -> Self {
        self.track_glyph = glyph;
        self
    }

    /// Replace the thumb glyph.
    pub fn thumb_glyph(mut self, glyph: char) -> Self {
        self.thumb_glyph = glyph;
        self
    }

    /// Override the track style for this bar.
    pub fn track_style(mut self, style: Style) -> Self {
        self.track_style = Some(style);
        self
    }

    /// Override the thumb style for this bar.
    pub fn thumb_style(mut self, style: Style) -> Self {
        self.thumb_style = Some(style);
        self
    }

    fn track_len(&self, area: Rect) -> u16 {
        match self.orientation {
            Orientation::Vertical => area.height,
            Orientation::Horizontal => area.width,
        }
    }

    fn thumb(&self, track_len: u16) -> (u16, u16) {
        if track_len == 0 || self.window.total == 0 {
            return (0, 0);
        }
        let track = u128::from(track_len);
        let thumb_len = ((track * self.window.len as u128) / self.window.total as u128)
            .max(1)
            .min(track) as u16;
        let travel = track_len.saturating_sub(thumb_len);
        let max_start = self.window.max_start().max(1);
        let thumb_start = ((self.window.start.min(max_start) as u128 * u128::from(travel))
            / max_start as u128) as u16;
        (thumb_start, thumb_len)
    }
}

impl View for Scrollbar {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let len = self.window.len.min(u16::MAX as usize) as u16;
        match self.orientation {
            Orientation::Vertical => Size::new(available.width.min(1), available.height.min(len)),
            Orientation::Horizontal => Size::new(available.width.min(len), available.height.min(1)),
        }
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() || !self.window.overflows() {
            return;
        }
        let track_len = self.track_len(area);
        let (thumb_start, thumb_len) = self.thumb(track_len);
        let track_style = self.track_style.unwrap_or_else(|| {
            ctx.style(StyleRole::SCROLLBAR_TRACK)
                .apply(Style::default().fg(ctx.theme.dim))
        });
        let thumb_style = self.thumb_style.unwrap_or_else(|| {
            ctx.style(StyleRole::SCROLLBAR_THUMB)
                .apply(Style::default().fg(ctx.theme.muted))
        });
        for index in 0..track_len {
            let thumb = index >= thumb_start && index < thumb_start.saturating_add(thumb_len);
            let (glyph, style) = if thumb {
                (self.thumb_glyph, thumb_style)
            } else {
                (self.track_glyph, track_style)
            };
            let (x, y) = match self.orientation {
                Orientation::Vertical => (area.x, area.y.saturating_add(index)),
                Orientation::Horizontal => (area.x.saturating_add(index), area.y),
            };
            surface.set(x, y, glyph, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20-row collection in a 5-row pane, the two policies side by side.
    #[test]
    fn keeping_trails_the_anchor_while_around_recenters() {
        let edge = |anchor: usize| VirtualWindow::keeping(20, 5, 0, Some(anchor)).start();
        let center = |anchor: usize| VirtualWindow::around(20, 5, Some(anchor)).start();

        // Inside the first window: neither policy can scroll yet.
        assert_eq!(edge(2), 0);
        assert_eq!(center(2), 0);
        // Past the bottom edge: `keeping` trails by one row, `around` centers.
        assert_eq!(edge(7), 3);
        assert_eq!(center(7), 5);
        // At the end both clamp to the last full window.
        assert_eq!(edge(19), 15);
        assert_eq!(center(19), 15);
    }

    #[test]
    fn keeping_holds_start_for_an_anchor_already_inside_the_window() {
        // Moving 7 -> 6 with the window already at 3 changes nothing...
        assert_eq!(VirtualWindow::keeping(20, 5, 3, Some(6)).start(), 3);
        // ...while a stateless caller re-derives the trailing window.
        assert_eq!(VirtualWindow::keeping(20, 5, 0, Some(6)).start(), 2);
        // Above the window, the anchor becomes the top row.
        assert_eq!(VirtualWindow::keeping(20, 5, 10, Some(4)).start(), 4);
    }

    #[test]
    fn keeping_survives_degenerate_input() {
        assert_eq!(
            VirtualWindow::keeping(0, 5, 9, Some(3)),
            VirtualWindow::default()
        );
        // No anchor leaves the start alone, clamped to the collection.
        assert_eq!(VirtualWindow::keeping(20, 5, 99, None).start(), 15);
        // A zero-height pane can show no anchor at all, so the start is simply
        // carried through — the same as `new` with an empty window.
        let empty = VirtualWindow::keeping(20, 0, 4, Some(19));
        assert!(empty.is_empty());
        assert_eq!(empty.start(), 4);
        // An out-of-range anchor clamps to the last entry.
        assert_eq!(
            VirtualWindow::keeping(20, 5, 0, Some(usize::MAX)).start(),
            15
        );
    }

    #[test]
    fn selection_anchor_dispatches_to_the_two_policies() {
        assert_eq!(SelectionAnchor::default(), SelectionAnchor::Center);
        assert_eq!(SelectionAnchor::Center.window(20, 5, Some(7)).start(), 5);
        assert_eq!(SelectionAnchor::Edge.window(20, 5, Some(7)).start(), 3);
    }
}
