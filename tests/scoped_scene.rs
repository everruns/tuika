//! Consumer-style coverage for painting a borrowed host root and owned dialog
//! as one view.

use tuika::prelude::*;
use tuika::testing::grid;
use tuika::ui::{Buffer, Rect};

struct Transcript<'data> {
    messages: &'data [String],
}

impl View for Transcript<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(
            available.width,
            (self.messages.len() as u16).min(available.height),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        for (row, message) in self.messages.iter().take(area.height as usize).enumerate() {
            surface.set_string(area.x, area.y + row as u16, message, ctx.theme.text_style());
        }
    }
}

#[test]
fn borrowed_transcript_and_owned_dialog_paint_as_one_root() {
    let messages = vec!["first message".to_owned(), "second message".to_owned()];
    let transcript = Transcript {
        messages: &messages,
    };
    let scene = ScopedScene::new(&transcript).dialog(
        Dialog::new("Confirm", element(Text::raw("Delete?")))
            .size(20, 5)
            .dim_backdrop(true)
            .focus_owner("confirm"),
    );
    let area = Rect::new(0, 0, 30, 8);
    let mut buffer = Buffer::empty(area);
    let mut focus = FocusRegistry::new();
    focus.register("transcript");

    scene.sync_focus(&mut focus);
    paint(&mut buffer, area, &Theme::default(), &scene, &[]);

    let painted = grid(&buffer);
    assert!(painted.contains("Confirm"));
    assert!(painted.contains("Delete?"));
    assert_eq!(focus.active(), Some("confirm"));
    assert_eq!(messages[1], "second message");
}

#[test]
fn borrowed_view_composes_through_macro_flex_and_boxed() {
    let source = String::from("# nested borrow");
    let highlighter = PlainHighlighter;
    let root: ScopedElement<'_> = view! {
        boxed(title = " transcript ") {
            col {
                node(Markdown::new(&source).highlighter(&highlighter))
            }
        }
    };
    let scene = ScopedScene::new(root.as_ref());
    let rendered = tuika::testing::render(&scene, 24, 3, &Theme::default());

    assert!(grid(&rendered).contains("nested borrow"));
}
