//! A deliberately small row-major terminal grid.

use crate::geometry::Rect;

use crate::geometry::{Padding, Size};
use crate::surface::Surface;
use crate::view::{AvailableSpace, Element, MeasureRequest, RenderCtx, ScopedElement, View};

/// An equal-column, row-major grid for terminal interfaces.
///
/// This is not CSS Grid: it intentionally has no named lines, implicit tracks,
/// dense packing, or spanning. It covers the common TUI case with a stable
/// column count, exact cell rounding, intrinsic row heights, gaps, and padding.
///
/// ![grid demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/grid.png)
pub struct Grid<V: View = Element> {
    columns: u16,
    row_gap: u16,
    column_gap: u16,
    padding: Padding,
    children: Vec<V>,
}

impl<V: View> Grid<V> {
    fn empty(columns: u16) -> Self {
        Self {
            columns: columns.max(1),
            row_gap: 0,
            column_gap: 0,
            padding: Padding::ZERO,
            children: Vec::new(),
        }
    }

    /// Set both row and column gaps.
    pub fn gap(mut self, gap: u16) -> Self {
        self.row_gap = gap;
        self.column_gap = gap;
        self
    }

    /// Set the gap between rows.
    pub fn row_gap(mut self, gap: u16) -> Self {
        self.row_gap = gap;
        self
    }

    /// Set the gap between columns.
    pub fn column_gap(mut self, gap: u16) -> Self {
        self.column_gap = gap;
        self
    }

    /// Set inner padding.
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Append a cell in row-major order.
    pub fn cell(mut self, view: V) -> Self {
        self.children.push(view);
        self
    }

    /// Resolve one rectangle per cell without rendering.
    pub fn solve(&self, area: Rect, ctx: &RenderCtx) -> Vec<Rect> {
        if self.children.is_empty() {
            return Vec::new();
        }
        let inner = self.padding.inner(area);
        let columns = usize::from(self.columns);
        let total_column_gap = self
            .column_gap
            .saturating_mul(columns.saturating_sub(1) as u16);
        let track_space = inner.width.saturating_sub(total_column_gap);
        let track = |column: usize| {
            let start = u32::from(track_space) * column as u32 / self.columns as u32;
            let end = u32::from(track_space) * (column + 1) as u32 / self.columns as u32;
            (start as u16, (end - start) as u16)
        };

        let row_count = self.children.len().div_ceil(columns);
        let mut row_heights = Vec::with_capacity(row_count);
        for row in 0..row_count {
            let mut height = 0u16;
            for (offset, child) in self.children[row * columns..]
                .iter()
                .take(columns)
                .enumerate()
            {
                let (_, width) = track(offset);
                let measured = child.measure_request(
                    MeasureRequest::new(Size::new(width, inner.height)).with_known_width(width),
                    ctx,
                );
                height = height.max(measured.height);
            }
            row_heights.push(height);
        }

        let mut rects = Vec::with_capacity(self.children.len());
        let mut y = inner.y;
        for (row, row_height) in row_heights.into_iter().enumerate() {
            for column in 0..columns {
                let index = row * columns + column;
                if index >= self.children.len() {
                    break;
                }
                let (track_start, width) = track(column);
                let x = inner
                    .x
                    .saturating_add(track_start)
                    .saturating_add(self.column_gap.saturating_mul(column as u16));
                let height = row_height.min(inner.bottom().saturating_sub(y));
                rects.push(Rect::new(
                    x.min(inner.right()),
                    y.min(inner.bottom()),
                    width,
                    height,
                ));
            }
            y = y.saturating_add(row_height).saturating_add(self.row_gap);
        }
        rects
    }
}

impl Grid<Element> {
    /// Create an empty owned grid with `columns` equal-width columns.
    pub fn new(columns: u16) -> Self {
        Self::empty(columns)
    }

    /// Create a grid whose cells may borrow frame state.
    pub fn scoped<'view>(columns: u16) -> Grid<ScopedElement<'view>> {
        Grid::empty(columns)
    }
}

impl<V: View> View for Grid<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let rects = self.solve(Rect::new(0, 0, available.width, available.height), ctx);
        Size::new(
            rects
                .iter()
                .map(|rect| rect.right())
                .max()
                .unwrap_or(self.padding.left)
                .saturating_add(self.padding.right),
            rects
                .iter()
                .map(|rect| rect.bottom())
                .max()
                .unwrap_or(self.padding.top)
                .saturating_add(self.padding.bottom),
        )
        .clamp_to(available)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        if matches!(request.available_width, AvailableSpace::Definite(_))
            && matches!(request.available_height, AvailableSpace::Definite(_))
        {
            return request.resolve(self.measure(request.fallback_available(), ctx));
        }

        let columns = usize::from(self.columns);
        let concrete_width = request.known_width.or(match request.available_width {
            AvailableSpace::Definite(width) => Some(width),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        });
        let total_gap = self
            .column_gap
            .saturating_mul(self.columns.saturating_sub(1));
        let track_width = concrete_width.map(|width| {
            width
                .saturating_sub(self.padding.horizontal())
                .saturating_sub(total_gap)
                / self.columns
        });
        let mut row_heights = vec![0u16; self.children.len().div_ceil(columns)];
        let mut widest = 0u16;
        for (index, child) in self.children.iter().enumerate() {
            let mut child_request = request;
            if let Some(width) = track_width {
                child_request.known_width = Some(width);
                child_request.available_width = AvailableSpace::Definite(width);
            }
            let measured = child.measure_request(child_request, ctx);
            widest = widest.max(measured.width);
            row_heights[index / columns] = row_heights[index / columns].max(measured.height);
        }
        let rows = row_heights.len() as u16;
        let width = widest
            .saturating_mul(self.columns)
            .saturating_add(total_gap)
            .saturating_add(self.padding.horizontal());
        let height = row_heights
            .into_iter()
            .fold(0u16, u16::saturating_add)
            .saturating_add(self.row_gap.saturating_mul(rows.saturating_sub(1)))
            .saturating_add(self.padding.vertical());
        request.resolve(Size::new(width, height))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        for (child, rect) in self.children.iter().zip(self.solve(area, ctx)) {
            child.render(rect, &mut surface.child(rect), ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;

    #[test]
    fn grid_rounds_track_boundaries_without_losing_cells() {
        let grid = Grid::new(3)
            .cell(crate::element(Text::raw("a")))
            .cell(crate::element(Text::raw("b")))
            .cell(crate::element(Text::raw("c")));
        let theme = crate::Theme::default();
        let rects = grid.solve(Rect::new(0, 0, 10, 1), &RenderCtx::new(&theme));
        assert_eq!(
            rects.iter().map(|rect| rect.width).collect::<Vec<_>>(),
            [3, 3, 4]
        );
        assert_eq!(rects.last().unwrap().right(), 10);
    }
}
