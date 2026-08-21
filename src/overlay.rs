//! Overlay positioning (Pi's overlay model).
//!
//! An overlay is a view drawn on top of the base tree at an anchored, sized
//! rect. Because the full-screen renderer owns the whole alternate-screen
//! buffer, an overlay is just "paint this last, into this rect, after clearing
//! it" — none of the inline-viewport chrome-bleed the inline path fights. The
//! [`OverlaySpec`] resolves a [`Rect`] from either a screen [`Anchor`] or a
//! [`TargetPlacement`] relative to another laid-out view, plus size constraints
//! relative to the screen.
//!
//! The two types here are the two halves of one pipeline, which is why they
//! share a module: an [`OverlaySpec`] says *where* an overlay wants to be and
//! answers with a `Rect`; an [`Overlay`] pairs that resolved rect with the view
//! to paint into it, and is what [`paint`](crate::host::paint) consumes.

use crate::geometry::Rect;

use super::geometry::Size;
use super::view::View;

/// An overlay to composite over the base tree at a resolved rect.
///
/// Produced by pairing a view with the `Rect` an [`OverlaySpec`] resolved to,
/// then handed to [`paint`](crate::host::paint) — overlays are composited in
/// slice order, so the last one is topmost.
pub struct Overlay<'a> {
    /// Rect the overlay is composited into.
    pub area: Rect,
    /// View painted within `area`.
    pub view: &'a dyn View,
    /// Clear the rect (fill with the theme background) before painting.
    pub clear: bool,
}

/// Where an overlay sits relative to the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    /// Centered horizontally and vertically.
    #[default]
    Center,
    /// Centered horizontally, flush to the top edge.
    Top,
    /// Centered horizontally, flush to the bottom edge.
    Bottom,
    /// Centered vertically, flush to the left edge.
    Left,
    /// Centered vertically, flush to the right edge.
    Right,
    /// Flush to the top-left corner.
    TopLeft,
    /// Flush to the top-right corner.
    TopRight,
    /// Flush to the bottom-left corner.
    BottomLeft,
    /// Flush to the bottom-right corner.
    BottomRight,
}

/// Which side of a target rect an overlay prefers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TargetSide {
    /// Place the overlay before the target on the vertical axis.
    Above,
    /// Place the overlay after the target on the vertical axis.
    #[default]
    Below,
    /// Place the overlay before the target on the horizontal axis.
    Left,
    /// Place the overlay after the target on the horizontal axis.
    Right,
}

impl TargetSide {
    fn opposite(self) -> Self {
        match self {
            Self::Above => Self::Below,
            Self::Below => Self::Above,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// How an overlay aligns along the target's cross axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TargetAlign {
    /// Align the overlay's leading edge with the target's leading edge.
    Start,
    /// Center the overlay on the target.
    #[default]
    Center,
    /// Align the overlay's trailing edge with the target's trailing edge.
    End,
}

/// Placement of an overlay relative to a target rect.
///
/// Placement defaults to centered below the target with automatic flipping.
/// When the preferred side cannot fit the overlay and its opposite has more
/// room, the resolver flips sides, then clamps the result within the screen
/// margin. Disable that behavior with [`flip(false)`](Self::flip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetPlacement {
    side: TargetSide,
    align: TargetAlign,
    gap: u16,
    flip: bool,
}

impl Default for TargetPlacement {
    fn default() -> Self {
        Self {
            side: TargetSide::Below,
            align: TargetAlign::Center,
            gap: 0,
            flip: true,
        }
    }
}

impl TargetPlacement {
    /// Create a centered placement on `side`, with automatic flipping enabled.
    pub fn new(side: TargetSide) -> Self {
        Self {
            side,
            ..Self::default()
        }
    }

    /// Place below the target.
    pub fn below() -> Self {
        Self::new(TargetSide::Below)
    }

    /// Place above the target.
    pub fn above() -> Self {
        Self::new(TargetSide::Above)
    }

    /// Place to the left of the target.
    pub fn left() -> Self {
        Self::new(TargetSide::Left)
    }

    /// Place to the right of the target.
    pub fn right() -> Self {
        Self::new(TargetSide::Right)
    }

    /// Set cross-axis alignment against the target.
    pub fn align(mut self, align: TargetAlign) -> Self {
        self.align = align;
        self
    }

    /// Set the gap between the target and overlay, in cells.
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Enable or disable automatic placement on the opposite side.
    pub fn flip(mut self, flip: bool) -> Self {
        self.flip = flip;
        self
    }
}

/// A size request: an absolute cell count or a percentage of the screen extent,
/// with optional min/max clamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// An absolute size in terminal cells.
    Cells(u16),
    /// A percentage (0–100) of the available screen extent.
    Percent(u16),
}

