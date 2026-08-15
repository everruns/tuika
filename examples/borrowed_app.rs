//! Data-driven application whose custom view borrows application state.
//!
//! Run with `cargo run --example borrowed_app`. Press any key to increment the
//! counter; press `q` or Escape to quit.

use std::io;

use tuika::prelude::*;

struct CounterApp {
    title: String,
    count: u64,
}

struct CounterView<'app> {
    title: &'app str,
    count: u64,
}

impl View for CounterView<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(available.width, 2.min(available.height))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        surface.set_string(area.x, area.y, self.title, ctx.theme.text_style());
        if area.height > 1 {
            let count = format!("count: {}", self.count);
            surface.set_string(area.x, area.y + 1, &count, ctx.theme.accent_style());
        }
    }
}

impl Application for CounterApp {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        match signal {
            Signal::Event(Event::Key(key))
                if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
            {
                UpdateResult::Exit
            }
            Signal::Event(Event::Key(_)) => {
                self.count = self.count.saturating_add(1);
                UpdateResult::Dirty
            }
            _ => UpdateResult::Clean,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        element(CounterView {
            title: &self.title,
            count: self.count,
        })
    }
}

fn main() -> io::Result<()> {
    let mut app = CounterApp {
        title: "Borrowed application state".into(),
        count: 0,
    };
    Runner::new(RunnerConfig::default()).run(&Theme::default(), &mut app)
}
