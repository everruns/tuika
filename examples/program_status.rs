//! OSC 7501 program-status reports from a tuika app. Run with
//! `cargo run --example program_status` (`y`/`n` answer the prompt, `r`
//! restarts, `q` or `Esc` quits).
//!
//! A scripted build job walks through every state the Program Status Protocol
//! has: `working` with a rising `progress=`, `blocked` with `kind=question`
//! while it waits for an answer, then `done` (or `error` if the answer is no),
//! and a `clear` on exit. Each report goes to the terminal through
//! [`tuika::term::program_status::write`]; the lower panel prints the exact
//! bytes of every report sent, so what the terminal receives is on screen.
//!
//! In a terminal that reads the protocol (tuios, Rex) the pane's status follows
//! along; anywhere else the sequences are swallowed and only the UI changes.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind};
use tuika::term::program_status::{self, BlockKind, Report, State};
use tuika::term::terminal::{Terminal, TerminalOptions, Viewport};
use tuika::ui::{Line, Span};

use tuika::prelude::*;

mod support;

/// The application name every report carries.
const APP: &str = "build";
/// Reports kept in the on-screen log.
const LOG_LEN: usize = 6;

/// The steps of the scripted job and the progress each one ends at.
const STEPS: [(&str, u8); 4] = [
    ("Fetching crates", 20),
    ("Compiling", 60),
    ("Running tests", 85),
    ("Packaging", 100),
];
/// Progress at which the job stops to ask before packaging.
const ASK_AT: u8 = 85;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Asking,
    Done,
    Failed,
}

struct Job {
    phase: Phase,
    progress: u8,
    answered: bool,
    /// The last report sent, so one goes out per change rather than per frame.
    last: Option<Report>,
    log: VecDeque<String>,
}

impl Job {
    fn new() -> Self {
        Job {
            phase: Phase::Running,
            progress: 0,
            answered: false,
            last: None,
            log: VecDeque::new(),
        }
    }

    fn step(&self) -> &'static str {
        STEPS
            .iter()
            .find(|(_, end)| self.progress < *end)
            .map_or("Packaging", |(name, _)| name)
    }

    fn tick(&mut self) {
        if self.phase != Phase::Running {
            return;
        }
        if self.progress == ASK_AT && !self.answered {
            self.phase = Phase::Asking;
        } else if self.progress >= 100 {
            self.phase = Phase::Done;
        } else {
            self.progress += 1;
        }
    }

    fn answer(&mut self, yes: bool) {
        if self.phase != Phase::Asking {
            return;
        }
        self.answered = true;
        self.phase = if yes { Phase::Running } else { Phase::Failed };
    }

    /// What the job looks like as a program-status report right now.
    ///
    /// Progress is reported in steps of 5 so a report goes out on a visible
    /// change, not on every frame the bar moves.
    fn report(&self) -> Report {
        let reported = self.progress - self.progress % 5;
        match self.phase {
            Phase::Running => Report::new(State::Working)
                .app(APP)
                .progress(reported)
                .msg(self.step()),
            Phase::Asking => Report::new(State::Blocked)
                .kind(BlockKind::Question)
                .app(APP)
                .progress(reported)
                .msg("Tests passed. Publish the build?"),
            Phase::Done => Report::new(State::Done).app(APP).msg("Built 12 crates"),
            Phase::Failed => Report::new(State::Error).app(APP).msg("Publish declined"),
        }
    }

    /// Send the current report if it changed since the last one.
    fn sync(&mut self) -> io::Result<()> {
        let report = self.report();
        if self.last.as_ref() == Some(&report) {
            return Ok(());
        }
        self.send(&report)?;
        self.last = Some(report);
        Ok(())
    }

    fn send(&mut self, report: &Report) -> io::Result<()> {
        let mut out = io::stdout();
        program_status::write(&mut out, report)?;
        out.flush()?;
        if let Some(bytes) = report.encode() {
            // Keep only the payload; the frame is drawn around it as `\e]` / `\e\`.
            let shown = bytes
                .trim_start_matches("\x1b]")
                .trim_end_matches("\x1b\\")
                .to_string();
            if self.log.len() == LOG_LEN {
                self.log.pop_front();
            }
            self.log.push_back(shown);
        }
        Ok(())
    }
}

