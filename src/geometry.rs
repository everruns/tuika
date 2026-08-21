//! Geometry helpers shared across `tuika`.
//!
//! [`Rect`] is the area type every [`View`](crate::View) is handed, and
//! [`Position`] a single cell coordinate. The layout solver, though, reasons
//! about a direction-agnostic main/cross axis: an [`Axis`] projects a `Rect`
//! onto its main or cross extent so the flex solver can be written once for
//! rows and columns.
//!
//! Coordinates are `u16` and every operation saturates. A terminal can report a
//! degenerate or absurd size, and a custom `View` can hand back any `Rect` it
//! likes, so arithmetic that could overflow clamps instead of panicking — the
//! size sweep in the test suite walks from `0×0` up asserting exactly that.

/// A single cell coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    /// Column, counted from the left edge.
    pub x: u16,
    /// Row, counted from the top edge.
    pub y: u16,
}

impl Position {
    /// The top-left corner.
    pub const ORIGIN: Position = Position { x: 0, y: 0 };

    /// A position at `x`, `y`.
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

impl From<(u16, u16)> for Position {
    fn from((x, y): (u16, u16)) -> Self {
        Self { x, y }
    }
}

impl From<Position> for (u16, u16) {
    fn from(position: Position) -> Self {
        (position.x, position.y)
    }
}

/// A rectangular region of the cell grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Column of the top-left corner.
    pub x: u16,
    /// Row of the top-left corner.
    pub y: u16,
    /// Width in cells.
    pub width: u16,
    /// Height in cells.
    pub height: u16,
}

impl Rect {
    /// A zero-size rect at the origin.
    pub const ZERO: Rect = Rect {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    /// A rect at `x`, `y` with the given extents.
    ///
    /// The extents are clamped so the far edge cannot exceed [`u16::MAX`]:
    /// `Rect::new(u16::MAX - 1, 0, 10, 1)` is one cell wide, not a wrapped
    /// rect. Callers therefore never have to pre-check their own arithmetic.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width: x.saturating_add(width) - x,
            height: y.saturating_add(height) - y,
        }
    }

    /// Cell count, as `u32` because `width * height` overflows `u16`.
    pub const fn area(self) -> u32 {
        (self.width as u32) * (self.height as u32)
    }

    /// Whether this rect covers no cells.
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Column of the left edge (inclusive).
    pub const fn left(self) -> u16 {
        self.x
    }

    /// Column just past the right edge (exclusive).
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// Row of the top edge (inclusive).
    pub const fn top(self) -> u16 {
        self.y
    }

    /// Row just past the bottom edge (exclusive).
    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// The overlapping region, or a zero-size rect when they do not overlap.
    ///
    /// This is what [`Surface`](crate::Surface) clips every write with, so a
    /// child can never paint outside the area layout gave it.
    #[must_use = "returns a new Rect"]
    pub fn intersection(self, other: Self) -> Self {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());
        Self {
            x: x1,
            y: y1,
            width: x2.saturating_sub(x1),
            height: y2.saturating_sub(y1),
        }
    }

    /// The smallest rect covering both.
    #[must_use = "returns a new Rect"]
    pub fn union(self, other: Self) -> Self {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = self.right().max(other.right());
        let y2 = self.bottom().max(other.bottom());
        Self {
            x: x1,
            y: y1,
            width: x2.saturating_sub(x1),
            height: y2.saturating_sub(y1),
        }
    }

    /// Whether the two rects share at least one cell.
    pub const fn intersects(self, other: Self) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    /// Whether `position` falls inside this rect.
    pub const fn contains(self, position: Position) -> bool {
        position.x >= self.x
            && position.x < self.right()
            && position.y >= self.y
            && position.y < self.bottom()
    }

    /// Move and shrink this rect so it fits inside `other`.
    ///
    /// Size is reduced first, then the origin is pulled back — so the result is
    /// always inside `other`, even when this rect started larger.
    #[must_use = "returns a new Rect"]
    pub fn clamp(self, other: Self) -> Self {
        let width = self.width.min(other.width);
        let height = self.height.min(other.height);
        let x = self.x.clamp(other.x, other.right().saturating_sub(width));
        let y = self.y.clamp(other.y, other.bottom().saturating_sub(height));
        Self::new(x, y, width, height)
    }

    /// The top-left corner.
    pub const fn as_position(self) -> Position {
        Position {
            x: self.x,
            y: self.y,
        }
    }

    /// The extents, discarding the origin.
    pub const fn as_size(self) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    /// Every row of this rect, top to bottom, each a full-width one-row rect.
    pub fn rows(self) -> impl Iterator<Item = Rect> {
        (self.top()..self.bottom()).map(move |y| Rect::new(self.x, y, self.width, 1))
    }

    /// Every column of this rect, left to right, each a full-height one-column rect.
    pub fn columns(self) -> impl Iterator<Item = Rect> {
        (self.left()..self.right()).map(move |x| Rect::new(x, self.y, 1, self.height))
    }

    /// Every cell position in this rect, in row-major order.
    pub fn positions(self) -> impl Iterator<Item = Position> {
        (self.top()..self.bottom())
            .flat_map(move |y| (self.left()..self.right()).map(move |x| Position { x, y }))
    }
}