impl Extent {
    fn resolve(self, available: u16) -> u16 {
        match self {
            Extent::Cells(n) => n,
            Extent::Percent(p) => ((available as u32 * p.min(100) as u32) / 100) as u16,
        }
    }
}

/// Anchored, sized overlay placement relative to a screen rect.
///
/// Resolve a [`Rect`] for the current screen, then paint a view into it last
/// (see [`paint`](crate::paint)); the host routes input to whichever layer owns
/// focus.
///
/// # Examples
///
/// ```
/// use tuika::OverlaySpec;
/// use tuika::ui::Rect;
///
/// // A centered dialog at 50% × 40% of a 100 × 40 screen, never smaller than 34 × 7.
/// let rect = OverlaySpec::centered(50, 40).min_size(34, 7).resolve(Rect::new(0, 0, 100, 40));
/// assert_eq!((rect.width, rect.height), (50, 16));
/// assert_eq!((rect.x, rect.y), (25, 12));
/// ```
///
/// ![overlay demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/overlay.png)
#[derive(Clone, Copy, Debug)]
pub struct OverlaySpec {
    /// Where the resolved rect sits within the screen.
    pub anchor: Anchor,
    /// Requested width, absolute or a percentage of the screen width.
    pub width: Extent,
    /// Requested height, absolute or a percentage of the screen height.
    pub height: Extent,
    /// Lower clamp on the resolved width, in cells.
    pub min_width: u16,
    /// Lower clamp on the resolved height, in cells.
    pub min_height: u16,
    /// Upper clamp on the resolved width, in cells.
    pub max_width: u16,
    /// Upper clamp on the resolved height, in cells.
    pub max_height: u16,
    /// Gap in cells kept between the overlay and every screen edge.
    pub margin: u16,
}

impl OverlaySpec {
    /// A centered overlay sized as a percentage of the screen.
    pub fn centered(width_pct: u16, height_pct: u16) -> Self {
        Self {
            anchor: Anchor::Center,
            width: Extent::Percent(width_pct),
            height: Extent::Percent(height_pct),
            min_width: 0,
            min_height: 0,
            max_width: u16::MAX,
            max_height: u16::MAX,
            margin: 0,
        }
    }

    /// Set the anchor that positions the overlay within the screen.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Set the minimum resolved size in cells.
    pub fn min_size(mut self, width: u16, height: u16) -> Self {
        self.min_width = width;
        self.min_height = height;
        self
    }

    /// Set the maximum resolved size in cells.
    pub fn max_size(mut self, width: u16, height: u16) -> Self {
        self.max_width = width;
        self.max_height = height;
        self
    }

    /// Set the gap in cells kept between the overlay and every screen edge.
    pub fn margin(mut self, margin: u16) -> Self {
        self.margin = margin;
        self
    }

