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
    let screen_mode = if capture_mouse {
        ScreenMode::Alternate.with_mouse_capture()
    } else {
        ScreenMode::Alternate
    };
    let runner = Runner::new(RunnerConfig {
        screen_mode,
        ..RunnerConfig::default()
    });

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
