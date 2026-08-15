//! Interactive picker demonstrating Tuika's configurable selection policies.
//!
//! Run with:
//!
//! ```text
//! cargo run --example select
//! ```
//!
//! Navigate with arrows, j/k, Ctrl+N/P, or Tab/Shift+Tab. Number keys toggle
//! items directly; Enter/Space toggles the cursor row; left-click toggles a row.
//! Press `c` to clear checked items and `q`/Escape to quit.

use std::io;

use tuika::prelude::*;

const ITEMS: [&str; 7] = [
    "Build release",
    "Run test suite",
    "Generate docs",
    "Publish artifacts",
    "Notify maintainers",
    "Deploy canary",
    "Promote production",
];
const LIST_Y: u16 = 4;

#[derive(Default)]
struct PickerState {
    selection: MultiSelectState,
    message: String,
}

impl PickerState {
    fn new() -> Self {
        Self {
            selection: MultiSelectState::new(),
            message: "Choose any number of tasks".into(),
        }
    }
}

fn scene(state: &PickerState, _theme: &Theme) -> Element {
    let rows = ITEMS
        .iter()
        .enumerate()
        .map(|(index, item)| {
            Line::from(format!(
                "{} {}  {item}",
                index + 1,
                if state.selection.contains(index) {
                    "[x]"
                } else {
                    "[ ]"
                }
            ))
        })
        .collect();
    let checked = state.selection.selected().count();

    view! {
        col {
            fixed(1) { node(Text::raw("Interactive multi-select")) }
            fixed(1) { node(Text::raw("↑↓/jk  Ctrl+N/P  Tab/Shift+Tab  1-7  click  Enter/Space")) }
            fixed(1) { node(Text::raw(format!("{} · {checked} checked", state.message))) }
            fixed(1) { node(Text::raw("")) }
            fixed(7) { node(SelectList::new(rows, state.selection.cursor())) }
            fixed(1) { node(Text::raw("c clear · q/esc quit")) }
        }
    }
}

fn update(state: &mut PickerState, event: &Event) -> UpdateResult {
    if let Event::Key(key) = event
        && key.plain()
    {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return UpdateResult::Exit,
            KeyCode::Char('c') => {
                state.selection.clear();
                state.message = "Selection cleared".into();
                return UpdateResult::Dirty;
            }
            _ => {}
        }
    }

    let outcome = match event {
        Event::Mouse(_) => state.selection.handle_mouse(
            event,
            ITEMS.len(),
            Rect::new(0, LIST_Y, u16::MAX, ITEMS.len() as u16),
            0,
        ),
        _ => state
            .selection
            .handle(event, ITEMS.len(), SelectNavigation::common()),
    };
    match outcome {
        InputOutcome::Ignored | InputOutcome::Consumed => UpdateResult::Clean,
        InputOutcome::Changed => {
            state.message = match state.selection.cursor().selected() {
                Some(index) if state.selection.contains(index) => {
                    format!("Checked {}", ITEMS[index])
                }
                Some(index) => format!("Cursor on {}", ITEMS[index]),
                None => "No selection".into(),
            };
            UpdateResult::Dirty
        }
        InputOutcome::Submitted => UpdateResult::Dirty,
        InputOutcome::Cancelled => UpdateResult::Exit,
    }
}

fn main() -> io::Result<()> {
    let theme = Theme::default();
    let runner = Runner::new(RunnerConfig::default());
    let mut state = PickerState::new();
    runner.run(
        &theme,
        from_fn(
            &mut state,
            |state, _frame| scene(state, &theme),
            |state, signal| match signal {
                Signal::Event(event) => update(state, &event),
                Signal::Tick => UpdateResult::Clean,
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuika::event::{Mouse, MouseButton, MouseKind};

    #[test]
    fn aliases_numbers_mouse_and_clear_drive_the_example_state() {
        let mut state = PickerState::new();
        assert_eq!(
            update(&mut state, &Event::Key(Key::new(KeyCode::Char('j')))),
            UpdateResult::Dirty
        );
        assert_eq!(state.selection.cursor().selected(), Some(1));

        assert_eq!(
            update(&mut state, &Event::Key(Key::new(KeyCode::Char('3')))),
            UpdateResult::Dirty
        );
        assert!(state.selection.contains(2));

        let click = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 3, LIST_Y + 4));
        assert_eq!(update(&mut state, &click), UpdateResult::Dirty);
        assert!(state.selection.contains(4));

        assert_eq!(
            update(&mut state, &Event::Key(Key::new(KeyCode::Char('c')))),
            UpdateResult::Dirty
        );
        assert_eq!(state.selection.selected().count(), 0);
    }
}
