use tuika::prelude::*;
use tuika::testing::{TestHarness, grid};

struct BorrowedLabel<'app> {
    text: &'app str,
}

impl View for BorrowedLabel<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(available.width, available.height.min(1))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        surface.set_string(area.x, area.y, self.text, ctx.theme.text_style());
    }
}

struct App {
    label: String,
}

impl Application for App {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        match signal {
            Signal::Tick => {
                self.label.push('!');
                UpdateResult::Dirty
            }
            Signal::Event(Event::Resize { .. }) => UpdateResult::Clean,
            Signal::Event(_) => UpdateResult::Exit,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        element(BorrowedLabel { text: &self.label })
    }
}

#[test]
fn application_frames_borrow_state_and_resize_repaints() {
    let mut harness = TestHarness::new(
        App {
            label: "borrowed".into(),
        },
        10,
        1,
    );

    assert_eq!(grid(&harness.render_app()), "borrowed  ");
    let (result, frame) = harness.step_app(Signal::Tick);
    assert_eq!(result, UpdateResult::Dirty);
    assert_eq!(grid(&frame.unwrap()), "borrowed! ");

    let (result, frame) = harness.step_app(Signal::Event(Event::Resize {
        width: 12,
        height: 1,
    }));
    assert_eq!(result, UpdateResult::Clean);
    assert_eq!(grid(&frame.unwrap()), "borrowed!   ");
}
