//! Flex container — the composition primitive.
//!
//! Holds a [`LayoutStyle`] and a list of children, each tagged with a
//! [`Dimension`] for its main-axis sizing. Delegates rect assignment to the
//! [`solve`](crate::layout::solve) flexbox solver, then renders each
//! child into its rect through a clipped child surface. Nesting `Flex`es is how
//! every screen is built.

use crate::geometry::Rect;
use crate::style::Style;

use crate::geometry::Size;
use crate::layout::{AlignContent, Dimension, FlexItemStyle, FlexWrap, Item, LayoutStyle, solve};
use crate::surface::Surface;
use crate::view::{AvailableSpace, Element, MeasureRequest, RenderCtx, ScopedElement, View};

struct Child<V: View> {
    view: V,
    style: FlexItemStyle,
}

/// A flexbox container of child views.
///
/// # Example
///
/// ```
/// use tuika::prelude::*;
/// use tuika::testing::{grid, render};
///
/// // A row of two fixed-width cells, laid left to right.
/// let view = Flex::row()
///     .fixed(3, element(Text::raw("abc")))
///     .fixed(3, element(Text::raw("xyz")));
///
/// let buffer = render(&view, 6, 1, &Theme::default());
/// assert_eq!(grid(&buffer), "abcxyz");
/// ```
///
/// ![flex demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/flex.png)
pub struct Flex<V: View = Element> {
    style: LayoutStyle,
    children: Vec<Child<V>>,
    background: Option<Style>,
}

impl<V: View> Flex<V> {
    fn empty(style: LayoutStyle) -> Self {
        Self {
            style,
            children: Vec::new(),
            background: None,
        }
    }

    /// Fill the container area with `style` before drawing children.
    pub fn background(mut self, style: Style) -> Self {
        self.background = Some(style);
        self
    }

    /// Set the gap between children (passthrough to the layout style).
    pub fn gap(mut self, gap: u16) -> Self {
        self.style.row_gap = gap;
        self.style.column_gap = gap;
        self
    }

    /// Set the gap between flex rows.
    pub fn row_gap(mut self, gap: u16) -> Self {
        self.style.row_gap = gap;
        self
    }

    /// Set the gap between flex columns.
    pub fn column_gap(mut self, gap: u16) -> Self {
        self.style.column_gap = gap;
        self
    }

    /// Configure whether overflowing children form additional flex lines.
    pub fn wrap(mut self, wrap: FlexWrap) -> Self {
        self.style.wrap = wrap;
        self
    }

    /// Set cross-axis distribution of multiple flex lines.
    pub fn align_content(mut self, align: AlignContent) -> Self {
        self.style.align_content = align;
        self
    }

    /// Set padding inside the container (passthrough to the layout style).
    pub fn padding(mut self, padding: crate::geometry::Padding) -> Self {
        self.style.padding = padding;
        self
    }

    /// Set cross-axis alignment (passthrough to the layout style).
    pub fn align(mut self, align: crate::layout::Align) -> Self {
        self.style.align_items = align;
        self
    }

    /// Set main-axis distribution (passthrough to the layout style).
    pub fn justify(mut self, justify: crate::layout::Justify) -> Self {
        self.style.justify = justify;
        self
    }

    /// Add a child with an explicit main-axis dimension.
    pub fn child(mut self, dimension: Dimension, view: V) -> Self {
        self.children.push(Child {
            view,
            style: FlexItemStyle::from_dimension(dimension),
        });
        self
    }

    /// Add a child with independent basis, grow, shrink, and alignment.
    pub fn styled(mut self, style: FlexItemStyle, view: V) -> Self {
        self.children.push(Child { view, style });
        self
    }

    /// Add an auto-sized child (sizes to its measured content).
    pub fn auto(self, view: V) -> Self {
        self.child(Dimension::Auto, view)
    }

    /// Add a flexible child that grows to share leftover space.
    pub fn grow(self, weight: u16, view: V) -> Self {
        self.child(Dimension::Flex(weight), view)
    }

    /// Add a fixed-size child.
    pub fn fixed(self, cells: u16, view: V) -> Self {
        self.child(Dimension::Fixed(cells), view)
    }

    fn space_for_children(&self, inner_available: Size) -> u16 {
        let axis = self.style.direction.axis();
        let total_gap = self
            .style
            .main_gap()
            .saturating_mul(self.children.len().saturating_sub(1) as u16);
        axis.main(inner_available).saturating_sub(total_gap)
    }

