use std::cell::RefCell;
use std::rc::Rc;
use tuika::prelude::*;
use tuika::testing::render_with_sheet;

struct HostView;

impl View for HostView {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let padding = ctx
            .sheet
            .panel
            .padding
            .expect("host stylesheet reaches third-party measurement");
        Size::new(
            1u16.saturating_add(padding.horizontal()),
            1u16.saturating_add(padding.vertical()),
        )
        .clamp_to(available)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        surface.set(area.x, area.y, 'x', ctx.sheet.heading.to_style());
    }
}

#[test]
fn third_party_measurement_receives_the_context_through_containers() {
    let theme = Theme::default();
    let sheet = StyleSheet {
        panel: StyleBundle::new().padding(Padding::all(2)),
        heading: StyleBundle::new().fg(Color::Cyan),
        ..StyleSheet::from_theme(&theme)
    };
    let tree = Flex::column().auto(element(Boxed::new(element(HostView))));
    let ctx = RenderCtx::new(&theme).with_sheet(sheet);

    // HostView asks for 5x5 from the active padding; Boxed applies the same
    // stylesheet padding plus its border, and Flex forwards that result.
    assert_eq!(tree.measure(Size::new(20, 20), &ctx), Size::new(11, 11));

    let buffer = render_with_sheet(&tree, 11, 11, &theme, sheet);
    assert_eq!(buffer[(3, 3)].symbol(), "x");
    assert_eq!(buffer[(3, 3)].fg, Color::Cyan);
}

#[derive(Clone)]
struct RequestProbe(Rc<RefCell<Vec<MeasureRequest>>>);

impl View for RequestProbe {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        available
    }

    fn measure_request(&self, request: MeasureRequest, _ctx: &RenderCtx) -> Size {
        self.0.borrow_mut().push(request);
        Size::new(
            request.known_width.unwrap_or(7),
            request.known_height.unwrap_or(3),
        )
    }

    fn render(&self, _area: Rect, _surface: &mut Surface, _ctx: &RenderCtx) {}
}

#[test]
fn flex_and_grid_forward_resolved_axes_to_third_party_views() {
    let theme = Theme::default();
    let ctx = RenderCtx::new(&theme);

    let flex_requests = Rc::new(RefCell::new(Vec::new()));
    let flex = Flex::row().fixed(5, element(RequestProbe(Rc::clone(&flex_requests))));
    let _ = flex.measure(Size::new(12, 4), &ctx);
    assert!(
        flex_requests
            .borrow()
            .iter()
            .any(|request| request.known_width == Some(5))
    );

    let grid_requests = Rc::new(RefCell::new(Vec::new()));
    let grid = Grid::new(2)
        .column_gap(2)
        .cell(element(RequestProbe(Rc::clone(&grid_requests))));
    let _ = grid.measure(Size::new(12, 4), &ctx);
    assert!(
        grid_requests
            .borrow()
            .iter()
            .any(|request| request.known_width == Some(5))
    );
}

#[test]
fn flex_and_grid_preserve_intrinsic_measurement_modes() {
    let theme = Theme::default();
    let ctx = RenderCtx::new(&theme);
    let intrinsic = MeasureRequest::new(Size::new(20, 8))
        .with_available_width(AvailableSpace::MaxContent)
        .with_available_height(AvailableSpace::MinContent);

    let flex_requests = Rc::new(RefCell::new(Vec::new()));
    let flex = Flex::row().auto(element(RequestProbe(Rc::clone(&flex_requests))));
    assert_eq!(flex.measure_request(intrinsic, &ctx), Size::new(7, 3));
    assert!(flex_requests.borrow().iter().any(|request| {
        request.available_width == AvailableSpace::MaxContent
            && request.available_height == AvailableSpace::MinContent
    }));

    let grid_requests = Rc::new(RefCell::new(Vec::new()));
    let grid = Grid::new(2).cell(element(RequestProbe(Rc::clone(&grid_requests))));
    assert_eq!(grid.measure_request(intrinsic, &ctx), Size::new(14, 3));
    assert!(grid_requests.borrow().iter().any(|request| {
        request.available_width == AvailableSpace::MaxContent
            && request.available_height == AvailableSpace::MinContent
    }));
}
