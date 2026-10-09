//! Action picker borrowing host-owned rows and selection state.

use std::io;

use tuika::prelude::*;

mod support;

fn hints() -> KeyHints {
    KeyHints::new([("↑/↓", "move"), ("enter", "select"), ("esc", "cancel")])
}

fn screen<'rows>(rows: &'rows [Line<'static>], state: &SelectState) -> SelectionScreen<'rows> {
    SelectionScreen::borrowed("Select an action", rows, state)
        .leading_rule()
        .trailing_rule()
        .footer(hints())
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let rows = [
        Line::from("Run command"),
        Line::from("Delegate to agent"),
        Line::from("Request permission"),
        Line::from("Resume session"),
    ];
    let mut state = SelectState::new();
    state.select(Some(2));
    write_once(
        &mut io::stdout(),
        &screen(&rows, &state),
        &cli.theme,
        OneShotOptions {
            width: 54,
            max_height: 9,
            ..OneShotOptions::default()
        },
    )
}