    fn child_available(&self, inner_available: Size, style: FlexItemStyle) -> Size {
        let axis = self.style.direction.axis();
        let space_for_children = self.space_for_children(inner_available);
        let main = match style.basis {
            Dimension::Fixed(cells) => cells.min(space_for_children),
            Dimension::Percent(percent) => {
                ((space_for_children as u32 * percent.min(100) as u32) / 100) as u16
            }
            Dimension::Auto | Dimension::Flex(_) => axis.main(inner_available),
        };
        axis.size(main, axis.cross(inner_available))
    }

    fn child_request(
        &self,
        inner_available: Size,
        style: FlexItemStyle,
        mut request: MeasureRequest,
    ) -> MeasureRequest {
        let main = self
            .style
            .direction
            .axis()
            .main(self.child_available(inner_available, style));
        let resolves_main = matches!(style.basis, Dimension::Fixed(_) | Dimension::Percent(_));
        match (self.style.direction, resolves_main) {
            (crate::layout::Direction::Row, true) => {
                request.available_width = AvailableSpace::Definite(main);
                request.known_width = Some(main);
            }
            (crate::layout::Direction::Column, true) => {
                request.available_height = AvailableSpace::Definite(main);
                request.known_height = Some(main);
            }
            (_, false) => {}
        }
        request
    }

    fn items(&self, area: Rect, ctx: &RenderCtx) -> Vec<Item> {
        let inner_available = Size::from(self.style.padding.inner(area));
        let mut items: Vec<Item> = self
            .children
            .iter()
            .map(|child| {
                let available = self.child_available(inner_available, child.style);
                let request = self.child_request(
                    inner_available,
                    child.style,
                    MeasureRequest::new(available),
                );
                Item::styled(child.style, child.view.measure_request(request, ctx))
            })
            .collect();

        // Flex widths/heights are only known after auto/fixed/percent children
        // have consumed their space. Refine flex intrinsic cross sizes against
        // their actual main-axis allocation, then solve once more below.
        if !matches!(self.style.align_items, crate::layout::Align::Stretch) {
            let preliminary = solve(area, &self.style, &items);
            let axis = self.style.direction.axis();
            for ((child, item), rect) in self.children.iter().zip(items.iter_mut()).zip(preliminary)
            {
                let main = axis.main(Size::from(rect));
                let available = axis.size(main, axis.cross(inner_available));
                let request = match self.style.direction {
                    crate::layout::Direction::Row => {
                        MeasureRequest::new(available).with_known_width(main)
                    }
                    crate::layout::Direction::Column => {
                        MeasureRequest::new(available).with_known_height(main)
                    }
                };
                item.intrinsic = child.view.measure_request(request, ctx);
            }
        }

        items
    }

    /// Resolve the child rects this container would assign inside `area`,
    /// without painting anything.
    ///
    /// This is the same measure-then-[`solve`] pass `render` runs, exposed so a
    /// host can compute layout ahead of (or instead of) a render — to clamp a
    /// scroll offset to a pane's real height, hit-test a click against child
    /// rects, or decide what fits before drawing. The returned `Vec` has one
    /// rect per child, in insertion order.
    ///
    /// ```
    /// use tuika::prelude::*;
    /// use tuika::ui::Rect;
    ///
    /// let flex = Flex::row()
    ///     .fixed(4, element(Text::raw("abcd")))
    ///     .grow(1, element(Text::raw("rest")));
    /// let theme = Theme::default();
    /// let ctx = RenderCtx::new(&theme);
    /// let rects = flex.solve(Rect::new(0, 0, 10, 1), &ctx);
    /// assert_eq!(rects.len(), 2);
    /// assert_eq!(rects[0], Rect::new(0, 0, 4, 1));
    /// assert_eq!(rects[1], Rect::new(4, 0, 6, 1)); // grows into the leftover
    /// ```
    pub fn solve(&self, area: Rect, ctx: &RenderCtx) -> Vec<Rect> {
        solve(area, &self.style, &self.items(area, ctx))
    }
}

impl Flex<Element> {
    /// An empty owned flex container with the given layout style.
    pub fn new(style: LayoutStyle) -> Self {
        Self::empty(style)
    }

