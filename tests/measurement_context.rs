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
