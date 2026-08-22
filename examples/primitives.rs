//! Owned scenes, arbitrary-child viewports, forms, and custom drawing.

use ratatui_core::layout::Rect;
use tuika::prelude::*;
use tuika::view::DrawView;

mod support;

fn main() -> std::io::Result<()> {
    let cli = support::Cli::parse()?;
    if !cli.args.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run --example primitives [-- --theme NAME]",
        ));
    }
    let theme = cli.theme;
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

    write_once(
        &mut std::io::stdout(),
        &scene,
        &theme,
        OneShotOptions {
            width: 48,
            max_height: 16,
            ..OneShotOptions::default()
        },
    )
}