fn build(job: &Job, frame: u64, theme: &Theme) -> tuika::Element {
    let fraction = f32::from(job.progress) / 100.0;
    let (status, status_style) = match job.phase {
        Phase::Running => (format!("{}…", job.step()), theme.accent_style()),
        Phase::Asking => (
            "Tests passed. Publish the build?  y / n".to_string(),
            theme.warning_style(),
        ),
        Phase::Done => ("Built 12 crates".to_string(), theme.success_style()),
        Phase::Failed => ("Publish declined".to_string(), theme.danger_style()),
    };
    let steps: Vec<Line> = STEPS
        .iter()
        .enumerate()
        .map(|(i, (name, end))| {
            let start = if i == 0 { 0 } else { STEPS[i - 1].1 };
            let (mark, style) = if job.progress >= *end {
                ("✓", theme.success_style())
            } else if job.progress >= start && job.phase != Phase::Failed {
                ("›", theme.accent_style())
            } else {
                ("·", theme.muted_style())
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), style),
                Span::raw(name.to_string()),
            ])
        })
        .collect();
    let log: Vec<Line> = job
        .log
        .iter()
        .map(|entry| {
            Line::from(vec![
                Span::styled("\\e]", theme.muted_style()),
                Span::raw(entry.clone()),
                Span::styled("\\e\\", theme.muted_style()),
            ])
        })
        .collect();
    let spinning = job.phase == Phase::Running;

    view! {
        col(padding = tuika::Padding::all(1), gap = 1,
            background = tuika::ui::Style::default().bg(theme.background)) {
            fixed(10) {
                boxed(title = Line::from(Span::styled(" build job ", theme.accent_style()))) {
                    col(gap = 1) {
                        fixed(4) { node(Text::new(steps)) }
                        fixed(1) { node(ProgressBar::determinate(fraction).percent(true)) }
                        fixed(1) {
                            row(gap = 1) {
                                fixed(1) {
                                    node(if spinning {
                                        element(Spinner::new(frame))
                                    } else {
                                        element(Text::new(vec![Line::from(" ")]))
                                    })
                                }
                                node(Text::new(vec![Line::from(Span::styled(status, status_style))]))
                            }
                        }
                    }
                }
            }
            grow(1) {
                boxed(title = Line::from(Span::styled(" OSC 7501 sent to the terminal ", theme.accent_style()))) {
                    node(Wrap::new(log))
                }
            }
            fixed(1) {
                node(Text::new(vec![Line::from(Span::styled(
                    "y / n answer   r restart   q quit",
                    theme.muted_style(),
                ))]))
            }
        }
    }
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let theme = cli.theme;
    let _session = tuika::TerminalSession::enter_with(ScreenMode::Alternate)?;
    let mut terminal = Terminal::with_options(
        tuika::term::backend::CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;

    let mut job = Job::new();
    let mut frame = 0u64;
    loop {
        job.sync()?;
        terminal.draw(|f| {
            let area = f.area();
            let root = build(&job, frame, &theme);
            tuika::paint(f.buffer_mut(), area, &theme, root.as_ref(), &[]);
        })?;
        if event::poll(Duration::from_millis(80))?
            && let CtEvent::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('y') => job.answer(true),
                KeyCode::Char('n') => job.answer(false),
                KeyCode::Char('r') => {
                    let log = std::mem::take(&mut job.log);
                    job = Job::new();
                    job.log = log;
                }
                _ => {}
            }
        }
        job.tick();
        frame = frame.wrapping_add(1);
    }

    // Remove the record, so the terminal stops showing this pane as a job.
    let _ = program_status::write(&mut io::stdout(), &Report::clear(None));
    let _ = io::stdout().flush();
    let _ = terminal.clear();
    drop(terminal);
    Ok(())
}
