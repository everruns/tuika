//! OSC 7577 program resume from a tuika app. Run with
//! `cargo run --example program_resume` (`↑`/`↓` or `j`/`k` pick a chapter,
//! `q` or `Esc` quits).
//!
//! A small reader keeps one piece of state, the open chapter, and tells the
//! terminal how to get back to it: every time the chapter changes it reports
//! `argv = [<this binary>, --chapter, N]` plus the working directory through
//! [`tuika::term::program_resume::Reporter`]. A terminal that restores its
//! panes after a restart (Rex, for one) relaunches exactly that command, and
//! the reader opens on the same chapter — started with `--chapter N`, it says
//! it was resumed.
//!
//! Quitting with `q` is the user ending the program, so it sends `clear` and
//! the terminal forgets the command. Closing the terminal, `SIGHUP`, or a
//! crash sends nothing, which is the point: that is when resuming is wanted.
//!
//! Reports are signed with the token the terminal puts in `OSC7577_TOKEN`.
//! Without it (any terminal that does not offer resume) the reader runs the
//! same and the record panel says nothing is being sent. To watch the bytes in
//! any terminal, give it a token yourself:
//!
//! ```sh
//! OSC7577_TOKEN=local-demo-token-0123456789 cargo run --example program_resume
//! ```

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind};
use tuika::term::program_resume::{Action, Report, Reporter};
use tuika::term::terminal::{Terminal, TerminalOptions, Viewport};
use tuika::ui::{Line, Span};

use tuika::prelude::*;

mod support;

/// The reader's chapters: a title and one paragraph each.
const CHAPTERS: [(&str, &str); 6] = [
    (
        "Tides",
        "Twice a day the sea leaves the flats and comes back. The plants that live here keep time by it.",
    ),
    (
        "Salt marsh",
        "Cordgrass holds the mud together. Between the tussocks, crabs dig burrows that breathe with the tide.",
    ),
    (
        "Dunes",
        "Marram grass grows faster when it is buried, so every storm that drops sand on it builds the dune higher.",
    ),
    (
        "Rock pools",
        "A pool is an island twice a day: anemones close, limpets clamp down, and the water slowly warms.",
    ),
    (
        "Estuary",
        "Fresh water rides out over the salt. Where they meet, silt drops, and the channel wanders year by year.",
    ),
    (
        "Shingle",
        "Pebbles sort themselves by size along the beach, the largest thrown highest by the winter waves.",
    ),
];

struct App {
    chapter: usize,
    /// The chapter the program was resumed at, when started with `--chapter`.
    resumed_at: Option<usize>,
    /// `Some` only when the terminal offered a token.
    resume: Option<Reporter>,
    /// Extra arguments the resume command must carry to reproduce this run.
    theme_args: Vec<String>,
    exe: Option<PathBuf>,
    cwd: Option<PathBuf>,
    /// The last sequence sent, for the record panel.
    last_sent: Option<String>,
    /// The last thing that went wrong, for the record panel.
    problem: Option<String>,
}

impl App {
    /// The command that reopens this exact view: the binary by absolute path
    /// (a bare `program_resume` would not be on `PATH`), the chapter, and the
    /// theme, since the spec carries no environment and nothing else would
    /// bring it back.
    fn resume_argv(&self) -> Option<Vec<String>> {
        let exe = self.exe.as_ref()?.to_str()?.to_string();
        let mut argv = vec![exe, "--chapter".into(), (self.chapter + 1).to_string()];
        argv.extend(self.theme_args.iter().cloned());
        Some(argv)
    }

    fn report(&self) -> Option<Report> {
        let mut report = Report::resume(self.resume_argv()?);
        if let Some(cwd) = &self.cwd {
            report = report.cwd(cwd);
        }
        Some(report)
    }

    /// Tell the terminal about the current chapter, if it changed.
    fn sync(&mut self) {
        let Some(report) = self.report() else {
            self.problem = Some("binary path is not UTF-8; not reporting".into());
            return;
        };
        let Some(reporter) = &mut self.resume else {
            return;
        };
        // Sent through a buffer so the panel can show the real bytes.
        let mut sent = Vec::new();
        let result = reporter.update(&mut sent, &report);
        if !sent.is_empty() {
            let mut out = io::stdout();
            let _ = out.write_all(&sent).and_then(|()| out.flush());
            self.last_sent = Some(shown(&sent));
        }
        if let Err(err) = result {
            self.problem = Some(err.to_string());
        }
    }
}

/// The sequence as the panel prints it: escapes spelled out, the MAC cut to
/// its first digits. The MAC is not secret, but 64 hex digits push
/// everything useful off the line.
fn shown(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = match text.rfind(":mac=") {
        Some(at) => format!(
            "{}:mac={}…",
            &text[..at],
            &text[at + 5..(at + 13).min(text.len())]
        ),
        None => text.into_owned(),
    };
    text.replace('\x1b', "\\e")
}

