//! Owned scenes, arbitrary-child viewports, forms, and custom drawing.

use ratatui_core::layout::Rect;
use tuika::prelude::*;
use tuika::testing::{grid, render};
use tuika::view::DrawView;

fn main() {
    let theme = Theme::default();
    let scroll = ScrollState::default();
    let form_state = FormState::default();

    let canvas = DrawView::new(
        |area: Rect, surface: &mut tuika::Surface<'_>, ctx: &tuika::RenderCtx<'_>| {
            for y in area.y..area.bottom() {
                surface.set_string(area.x, y, "界 0123456789 abcdef", ctx.theme.info_style());
            }
        },
    )
    .intrinsic_size(Size::new(20, 4));

    let form = Form::new(
        vec![
            FormField::new("Name", element(Text::raw("Ada"))).help("Host-owned field value"),
            FormField::new(
                "Preview",
                element(
                    Viewport::new(element(canvas), Size::new(20, 4), &scroll)
                        .horizontal_scrollbar(true),
                ),
            )
            .error("Example validation row"),
        ],
        &form_state,
    );

    let scene = Scene::new(element(Text::raw("Base screen"))).dialog(
        Dialog::new("Reusable primitives", element(form))
            .size(44, 12)
            .key_hints([("enter", "submit"), ("esc", "cancel")])
            .dim_backdrop(true)
            .focus_owner("example-form"),
    );

    println!("{}", grid(&render(&scene, 48, 16, &theme)));
}
