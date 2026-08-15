//! Stable, expandable tree navigation over host-provided rows.
//!
//! Run with `cargo run --example tree_list`. Use arrows and Enter to navigate,
//! click labels or disclosure markers, press `r` to reorder refreshed roots,
//! and resize the terminal to see the persistent viewport reconcile.

use std::io;

use tuika::prelude::*;

const TREE_Y: u16 = 2;

struct TreeApp {
    rows: Vec<TreeRow<'static, u64>>,
    tree: TreeState<u64>,
    window: VirtualWindow,
    viewport_rows: usize,
    reordered: bool,
}

impl TreeApp {
    fn new() -> Self {
        let rows = vec![
            TreeRow::root(1, "workspace", true),
            TreeRow::new(2, Some(1), 1, "src", true),
            TreeRow::new(3, Some(2), 2, "components", true),
            TreeRow::new(4, Some(3), 3, "tree_list.rs", false),
            TreeRow::new(5, Some(2), 2, "lib.rs", false),
            TreeRow::new(6, Some(1), 1, "tests", true),
            TreeRow::new(7, Some(6), 2, "integration.rs", false),
            TreeRow::root(8, "target", true),
            TreeRow::new(9, Some(8), 1, "debug", false),
        ];
        let mut tree = TreeState::with_selected(1);
        tree.expand(1);
        tree.expand(2);
        tree.expand(3);
        let viewport_rows = 8;
        let window = tree.resolve(&rows, viewport_rows);
        Self {
            rows,
            tree,
            window,
            viewport_rows,
            reordered: false,
        }
    }

    fn reconcile(&mut self) {
        self.window = self.tree.resolve(&self.rows, self.viewport_rows);
    }

    fn refresh(&mut self) {
        if self.reordered {
            self.rows.rotate_left(2);
        } else {
            self.rows.rotate_right(2);
        }
        self.reordered = !self.reordered;
        self.reconcile();
    }
}

struct TreeScreen<'a> {
    app: &'a TreeApp,
}

impl View for TreeScreen<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        available
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        Text::raw("TreeList · stable ids, expansion, persistent scrolling").render(
            Rect::new(area.x, area.y, area.width, 1),
            surface,
            ctx,
        );
        Text::raw("↑↓ navigate · ← parent/collapse · → expand · enter toggle · r refresh · q quit")
            .render(
                Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
                surface,
                ctx,
            );
        TreeList::new(&self.app.rows, &self.app.tree)
            .visible_window(self.app.window)
            .render(
                Rect::new(
                    area.x,
                    area.y.saturating_add(TREE_Y),
                    area.width,
                    area.height.saturating_sub(TREE_Y),
                ),
                surface,
                ctx,
            );
    }
}

impl Application for TreeApp {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        let Signal::Event(event) = signal else {
            return UpdateResult::Clean;
        };
        if let Event::Resize { height, .. } = event {
            self.viewport_rows = usize::from(height.saturating_sub(TREE_Y));
            self.reconcile();
            return UpdateResult::Dirty;
        }
        if let Event::Key(key) = &event
            && key.plain()
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return UpdateResult::Exit,
                KeyCode::Char('r') => {
                    self.refresh();
                    return UpdateResult::Dirty;
                }
                _ => {}
            }
        }
        let outcome = match event {
            Event::Mouse(Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                ..
            }) => self.tree.handle_mouse(
                &event,
                &self.rows,
                Rect::new(0, TREE_Y, u16::MAX, self.viewport_rows as u16),
                self.window,
            ),
            _ => self.tree.handle(&event, &self.rows, self.viewport_rows),
        };
        self.reconcile();
        match outcome {
            InputOutcome::Changed | InputOutcome::Submitted => UpdateResult::Dirty,
            InputOutcome::Ignored | InputOutcome::Consumed | InputOutcome::Cancelled => {
                UpdateResult::Clean
            }
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        element(TreeScreen { app: self })
    }
}

fn main() -> io::Result<()> {
    Runner::new(RunnerConfig::default()).run(&Theme::default(), &mut TreeApp::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_keeps_selected_id_and_mouse_uses_current_window() {
        let mut app = TreeApp::new();
        app.tree.select(Some(4));
        app.reconcile();
        app.refresh();
        assert_eq!(app.tree.selected(), Some(&4));

        let row = app
            .window
            .range()
            .position(|position| position == 0)
            .unwrap_or(0) as u16;
        let event = Event::Mouse(Mouse::at(
            MouseKind::Down(MouseButton::Left),
            8,
            TREE_Y + row,
        ));
        assert_eq!(app.update(Signal::Event(event)), UpdateResult::Dirty);
    }
}