/// Quote one argument for display, the way a shell would need it.
fn quoted(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

fn build(app: &App, theme: &Theme) -> tuika::Element {
    let chapters: Vec<Line> = CHAPTERS
        .iter()
        .enumerate()
        .map(|(i, (title, _))| {
            if i == app.chapter {
                Line::from(Span::styled(
                    format!("▸ {}. {title}", i + 1),
                    theme.accent_style(),
                ))
            } else {
                Line::from(Span::styled(
                    format!("  {}. {title}", i + 1),
                    theme.muted_style(),
                ))
            }
        })
        .collect();
    let (title, text) = CHAPTERS[app.chapter];

    let started = match app.resumed_at {
        Some(n) => Line::from(vec![
            Span::styled("Resumed ", theme.success_style()),
            Span::raw(format!("at chapter {} from the command line", n + 1)),
        ]),
        None => Line::from(Span::styled("Started fresh", theme.muted_style())),
    };
    let mut record = vec![started];
    match &app.resume {
        None => record.push(Line::from(Span::styled(
            "No OSC7577_TOKEN: this terminal does not offer resume, so nothing is sent.",
            theme.warning_style(),
        ))),
        Some(reporter) => {
            record.push(Line::from(vec![
                Span::styled("Token ", theme.muted_style()),
                Span::raw("present (never shown); reports are signed with it"),
            ]));
            match reporter.current().map(Report::action) {
                Some(Action::Resume { argv, cwd }) => {
                    let cmd: Vec<String> = argv.iter().map(|a| quoted(a)).collect();
                    record.push(Line::from(vec![
                        Span::styled("Resume ", theme.muted_style()),
                        Span::styled(cmd.join(" "), theme.accent_style()),
                    ]));
                    if let Some(cwd) = cwd {
                        record.push(Line::from(vec![
                            Span::styled("In     ", theme.muted_style()),
                            Span::raw(cwd.display().to_string()),
                        ]));
                    }
                }
                _ => record.push(Line::from(Span::styled(
                    "No command recorded",
                    theme.muted_style(),
                ))),
            }
        }
    }
    if let Some(sent) = &app.last_sent {
        record.push(Line::from(Span::styled(
            "Last report sent",
            theme.muted_style(),
        )));
        record.push(Line::from(sent.clone()));
    }
    if let Some(problem) = &app.problem {
        record.push(Line::from(Span::styled(
            problem.clone(),
            theme.danger_style(),
        )));
    }

    view! {
        col(padding = tuika::Padding::all(1), gap = 1,
            background = tuika::ui::Style::default().bg(theme.background)) {
            fixed(8) {
                row(gap = 1) {
                    fixed(20) {
                        boxed(title = Line::from(Span::styled(" field guide ", theme.accent_style()))) {
                            node(Text::new(chapters))
                        }
                    }
                    grow(1) {
                        boxed(title = Line::from(Span::styled(format!(" {title} "), theme.accent_style()))) {
                            node(Wrap::new(vec![Line::from(text)]))
                        }
                    }
                }
            }
            grow(1) {
                boxed(title = Line::from(Span::styled(" OSC 7577 resume record ", theme.accent_style()))) {
                    node(Wrap::new(record))
                }
            }
            fixed(1) {
                node(Text::new(vec![Line::from(Span::styled(
                    "↑/↓ chapter   q quit, clearing the record   (a crash keeps it)",
                    theme.muted_style(),
                ))]))
            }
        }
    }
}

/// `--chapter N` (1-based), the one argument the resume command adds.
fn parse_chapter(args: &[String]) -> io::Result<Option<usize>> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = if arg == "--chapter" {
            iter.next().map(String::as_str)
        } else if let Some(v) = arg.strip_prefix("--chapter=") {
            Some(v)
        } else {
            continue;
        };
        let n: usize = value
            .and_then(|v| v.parse().ok())
            .filter(|n| (1..=CHAPTERS.len()).contains(n))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("--chapter takes 1..={}", CHAPTERS.len()),
                )
            })?;
        return Ok(Some(n - 1));
    }
    Ok(None)
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let theme = cli.theme;
    let resumed_at = parse_chapter(&cli.args)?;
    let theme_args = cli
        .theme_name
        .map(|name| vec!["--theme".to_string(), name])
        .unwrap_or_default();

    let mut app = App {
        chapter: resumed_at.unwrap_or(0),
        resumed_at,
        // No token, no reporter: the program must not report at all.
        resume: Reporter::from_env(),
        theme_args,
        exe: std::env::current_exe().ok(),
        cwd: std::env::current_dir().ok(),
        last_sent: None,
        problem: None,
    };

    let _session = tuika::TerminalSession::enter_with(ScreenMode::Alternate)?;
    let mut terminal = Terminal::with_options(
        tuika::term::backend::CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;

    loop {
        app.sync();
        terminal.draw(|f| {
            let area = f.area();
            let root = build(&app, &theme);
            tuika::paint(f.buffer_mut(), area, &theme, root.as_ref(), &[]);
        })?;
        if event::poll(Duration::from_millis(100))?
            && let CtEvent::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Down | KeyCode::Char('j') => {
                    app.chapter = (app.chapter + 1).min(CHAPTERS.len() - 1);
                }
                KeyCode::Up | KeyCode::Char('k') => app.chapter = app.chapter.saturating_sub(1),
                _ => {}
            }
        }
    }

    // The user quit on purpose: nothing to resume, so drop the record.
    if let Some(reporter) = &mut app.resume {
        let _ = reporter.clear(&mut io::stdout());
    }
    let _ = io::stdout().flush();
    let _ = terminal.clear();
    drop(terminal);
    Ok(())
}
