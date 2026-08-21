//! Width-driven view selection for responsive terminal layouts.

use crate::geometry::Rect;

use crate::{Element, RenderCtx, Size, Surface, View};

/// Select between compact and wide layouts at a column breakpoint.
///
/// Each branch is a normal view tree, so an application can switch from a row
/// to a column, omit secondary content, or choose a shorter status component
/// without embedding breakpoint logic in individual leaves.
pub struct Responsive<V: View = Element> {
    breakpoint: u16,
    compact: V,
    wide: V,
}

impl<V: View> Responsive<V> {
    /// Use `compact` below `breakpoint` columns and `wide` otherwise.
    pub fn new(breakpoint: u16, compact: V, wide: V) -> Self {
        Self {
            breakpoint,
            compact,
            wide,
        }
    }

    fn select(&self, width: u16) -> &V {
        if width < self.breakpoint {
            &self.compact
        } else {
            &self.wide
        }
    }
}

impl<V: View> View for Responsive<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.select(available.width).measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.select(area.width).render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::style::Theme;
    use crate::tests::support::row;
    use crate::view::element;

    #[test]
    fn responsive_selects_layout_from_render_width() {
        let view = Responsive::new(
            10,
            element(Text::raw("compact")),
            element(Text::raw("wide")),
        );
        let theme = Theme::default();
        let compact = crate::testing::render(&view, 8, 1, &theme);
        let wide = crate::testing::render(&view, 12, 1, &theme);
        assert_eq!(row(&compact, 0), "compact");
        assert_eq!(row(&wide, 0), "wide");
    }
}
