//! Wrapping content flow built on flex lines.

use crate::geometry::Rect;

use crate::geometry::{Padding, Size};
use crate::layout::{Align, AlignContent, FlexItemStyle, FlexWrap};
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, ScopedElement, View};

use super::Flex;

/// A row-oriented flex container that wraps items onto additional lines.
///
/// `Flow` is the compact choice for tags, chips, actions, and other content
/// whose measured widths determine line breaks. Use [`Flex`] directly when the
/// direction or individual flex policies need to vary.
///
/// ![flow demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/flow.png)
pub struct Flow<V: View = Element> {
    flex: Flex<V>,
}

impl<V: View> Flow<V> {
    /// Set both horizontal and vertical gaps.
    pub fn gap(mut self, gap: u16) -> Self {
        self.flex = self.flex.gap(gap);
        self
    }

    /// Set the gap between wrapped rows.
    pub fn row_gap(mut self, gap: u16) -> Self {
        self.flex = self.flex.row_gap(gap);
        self
    }

    /// Set the gap between items within a row.
    pub fn column_gap(mut self, gap: u16) -> Self {
        self.flex = self.flex.column_gap(gap);
        self
    }

    /// Set inner padding.
    pub fn padding(mut self, padding: Padding) -> Self {
        self.flex = self.flex.padding(padding);
        self
    }

    /// Set item alignment within each row.
    pub fn align(mut self, align: Align) -> Self {
        self.flex = self.flex.align(align);
        self
    }

    /// Set distribution of wrapped rows.
    pub fn align_content(mut self, align: AlignContent) -> Self {
        self.flex = self.flex.align_content(align);
        self
    }

    /// Add an intrinsically sized item.
    pub fn item(mut self, view: V) -> Self {
        self.flex = self.flex.auto(view);
        self
    }

    /// Add a fixed-width item.
    pub fn fixed(mut self, width: u16, view: V) -> Self {
        self.flex = self.flex.fixed(width, view);
        self
    }

    /// Add an item with a complete child-owned flex policy.
    pub fn styled(mut self, style: FlexItemStyle, view: V) -> Self {
        self.flex = self.flex.styled(style, view);
        self
    }

    /// Resolve the rectangles assigned to flow items.
    pub fn solve(&self, area: Rect, ctx: &RenderCtx) -> Vec<Rect> {
        self.flex.solve(area, ctx)
    }
}

impl Flow<Element> {
    /// Create an empty owned wrapping flow.
    pub fn new() -> Self {
        Self {
            flex: Flex::row()
                .wrap(FlexWrap::Wrap)
                .align(Align::Start)
                .align_content(AlignContent::Start),
        }
    }

    /// Create an empty flow whose items may borrow frame state.
    pub fn scoped<'view>() -> Flow<ScopedElement<'view>> {
        Flow {
            flex: Flex::scoped_row()
                .wrap(FlexWrap::Wrap)
                .align(Align::Start)
                .align_content(AlignContent::Start),
        }
    }
}

impl Default for Flow<Element> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: View> View for Flow<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.flex.measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.flex.render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;

    #[test]
    fn flow_wraps_intrinsic_items() {
        let flow = Flow::new()
            .column_gap(1)
            .row_gap(1)
            .item(crate::element(Text::raw("one")))
            .item(crate::element(Text::raw("two")))
            .item(crate::element(Text::raw("three")));
        let theme = crate::Theme::default();
        let rects = flow.solve(Rect::new(0, 0, 7, 5), &RenderCtx::new(&theme));
        assert_eq!(rects[0], Rect::new(0, 0, 3, 1));
        assert_eq!(rects[1], Rect::new(4, 0, 3, 1));
        assert_eq!(rects[2], Rect::new(0, 2, 5, 1));
    }
}
