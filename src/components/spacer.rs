//! Spacer — empty, flexible filler. Paints nothing; useful as a
//! `Dimension::Flex` child to push siblings apart, or fixed to insert a gap.

use crate::geometry::Rect;

use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

/// An empty filler view that paints nothing; used as a flex or fixed gap.
pub struct Spacer;

impl View for Spacer {
    fn measure(&self, _available: Size, _ctx: &RenderCtx) -> Size {
        Size::ZERO
    }

    fn render(&self, _area: Rect, _surface: &mut Surface, _ctx: &RenderCtx) {}
}