impl From<(Position, Size)> for Rect {
    fn from((position, size): (Position, Size)) -> Self {
        Self::new(position.x, position.y, size.width, size.height)
    }
}

/// An intrinsic size in terminal cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    /// Width in terminal cells.
    pub width: u16,
    /// Height in terminal cells.
    pub height: u16,
}

impl Size {
    /// A zero-by-zero size.
    pub const ZERO: Size = Size {
        width: 0,
        height: 0,
    };

    /// A size of `width` by `height` cells.
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }

    /// Clamp both extents so the size fits inside `bounds`.
    pub fn clamp_to(self, bounds: Size) -> Size {
        Size {
            width: self.width.min(bounds.width),
            height: self.height.min(bounds.height),
        }
    }
}

impl From<Rect> for Size {
    fn from(rect: Rect) -> Self {
        Size {
            width: rect.width,
            height: rect.height,
        }
    }
}

/// The layout axis a flex container distributes children along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Main axis runs left-to-right; children stack in a row.
    Horizontal,
    /// Main axis runs top-to-bottom; children stack in a column.
    Vertical,
}

impl Axis {
    /// Extent of `size` along this axis (the "main" extent).
    pub fn main(self, size: Size) -> u16 {
        match self {
            Axis::Horizontal => size.width,
            Axis::Vertical => size.height,
        }
    }

    /// Extent of `size` across this axis (the "cross" extent).
    pub fn cross(self, size: Size) -> u16 {
        match self {
            Axis::Horizontal => size.height,
            Axis::Vertical => size.width,
        }
    }

    /// Build a `Size` from main/cross extents on this axis.
    pub fn size(self, main: u16, cross: u16) -> Size {
        match self {
            Axis::Horizontal => Size::new(main, cross),
            Axis::Vertical => Size::new(cross, main),
        }
    }

    /// Place a child rect at `main`/`cross` offsets with `main_len`/`cross_len`
    /// extents, relative to `origin`.
    pub fn place(self, origin: Rect, main: u16, cross: u16, main_len: u16, cross_len: u16) -> Rect {
        match self {
            Axis::Horizontal => Rect {
                x: origin.x.saturating_add(main),
                y: origin.y.saturating_add(cross),
                width: main_len,
                height: cross_len,
            },
            Axis::Vertical => Rect {
                x: origin.x.saturating_add(cross),
                y: origin.y.saturating_add(main),
                width: cross_len,
                height: main_len,
            },
        }
    }
}

/// Symmetric-per-edge padding, in cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    /// Cells inset from the left edge.
    pub left: u16,
    /// Cells inset from the right edge.
    pub right: u16,
    /// Cells inset from the top edge.
    pub top: u16,
    /// Cells inset from the bottom edge.
    pub bottom: u16,
}

impl Padding {
    /// No padding on any edge.
    pub const ZERO: Padding = Padding {
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
    };

