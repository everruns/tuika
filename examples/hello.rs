//! Small complete tuika application used by the getting-started guide.
//!
//! Run with `cargo run --example hello`; press `q` or `Esc` to quit.

use std::io;

use tuika::prelude::*;

mod support;

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let theme = cli.theme;
    let capture_mouse = cli.args.iter().any(|argument| argument == "--mouse");
    let runner = Runner::new(RunnerConfig {
        screen_mode: ScreenMode::Alternate,
        ..RunnerConfig::default()
    });
    let runner = if capture_mouse {
        runner.with_mouse_input(MouseInput::Captured)
    } else {
        runner
    };

    runner.run(
        &theme,
        from_fn(
            &mut (),
            |_state, _frame| {
                view! {
                    col(padding = Padding::all(1)) {
                        fixed(3) {
                            boxed(title = " tuika ") {
                                text("Hello from the terminal")
                            }
                        }
                        grow(1) { spacer() }
                        fixed(1) { text("q or esc to quit") }
                    }
                }
            },
            |_state, signal| match signal {
                Signal::Event(Event::Key(key))
                    if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
                {
                    UpdateResult::Exit
                }
                _ => UpdateResult::Clean,
            },
        ),
    )
}
