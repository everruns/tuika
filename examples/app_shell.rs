//! Interactive `AppShell` example.
//!
//! Run with `cargo run --example app_shell`. Use `↑`/`↓` to move, `Enter` to
//! open the selected file, and `q`/`Esc` to quit. Resize the terminal to see
//! secondary chrome collapse before the main content and footer. Pass `--dump`
//! after Cargo's `--` to print a deterministic frame instead.

use std::io;

use tuika::prelude::*;

mod support;

const FILES: [&str; 6] = [
    "src/components/app_shell.rs",
    "src/components/flex.rs",
    "src/components/selection_screen.rs",
    "examples/app_shell.rs",
    "docs/components/layout.md",
    "README.md",
];

struct App {
    selection: SelectState,
    message: String,
    theme: Theme,
}

impl App {
    fn new(theme: Theme) -> Self {
        Self {
            selection: SelectState::new(),
            message: "Ready".into(),
            theme,
        }
    }

    fn selected(&self) -> &'static str {
        self.selection
            .selected()
            .and_then(|index| FILES.get(index))
            .copied()
            .unwrap_or("no file")
    }
}

impl Application for App {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        let Signal::Event(event) = signal else {
            return UpdateResult::Clean;
        };
        if let Event::Key(key) = &event
            && key.plain()
            && key.code == KeyCode::Char('q')
        {
            return UpdateResult::Exit;
        }

        match self.selection.handle(&event, FILES.len()) {
            InputOutcome::Changed => {
                self.message = format!("Selected {}", self.selected());
                UpdateResult::Dirty
            }
            InputOutcome::Submitted => {
                self.message = format!("Opened {}", self.selected());
                UpdateResult::Dirty
            }
            InputOutcome::Cancelled => UpdateResult::Exit,
            InputOutcome::Ignored | InputOutcome::Consumed => UpdateResult::Clean,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        let rows = FILES
            .iter()
            .map(|path| Line::from(Span::styled(*path, self.theme.text_style())))
            .collect();
        let body = Boxed::new(element(SelectList::new(rows, &self.selection)))
            .title(" workspace ")
            .padding(Padding::symmetric(1, 0));
        let header = Text::new(vec![Line::from(vec![
            Span::styled("▲ tuika explorer  ", self.theme.accent_style()),
            Span::styled(
                "one growing body · intrinsic chrome",
                self.theme.muted_style(),
            ),
        ])]);
        let status = StatusBar::new()
            .left(vec![
                Span::styled(" READY ", self.theme.selection_style()),
                Span::styled(format!("  {}", self.message), self.theme.text_style()),
            ])
            .right(vec![Span::styled(
                format!("{}  ", self.selected()),
                self.theme.muted_style(),
            )]);

        element(
            AppShell::new(body)
                .header(header)
                .top_rule()
                .status(status)
                .bottom_rule()
                .footer(KeyHints::new([
                    ("↑/↓", "move"),
                    ("enter", "open"),
                    ("q/esc", "quit"),
                ])),
        )
    }
}

fn dump_frame(app: &App) -> String {
    let root = app.view(0);
    let buffer = tuika::testing::render(root.as_ref(), 72, 12, &app.theme);
    tuika::testing::grid(&buffer)
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let dump = match cli.args.as_slice() {
        [] => false,
        [arg] if arg == "--dump" => true,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: cargo run --example app_shell [-- --theme NAME] [--dump]",
            ));
        }
    };
    let theme = cli.theme;
    let mut app = App::new(theme);
    if dump {
        println!("{}", dump_frame(&app));
        return Ok(());
    }
    Runner::new(RunnerConfig::default()).run(&theme, &mut app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_contains_every_shell_region() {
        let theme = Theme::default();
        let app = App::new(theme);
        let grid = dump_frame(&app);

        for landmark in [
            "tuika explorer",
            "workspace",
            "app_shell.rs",
            "READY",
            "enter",
            "open",
        ] {
            assert!(grid.contains(landmark), "missing {landmark:?}\n{grid}");
        }
    }

    #[test]
    fn navigation_submission_and_exit_update_the_app() {
        let mut app = App::new(Theme::default());

        assert_eq!(
            app.update(Signal::Event(Event::Key(Key::new(KeyCode::Down)))),
            UpdateResult::Dirty
        );
        assert_eq!(app.selection.selected(), Some(1));
        assert!(app.message.contains("flex.rs"));

        assert_eq!(
            app.update(Signal::Event(Event::Key(Key::new(KeyCode::Enter)))),
            UpdateResult::Dirty
        );
        assert!(app.message.starts_with("Opened"));

        assert_eq!(
            app.update(Signal::Event(Event::Key(Key::new(KeyCode::Esc)))),
            UpdateResult::Exit
        );
    }
}
