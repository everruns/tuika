//! A two-dimensional viewport over any Tuika view.

use crate::buffer::{Buffer, Cell};
use crate::geometry::Rect;

use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, View};
use crate::width::str_cols;

use super::{ScrollState, Scrollbar, VirtualWindow};

/// A clipped, vertically scrollable and horizontally pannable child view.
///
/// Unlike [`Scroll`](super::Scroll), which is optimized for owned
/// [`Line`](crate::text::Line) rows, `Viewport` accepts any child view.
/// The host supplies the child's full cell extent and persists offsets in
/// [`ScrollState`].
///
/// ![viewport demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/primitives.gif)
pub struct Viewport<V: View = Element> {
    child: V,
    content: Size,
    offset: usize,
    x_offset: usize,
    vertical_scrollbar: bool,
    horizontal_scrollbar: bool,
}

impl<V: View> Viewport<V> {
    /// Create a viewport for `child` with its full un-clipped `content` extent.
    pub fn new(child: V, content: Size, state: &ScrollState) -> Self {
        Self {
            child,
            content,
            offset: state.offset(),
            x_offset: state.x_offset(),
            vertical_scrollbar: true,
            horizontal_scrollbar: false,
        }
    }

    /// Show or hide the right-edge scrollbar (default true when overflowing).
    pub fn vertical_scrollbar(mut self, show: bool) -> Self {
        self.vertical_scrollbar = show;
        self
    }

    /// Show or hide the bottom scrollbar (default false).
    pub fn horizontal_scrollbar(mut self, show: bool) -> Self {
        self.horizontal_scrollbar = show;
        self
    }

    /// Full content extent supplied by the host.
    pub fn content_size(&self) -> Size {
        self.content
    }

    fn viewport_size(&self, area: Rect) -> Size {
        let mut show_vertical =
            self.vertical_scrollbar && self.content.height as usize > area.height as usize;
        let mut show_horizontal =
            self.horizontal_scrollbar && self.content.width as usize > area.width as usize;
        // Reserving one scrollbar can create overflow on the other axis.
        // Iterate to the tiny two-boolean fixed point.
        for _ in 0..2 {
            let width = area.width.saturating_sub(u16::from(show_vertical));
            let height = area.height.saturating_sub(u16::from(show_horizontal));
            show_vertical =
                self.vertical_scrollbar && self.content.height as usize > height as usize;
            show_horizontal =
                self.horizontal_scrollbar && self.content.width as usize > width as usize;
        }
        Size::new(
            area.width.saturating_sub(u16::from(show_vertical)),
            area.height.saturating_sub(u16::from(show_horizontal)),
        )
    }

    /// Return offsets clamped for the given assigned area.
    pub fn clamped_offsets(&self, area: Rect) -> (usize, usize) {
        let viewport = self.viewport_size(area);
        (
            self.offset.min(ScrollState::max_offset(
                self.content.height as usize,
                viewport.height as usize,
            )),
            self.x_offset.min(ScrollState::max_x_offset(
                self.content.width as usize,
                viewport.width as usize,
            )),
        )
    }
}

