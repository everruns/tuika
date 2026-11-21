//! A two-screen app on `ScreenStack`: an inbox that opens a message, and back.
//!
//! Run with:
//!
//! ```text
//! cargo run --example screens
//! ```
//!
//! On the inbox, Tab moves focus between the filter and the list, Up/Down pick
//! a message, and Enter opens it. On a message, Esc goes back: the inbox comes
//! back with the same message highlighted and focus on the control you left.
//! `q` quits from anywhere, and so does Esc on the inbox.

use std::io;

use tuika::prelude::*;

mod support;

const MESSAGES: [&str; 4] = [
    "Release checklist",
    "CI is red on main",
    "Demo recordings",
    "Changelog review",
];

/// Each screen carries its own state, so a covered screen keeps it.
enum Screen {
    Inbox { list: SelectState },
    Message(usize),
}

struct App {
    screens: ScreenStack<Screen>,
    focus: FocusRegistry,
}

impl App {
    fn new() -> Self {
        Self {
            screens: ScreenStack::new(Screen::Inbox {
                list: SelectState::new(),
            }),
            focus: FocusRegistry::new(),
        }
    }

    /// Register this frame's focusable regions, as a render pass would.
    fn register_focus(&mut self) {
        self.focus.begin_frame();
        match self.screens.current() {
            Screen::Inbox { .. } => {
                self.focus.register("filter");
                self.focus.register("list");
            }
            Screen::Message(_) => self.focus.register("reply"),
        }
    }

    fn view(&self) -> Element {
        let marker = |id: &str| {
            if self.focus.is_focused(id) {
                "▶"
            } else {
                " "
            }
        };
        let trail = self
            .screens
            .iter()
            .map(|screen| match screen {
                Screen::Inbox { .. } => "Inbox".to_string(),
                Screen::Message(index) => MESSAGES[*index].to_string(),
            })
            .collect::<Vec<_>>()
            .join(" › ");
        let body = match self.screens.current() {
            Screen::Inbox { list } => {
                let rows = MESSAGES.iter().map(|m| Line::from(*m)).collect();
                view! {
                    col {
                        fixed(1) { node(Text::raw(format!("{} filter: (all)", marker("filter")))) }
                        fixed(1) { node(Text::raw(format!("{} messages", marker("list")))) }
                        grow(1) { node(SelectList::new(rows, list)) }
                        fixed(1) { node(Text::raw("tab focus · enter open · q/esc quit")) }
                    }
                }
            }
            Screen::Message(index) => view! {
                col {
                    fixed(1) { node(Text::raw(format!("Subject: {}", MESSAGES[*index]))) }
                    grow(1) { node(Text::raw("(message body)")) }
                    fixed(1) { node(Text::raw(format!("{} reply", marker("reply")))) }
                    fixed(1) { node(Text::raw("esc back · q quit")) }
                }
            },
        };
        view! {
            col {
                fixed(1) { node(Text::raw(trail)) }
                grow(1) { node(body) }
            }
        }
    }

    fn update(&mut self, event: &Event) -> UpdateResult {
        let Event::Key(key) = event else {
            return UpdateResult::Clean;
        };
        if key.plain() && key.code == KeyCode::Char('q') {
            return UpdateResult::Exit;
        }
        if self.focus.handle(event) == EventFlow::Consumed {
            return UpdateResult::Dirty;
        }
        // Esc is "back"; with only the root open there is nowhere to go, so
        // it quits, like every other example.
        if key.plain() && key.code == KeyCode::Esc {
            return match self.screens.pop(&mut self.focus) {
                Some(_) => UpdateResult::Dirty,
                None => UpdateResult::Exit,
            };
        }
        let list_focused = self.focus.is_focused("list");
        match self.screens.current_mut() {
            Screen::Inbox { list } if list_focused => match list.handle(event, MESSAGES.len()) {
                InputOutcome::Submitted => {
                    let index = list.selected().unwrap_or(0);
                    self.screens.push(Screen::Message(index), &self.focus);
                    UpdateResult::Dirty
                }
                InputOutcome::Changed => UpdateResult::Dirty,
                _ => UpdateResult::Clean,
            },
            _ => UpdateResult::Clean,
        }
    }
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let mut app = App::new();
    app.register_focus();
    let runner = Runner::new(RunnerConfig::default());
    // Focus registration happens once per update so the example stays one file;
    // a larger app registers while rendering, through `FocusScope`.
    runner.run(
        &cli.theme,
        from_fn(
            &mut app,
            |app, _| app.view(),
            |app, signal| {
                let result = match signal {
                    Signal::Event(event) => app.update(&event),
                    Signal::Tick => UpdateResult::Clean,
                };
                app.register_focus();
                result
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(Key::new(code))
    }

    fn press(app: &mut App, code: KeyCode) -> UpdateResult {
        let result = app.update(&key(code));
        app.register_focus();
        result
    }

    #[test]
    fn going_back_restores_the_inbox_selection_and_focus() {
        let mut app = App::new();
        app.register_focus();
        assert!(app.focus.is_focused("filter"));
        press(&mut app, KeyCode::Tab);
        assert!(app.focus.is_focused("list"));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Enter), UpdateResult::Dirty);
        assert!(matches!(app.screens.current(), Screen::Message(2)));
        assert!(app.focus.is_focused("reply"));

        press(&mut app, KeyCode::Esc);
        let Screen::Inbox { list } = app.screens.current() else {
            panic!("back on the inbox");
        };
        assert_eq!(list.selected(), Some(2));
        assert!(app.focus.is_focused("list"), "focus returns to the list");
        assert_eq!(press(&mut app, KeyCode::Esc), UpdateResult::Exit);
    }
}