    /// Resolve the overlay rect within `screen`.
    pub fn resolve(&self, screen: Rect) -> Rect {
        let (inner, w, h) = self.bounds_and_size(screen);
        let (x, y) = self.position(inner, w, h);
        // Clamp into the screen: a margin wider than the screen would otherwise
        // place `inner` (and the overlay) past the edge.
        let x = x.min(screen.right().saturating_sub(w)).max(screen.x);
        let y = y.min(screen.bottom().saturating_sub(h)).max(screen.y);
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Resolve an overlay relative to `target`, then keep it within `screen`.
    ///
    /// The requested size and `margin` still resolve against the screen. The
    /// target controls only placement. This makes a popover follow a probed
    /// trigger without coupling either view to the other's layout.
    pub fn resolve_target(&self, screen: Rect, target: Rect, placement: TargetPlacement) -> Rect {
        let (inner, w, h) = self.bounds_and_size(screen);
        let side = if placement.flip {
            let preferred = available_on_side(inner, target, placement.side, placement.gap);
            let opposite =
                available_on_side(inner, target, placement.side.opposite(), placement.gap);
            let needed = match placement.side {
                TargetSide::Above | TargetSide::Below => i32::from(h),
                TargetSide::Left | TargetSide::Right => i32::from(w),
            };
            if preferred < needed && opposite > preferred {
                placement.side.opposite()
            } else {
                placement.side
            }
        } else {
            placement.side
        };

        let (x, y) = target_position(target, w, h, side, placement.align, placement.gap);
        let min_x = i32::from(inner.x);
        let min_y = i32::from(inner.y);
        let max_x = i32::from(inner.right()) - i32::from(w);
        let max_y = i32::from(inner.bottom()) - i32::from(h);
        Rect::new(
            x.clamp(min_x, max_x) as u16,
            y.clamp(min_y, max_y) as u16,
            w,
            h,
        )
    }

    fn bounds_and_size(&self, screen: Rect) -> (Rect, u16, u16) {
        // Keep the inset symmetrical even when the requested margin is larger
        // than half the screen. Degenerate bounds remain inside `screen`.
        let margin_x = self.margin.min(screen.width / 2);
        let margin_y = self.margin.min(screen.height / 2);
        let avail = Size::new(
            screen.width.saturating_sub(margin_x.saturating_mul(2)),
            screen.height.saturating_sub(margin_y.saturating_mul(2)),
        );
        let w = resolve_extent(self.width, avail.width, self.min_width, self.max_width);
        let h = resolve_extent(self.height, avail.height, self.min_height, self.max_height);
        let inner = Rect::new(
            screen.x.saturating_add(margin_x),
            screen.y.saturating_add(margin_y),
            avail.width,
            avail.height,
        );
        (inner, w, h)
    }

    fn position(&self, inner: Rect, w: u16, h: u16) -> (u16, u16) {
        let free_x = inner.width.saturating_sub(w);
        let free_y = inner.height.saturating_sub(h);
        let (left, mid_x, right) = (inner.x, inner.x + free_x / 2, inner.x + free_x);
        let (top, mid_y, bottom) = (inner.y, inner.y + free_y / 2, inner.y + free_y);
        match self.anchor {
            Anchor::Center => (mid_x, mid_y),
            Anchor::Top => (mid_x, top),
            Anchor::Bottom => (mid_x, bottom),
            Anchor::Left => (left, mid_y),
            Anchor::Right => (right, mid_y),
            Anchor::TopLeft => (left, top),
            Anchor::TopRight => (right, top),
            Anchor::BottomLeft => (left, bottom),
            Anchor::BottomRight => (right, bottom),
        }
    }
}

fn resolve_extent(extent: Extent, available: u16, min: u16, max: u16) -> u16 {
    let upper = max.min(available);
    // Contradictory public fields can arise through struct literals. Treat the
    // maximum as the hard safety bound rather than panicking in `clamp`.
    let lower = min.min(upper);
    extent.resolve(available).clamp(lower, upper)
}

fn available_on_side(inner: Rect, target: Rect, side: TargetSide, gap: u16) -> i32 {
    let gap = i32::from(gap);
    let room = match side {
        TargetSide::Above => i32::from(target.y) - gap - i32::from(inner.y),
        TargetSide::Below => i32::from(inner.bottom()) - i32::from(target.bottom()) - gap,
        TargetSide::Left => i32::from(target.x) - gap - i32::from(inner.x),
        TargetSide::Right => i32::from(inner.right()) - i32::from(target.right()) - gap,
    };
    room.max(0)
}

fn target_position(
    target: Rect,
    width: u16,
    height: u16,
    side: TargetSide,
    align: TargetAlign,
    gap: u16,
) -> (i32, i32) {
    let target_x = i32::from(target.x);
    let target_y = i32::from(target.y);
    let target_right = i32::from(target.right());
    let target_bottom = i32::from(target.bottom());
    let width = i32::from(width);
    let height = i32::from(height);
    let gap = i32::from(gap);

    match side {
        TargetSide::Above => (
            align_axis(target_x, target_right, width, align),
            target_y - gap - height,
        ),
        TargetSide::Below => (
            align_axis(target_x, target_right, width, align),
            target_bottom + gap,
        ),
        TargetSide::Left => (
            target_x - gap - width,
            align_axis(target_y, target_bottom, height, align),
        ),
        TargetSide::Right => (
            target_right + gap,
            align_axis(target_y, target_bottom, height, align),
        ),
    }
}

fn align_axis(start: i32, end: i32, extent: i32, align: TargetAlign) -> i32 {
    match align {
        TargetAlign::Start => start,
        TargetAlign::Center => start + (end - start - extent).div_euclid(2),
        TargetAlign::End => end - extent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    #[test]
    fn overlay_centered_percentage() {
        let screen = Rect::new(0, 0, 100, 40);
        let spec = OverlaySpec::centered(50, 50);
        let rect = spec.resolve(screen);
        assert_eq!(rect.width, 50);
        assert_eq!(rect.height, 20);
        assert_eq!(rect.x, 25);
        assert_eq!(rect.y, 10);
    }

    #[test]
    fn overlay_anchors_to_corner_with_margin() {
        let screen = Rect::new(0, 0, 100, 40);
        let spec = OverlaySpec {
            anchor: Anchor::BottomRight,
            width: Extent::Cells(20),
            height: Extent::Cells(10),
            min_width: 0,
            min_height: 0,
            max_width: u16::MAX,
            max_height: u16::MAX,
            margin: 2,
        };
        let rect = spec.resolve(screen);
        assert_eq!(rect.width, 20);
        assert_eq!(rect.height, 10);
        // Bottom-right inside a 2-cell margin: right edge at 98, bottom at 38.
        assert_eq!(rect.right(), 98);
        assert_eq!(rect.bottom(), 38);
    }

    #[test]
    fn overlay_clamps_to_max() {
        let screen = Rect::new(0, 0, 100, 40);
        let spec = OverlaySpec::centered(90, 90).max_size(40, 20);
        let rect = spec.resolve(screen);
        assert_eq!(rect.width, 40);
        assert_eq!(rect.height, 20);
    }

    #[test]
    fn target_overlay_places_below_and_aligns_to_start() {
        let screen = Rect::new(10, 5, 80, 30);
        let target = Rect::new(30, 12, 12, 2);
        let spec = OverlaySpec {
            width: Extent::Cells(20),
            height: Extent::Cells(6),
            ..OverlaySpec::centered(0, 0)
        };

        let rect = spec.resolve_target(
            screen,
            target,
            TargetPlacement::below().align(TargetAlign::Start).gap(1),
        );

        assert_eq!(rect, Rect::new(30, 15, 20, 6));
    }

    #[test]
    fn target_overlay_centers_odd_size_differences_consistently() {
        let screen = Rect::new(0, 0, 30, 12);
        let target = Rect::new(10, 3, 1, 1);
        let spec = OverlaySpec {
            width: Extent::Cells(4),
            height: Extent::Cells(2),
            ..OverlaySpec::centered(0, 0)
        };

        let rect = spec.resolve_target(screen, target, TargetPlacement::below());

        assert_eq!(rect, Rect::new(8, 4, 4, 2));
    }

    #[test]
    fn target_overlay_flips_when_the_opposite_side_has_more_room() {
        let screen = Rect::new(0, 0, 40, 20);
        let target = Rect::new(16, 17, 8, 1);
        let spec = OverlaySpec {
            width: Extent::Cells(14),
            height: Extent::Cells(6),
            ..OverlaySpec::centered(0, 0)
        };

        let rect = spec.resolve_target(screen, target, TargetPlacement::below().gap(1));

        assert_eq!(rect, Rect::new(13, 10, 14, 6));
    }

    #[test]
    fn target_overlay_clamps_cross_axis_inside_screen_margin() {
        let screen = Rect::new(10, 5, 30, 12);
        let target = Rect::new(37, 9, 2, 1);
        let spec = OverlaySpec {
            width: Extent::Cells(12),
            height: Extent::Cells(4),
            ..OverlaySpec::centered(0, 0).margin(2)
        };

        let rect = spec.resolve_target(
            screen,
            target,
            TargetPlacement::below().align(TargetAlign::End),
        );

        assert_eq!(rect, Rect::new(26, 10, 12, 4));
    }

    #[test]
    fn target_overlay_places_on_the_horizontal_axis() {
        let screen = Rect::new(0, 0, 40, 20);
        let target = Rect::new(20, 8, 6, 3);
        let spec = OverlaySpec {
            width: Extent::Cells(6),
            height: Extent::Cells(4),
            ..OverlaySpec::centered(0, 0)
        };

        let rect = spec.resolve_target(
            screen,
            target,
            TargetPlacement::left().align(TargetAlign::End).gap(2),
        );

        assert_eq!(rect, Rect::new(12, 7, 6, 4));
    }

    #[test]
    fn target_overlay_can_disable_flipping() {
        let screen = Rect::new(0, 0, 40, 20);
        let target = Rect::new(35, 8, 2, 1);
        let spec = OverlaySpec {
            width: Extent::Cells(6),
            height: Extent::Cells(3),
            ..OverlaySpec::centered(0, 0)
        };

        let rect = spec.resolve_target(screen, target, TargetPlacement::right().gap(1).flip(false));

        assert_eq!(rect, Rect::new(34, 7, 6, 3));
    }

    #[test]
    fn contradictory_size_clamps_never_panic_and_max_wins() {
        let screen = Rect::new(0, 0, 20, 10);
        let spec = OverlaySpec::centered(100, 100)
            .min_size(18, 9)
            .max_size(8, 4);

        assert_eq!(
            spec.resolve(screen).as_size(),
            Rect::new(0, 0, 8, 4).as_size()
        );
    }
}