impl<V: View> View for Viewport<V> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(
            self.content.width.min(available.width),
            self.content.height.min(available.height),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() || self.content.width == 0 || self.content.height == 0 {
            return;
        }

        let viewport = self.viewport_size(area);
        let (offset, x_offset) = self.clamped_offsets(area);
        if viewport.width == 0 || viewport.height == 0 {
            return;
        }
        // Give the scratch buffer source-space coordinates and only allocate
        // visible cells. The child still receives its full content rect, while
        // Surface clipping discards everything outside this window.
        let source_area = Rect::new(
            x_offset as u16,
            offset as u16,
            viewport
                .width
                .min(self.content.width.saturating_sub(x_offset as u16)),
            viewport
                .height
                .min(self.content.height.saturating_sub(offset as u16)),
        );
        let mut scratch = Buffer::empty(source_area);
        {
            let content_area = Rect::new(0, 0, self.content.width, self.content.height);
            let mut content_surface = Surface::new(&mut scratch, source_area);
            self.child.render(content_area, &mut content_surface, ctx);
        }

        let destination =
            Rect::new(area.x, area.y, viewport.width, viewport.height).intersection(surface.area());
        let buffer = surface.buffer_mut();
        for y in destination.y..destination.bottom() {
            let source_y = offset + usize::from(y - area.y);
            if source_y >= self.content.height as usize {
                break;
            }
            for x in destination.x..destination.right() {
                let source_x = x_offset + usize::from(x - area.x);
                if source_x >= self.content.width as usize {
                    break;
                }
                let source = &scratch[(source_x as u16, source_y as u16)];
                let remaining = destination.right().saturating_sub(x);
                if str_cols(source.symbol()) > remaining {
                    buffer[(x, y)] = Cell::default();
                } else {
                    buffer[(x, y)] = source.clone();
                }
            }
        }

        let vertical_overflow = self.content.height as usize > viewport.height as usize;
        if vertical_overflow && self.vertical_scrollbar && viewport.width < area.width {
            Scrollbar::vertical(VirtualWindow::new(
                usize::from(self.content.height),
                usize::from(viewport.height),
                offset,
            ))
            .render(
                Rect::new(area.right() - 1, area.y, 1, viewport.height),
                surface,
                ctx,
            );
        }
        let horizontal_overflow = self.content.width as usize > viewport.width as usize;
        if horizontal_overflow && self.horizontal_scrollbar && viewport.height < area.height {
            Scrollbar::horizontal(VirtualWindow::new(
                usize::from(self.content.width),
                usize::from(viewport.width),
                x_offset,
            ))
            .render(
                Rect::new(area.x, area.y + viewport.height, viewport.width, 1),
                surface,
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::testing::{grid, render};
    use crate::text::Line;
    use crate::{Theme, element};

    #[test]
    fn viewport_clamps_offsets_and_crops_arbitrary_child() {
        let mut state = ScrollState::new();
        state.set_offset(99);
        state.set_x_offset(99);
        let child = Text::new(vec![
            Line::from("abcdef"),
            Line::from("ghijkl"),
            Line::from("mnopqr"),
        ]);
        let view = Viewport::new(element(child), Size::new(6, 3), &state).vertical_scrollbar(false);
        assert_eq!(view.clamped_offsets(Rect::new(0, 0, 3, 2)), (1, 3));
        assert_eq!(grid(&render(&view, 3, 2, &Theme::default())), "jkl\npqr");
    }

    #[test]
    fn viewport_does_not_split_wide_cell_at_right_edge() {
        let state = ScrollState::default();
        let view = Viewport::new(element(Text::raw("a界")), Size::new(3, 1), &state)
            .vertical_scrollbar(false);
        assert_eq!(grid(&render(&view, 2, 1, &Theme::default())), "a ");
    }

    #[test]
    fn viewport_handles_all_tiny_sizes() {
        let state = ScrollState::default();
        for width in 0..=2 {
            for height in 0..=2 {
                let view = Viewport::new(element(Text::raw("content")), Size::new(7, 1), &state)
                    .horizontal_scrollbar(true);
                let _ = render(&view, width, height, &Theme::default());
            }
        }
    }

    #[test]
    fn viewport_reclamps_after_resize() {
        let mut state = ScrollState::default();
        state.set_offset(8);
        state.set_x_offset(8);
        let view = Viewport::new(element(Text::raw("0123456789")), Size::new(10, 10), &state)
            .vertical_scrollbar(false);
        assert_eq!(view.clamped_offsets(Rect::new(0, 0, 2, 2)), (8, 8));
        assert_eq!(view.clamped_offsets(Rect::new(0, 0, 8, 8)), (2, 2));
    }

    #[test]
    fn one_scrollbar_can_trigger_the_other_axis() {
        let state = ScrollState::default();
        let view = Viewport::new(element(Text::raw("12345")), Size::new(5, 2), &state)
            .horizontal_scrollbar(true);
        // The bottom bar makes two content rows overflow vertically; the
        // resulting right bar then leaves a 3x1 content window.
        assert_eq!(view.viewport_size(Rect::new(0, 0, 4, 2)), Size::new(3, 1));
    }
}