    /// Uniform padding on every edge.
    pub fn all(value: u16) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }

    /// Independent horizontal and vertical padding.
    pub fn symmetric(horizontal: u16, vertical: u16) -> Self {
        Self {
            left: horizontal,
            right: horizontal,
            top: vertical,
            bottom: vertical,
        }
    }

    /// Combined left + right padding, in cells.
    pub fn horizontal(self) -> u16 {
        self.left.saturating_add(self.right)
    }

    /// Combined top + bottom padding, in cells.
    pub fn vertical(self) -> u16 {
        self.top.saturating_add(self.bottom)
    }

    /// Shrink `area` by this padding, saturating so it never inverts. The
    /// origin offset is clamped to the area's extent so an area smaller than
    /// its padding stays inside `area` (a zero-size box at the edge) instead of
    /// pushing the inner rect past the right/bottom edge.
    pub fn inner(self, area: Rect) -> Rect {
        let width = area.width.saturating_sub(self.horizontal());
        let height = area.height.saturating_sub(self.vertical());
        Rect {
            x: area.x.saturating_add(self.left.min(area.width)),
            y: area.y.saturating_add(self.top.min(area.height)),
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The saturation rule that keeps the size sweep panic-free: a rect placed
    /// near `u16::MAX` is truncated, never wrapped.
    #[test]
    fn new_clamps_extents_that_would_overflow() {
        let rect = Rect::new(u16::MAX - 1, u16::MAX - 2, 10, 10);
        assert_eq!(rect.width, 1);
        assert_eq!(rect.height, 2);
        assert_eq!(rect.right(), u16::MAX);
        assert_eq!(rect.bottom(), u16::MAX);
    }

    #[test]
    fn area_does_not_overflow_u16() {
        assert_eq!(Rect::new(0, 0, 300, 300).area(), 90_000);
    }

    #[test]
    fn intersection_of_disjoint_rects_is_empty() {
        let left = Rect::new(0, 0, 4, 4);
        let right = Rect::new(10, 10, 4, 4);
        assert!(left.intersection(right).is_empty());
        assert!(!left.intersects(right));
    }

    #[test]
    fn intersection_is_the_overlap() {
        let a = Rect::new(0, 0, 6, 6);
        let b = Rect::new(4, 4, 6, 6);
        assert_eq!(a.intersection(b), Rect::new(4, 4, 2, 2));
        assert!(a.intersects(b));
    }

    /// Rects that only touch edges do not overlap — an off-by-one here would
    /// let a clipped child paint one column into its sibling.
    #[test]
    fn adjacent_rects_do_not_intersect() {
        let a = Rect::new(0, 0, 4, 4);
        let b = Rect::new(4, 0, 4, 4);
        assert!(!a.intersects(b));
        assert!(a.intersection(b).is_empty());
    }

    #[test]
    fn union_covers_both() {
        let a = Rect::new(0, 0, 2, 2);
        let b = Rect::new(4, 4, 2, 2);
        assert_eq!(a.union(b), Rect::new(0, 0, 6, 6));
    }

    #[test]
    fn contains_is_half_open() {
        let rect = Rect::new(1, 1, 2, 2);
        assert!(rect.contains(Position::new(1, 1)));
        assert!(rect.contains(Position::new(2, 2)));
        assert!(!rect.contains(Position::new(3, 2)), "right edge excluded");
        assert!(!rect.contains(Position::new(2, 3)), "bottom edge excluded");
        assert!(!rect.contains(Position::new(0, 1)));
    }

    /// An overlay larger than the screen has to end up inside it, not merely
    /// moved — this is what keeps a too-big dialog on screen.
    #[test]
    fn clamp_shrinks_then_moves_inside() {
        let bounds = Rect::new(0, 0, 10, 10);
        let oversized = Rect::new(8, 8, 20, 20).clamp(bounds);
        assert_eq!(oversized, bounds);

        let overhanging = Rect::new(8, 8, 4, 4).clamp(bounds);
        assert_eq!(overhanging, Rect::new(6, 6, 4, 4));
    }

    #[test]
    fn rows_and_columns_walk_the_rect() {
        let rect = Rect::new(2, 3, 2, 2);
        let rows: Vec<_> = rect.rows().collect();
        assert_eq!(rows, vec![Rect::new(2, 3, 2, 1), Rect::new(2, 4, 2, 1)]);
        let columns: Vec<_> = rect.columns().collect();
        assert_eq!(columns, vec![Rect::new(2, 3, 1, 2), Rect::new(3, 3, 1, 2)]);
    }

    #[test]
    fn positions_are_row_major() {
        let positions: Vec<_> = Rect::new(0, 0, 2, 2).positions().collect();
        assert_eq!(
            positions,
            vec![
                Position::new(0, 0),
                Position::new(1, 0),
                Position::new(0, 1),
                Position::new(1, 1),
            ]
        );
    }

    #[test]
    fn an_empty_rect_yields_no_positions() {
        let empty = Rect::new(5, 5, 0, 3);
        assert_eq!(empty.rows().count(), 3);
        assert_eq!(empty.positions().count(), 0);
        assert_eq!(Rect::ZERO.rows().count(), 0);
    }
}
