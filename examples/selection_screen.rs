//! AGF-shaped action picker before and after `SelectionScreen`.
//!
//! The marked caller expressions are counted as nonblank source lines:
//! 8 before, 4 after. The new form also borrows `rows`; the old form clones
//! them because `SelectList` is an owned standalone view.

use std::io;

use tuika::prelude::*;

fn hints() -> KeyHints {
    KeyHints::new([("↑/↓", "move"), ("enter", "select"), ("esc", "cancel")])
}

fn before(rows: &[Line<'static>], state: &SelectState) -> impl View {
    // BEFORE CALLER START
    let header = Text::new(vec![Line::from("Select an action")]);
    let body = SelectList::new(rows.to_vec(), state);
    AppShell::new(body)
        .top_rule()
        .header(header)
        .top_rule()
        .bottom_rule()
        .footer(hints())
    // BEFORE CALLER END
}

fn after<'rows>(rows: &'rows [Line<'static>], state: &SelectState) -> SelectionScreen<'rows> {
    // AFTER CALLER START
    SelectionScreen::borrowed("Select an action", rows, state)
        .leading_rule()
        .trailing_rule()
        .footer(hints())
    // AFTER CALLER END
}

fn main() -> io::Result<()> {
    let rows = [
        Line::from("Run command"),
        Line::from("Delegate to agent"),
        Line::from("Request permission"),
        Line::from("Resume session"),
    ];
    let mut state = SelectState::new();
    state.select(Some(2));
    let _before_still_compiles = before(&rows, &state);
    write_once(
        &mut io::stdout(),
        &after(&rows, &state),
        &Theme::default(),
        OneShotOptions {
            width: 54,
            max_height: 9,
            ..OneShotOptions::default()
        },
    )
}