    /// An empty owned row, laying children left-to-right.
    pub fn row() -> Self {
        Self::new(LayoutStyle::row())
    }

    /// An empty owned column, stacking children top-to-bottom.
    pub fn column() -> Self {
        Self::new(LayoutStyle::column())
    }

    /// An empty frame-borrowed flex container with the given layout style.
    pub fn scoped<'view>(style: LayoutStyle) -> Flex<ScopedElement<'view>> {
        Flex::empty(style)
    }

    /// An empty frame-borrowed row.
    pub fn scoped_row<'view>() -> Flex<ScopedElement<'view>> {
        Self::scoped(LayoutStyle::row())
    }

    /// An empty frame-borrowed column.
    pub fn scoped_column<'view>() -> Flex<ScopedElement<'view>> {
        Self::scoped(LayoutStyle::column())
    }
}

impl<V: View> View for Flex<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        if self.style.wrap != FlexWrap::NoWrap {
            let area = Rect::new(0, 0, available.width, available.height);
            let rects = self.solve(area, ctx);
            let width = rects
                .iter()
                .map(|rect| rect.right())
                .max()
                .unwrap_or(self.style.padding.left)
                .saturating_add(self.style.padding.right);
            let height = rects
                .iter()
                .map(|rect| rect.bottom())
                .max()
                .unwrap_or(self.style.padding.top)
                .saturating_add(self.style.padding.bottom);
            return Size::new(width, height).clamp_to(available);
        }
        // Sum children on the main axis, max on the cross axis, plus gaps and
        // padding. Used when this Flex is itself an Auto child.
        let axis = self.style.direction.axis();
        let inner = self
            .style
            .padding
            .inner(Rect::new(0, 0, available.width, available.height));
        let inner_avail = Size::from(inner);
        let space_for_children = self.space_for_children(inner_avail);
        let mut main_total: u16 = 0;
        let mut cross_max: u16 = 0;
        for (i, c) in self.children.iter().enumerate() {
            let available = self.child_available(inner_avail, c.style);
            let request = self.child_request(inner_avail, c.style, MeasureRequest::new(available));
            let sz = c.view.measure_request(request, ctx);
            let intrinsic_main = axis.main(sz);
            let resolved_main = match c.style.basis {
                Dimension::Auto | Dimension::Flex(_) => intrinsic_main,
                Dimension::Fixed(cells) => cells,
                Dimension::Percent(percent) => {
                    ((space_for_children as u32 * percent.min(100) as u32) / 100) as u16
                }
            }
            .min(space_for_children);
            main_total = main_total.saturating_add(resolved_main);
            if i > 0 {
                main_total = main_total.saturating_add(self.style.main_gap());
            }
            cross_max = cross_max.max(axis.cross(sz));
        }
        let content = axis.size(main_total, cross_max);
        Size::new(
            content
                .width
                .saturating_add(self.style.padding.horizontal()),
            content.height.saturating_add(self.style.padding.vertical()),
        )
        .clamp_to(available)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        if matches!(request.available_width, AvailableSpace::Definite(_))
            && matches!(request.available_height, AvailableSpace::Definite(_))
        {
            return request.resolve(self.measure(request.fallback_available(), ctx));
        }

        let axis = self.style.direction.axis();
        let available = request.fallback_available();
        let inner = Size::from(self.style.padding.inner(Rect::new(
            0,
            0,
            available.width,
            available.height,
        )));
        let mut main = 0u16;
        let mut cross = 0u16;
        for (index, child) in self.children.iter().enumerate() {
            let measured = child
                .view
                .measure_request(self.child_request(inner, child.style, request), ctx);
            main = main.saturating_add(axis.main(measured));
            if index > 0 {
                main = main.saturating_add(self.style.main_gap());
            }
            cross = cross.max(axis.cross(measured));
        }
        let content = axis.size(main, cross);
        request.resolve(Size::new(
            content
                .width
                .saturating_add(self.style.padding.horizontal()),
            content.height.saturating_add(self.style.padding.vertical()),
        ))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if let Some(bg) = self.background {
            let mut fill = surface.child(area);
            fill.fill(bg);
        }
        let rects = self.solve(area, ctx);
        for (child, rect) in self.children.iter().zip(rects) {
            let mut child_surface = surface.child(rect);
            child.view.render(rect, &mut child_surface, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::geometry::Padding;
    use crate::probe::RectProbe;
    use crate::style::Theme;
    use crate::view::element;

    struct WidthSensitive;

    impl View for WidthSensitive {
        fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
            Size::new(available.width, if available.width < 10 { 2 } else { 1 })
        }

        fn render(&self, _area: Rect, _surface: &mut Surface, _ctx: &RenderCtx) {}
    }

    #[test]
    fn solve_matches_the_rects_render_paints_into() {
        // Probe every child, render, then compare the painted rects to what the
        // paint-free `solve` returns for the same area — they must be identical.
        let area = Rect::new(0, 0, 20, 6);
        let probes: Vec<RectProbe> = (0..3).map(|_| RectProbe::new()).collect();
        let flex = Flex::column()
            .fixed(1, probes[0].wrap(element(Text::raw("header"))))
            .grow(1, probes[1].wrap(element(Text::raw("body"))))
            .fixed(2, probes[2].wrap(element(Text::raw("footer"))));

        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let precomputed = flex.solve(area, &ctx);
        let _ = crate::testing::render(&flex, area.width, area.height, &theme);

        let painted: Vec<Rect> = probes.iter().map(|p| p.rect()).collect();
        assert_eq!(
            precomputed, painted,
            "solve() must return exactly the rects render() paints into"
        );
        // And they are the expected column layout: 1-row header, grown body, 2-row footer.
        assert_eq!(precomputed[0], Rect::new(0, 0, 20, 1));
        assert_eq!(precomputed[1], Rect::new(0, 1, 20, 3));
        assert_eq!(precomputed[2], Rect::new(0, 4, 20, 2));
    }

    #[test]
    fn solve_of_empty_container_is_empty() {
        let flex = Flex::row();
        let theme = Theme::default();
        assert!(
            flex.solve(Rect::new(0, 0, 10, 3), &RenderCtx::new(&theme))
                .is_empty()
        );
    }

    #[test]
    fn fixed_child_is_measured_at_its_declared_main_size() {
        let flex = Flex::row()
            .align(crate::layout::Align::Start)
            .fixed(5, element(WidthSensitive));

        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        assert_eq!(flex.measure(Size::new(10, 4), &ctx), Size::new(5, 2));
        assert_eq!(
            flex.solve(Rect::new(0, 0, 10, 4), &ctx)[0],
            Rect::new(0, 0, 5, 2)
        );
    }

    #[test]
    fn measure_resolves_declared_percent_main_size() {
        let flex = Flex::row().child(Dimension::Percent(50), element(Text::raw("x")));
        let theme = Theme::default();
        assert_eq!(
            flex.measure(Size::new(10, 2), &RenderCtx::new(&theme)),
            Size::new(5, 1)
        );
    }

    #[test]
    fn solve_measures_children_against_the_padded_inner_box() {
        let flex = Flex::column()
            .padding(Padding::symmetric(1, 0))
            .auto(element(WidthSensitive));

        let theme = Theme::default();
        assert_eq!(
            flex.solve(Rect::new(0, 0, 10, 4), &RenderCtx::new(&theme))[0].height,
            2
        );
    }

    #[test]
    fn non_stretch_flex_child_is_remeasured_at_its_allocated_main_size() {
        let flex = Flex::row()
            .align(crate::layout::Align::Start)
            .fixed(5, element(Text::raw("fixed")))
            .grow(1, element(WidthSensitive));

        let theme = Theme::default();
        assert_eq!(
            flex.solve(Rect::new(0, 0, 10, 4), &RenderCtx::new(&theme))[1],
            Rect::new(5, 0, 5, 2)
        );
    }

    #[test]
    fn crate_root_reexports_solver_primitives() {
        // `solve` and `Item` are reachable from the crate root, not just
        // `tuika::layout` — build a layout by hand without a Flex.
        use crate::Size;
        use crate::layout::{Dimension, Item, LayoutStyle, solve};
        let items = [
            Item::new(Dimension::Fixed(3), Size::new(3, 1)),
            Item::new(Dimension::Flex(1), Size::new(0, 1)),
        ];
        let rects = solve(Rect::new(0, 0, 10, 1), &LayoutStyle::row(), &items);
        assert_eq!(rects[0], Rect::new(0, 0, 3, 1));
        assert_eq!(rects[1], Rect::new(3, 0, 7, 1));
    }
}
