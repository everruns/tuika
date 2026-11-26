//! OSC 7501 program-status reports from a tuika app. Run with
//! `cargo run --example program_status` (`y`/`n` answer the prompt, `r`
//! restarts, `q` or `Esc` quits).
//!
//! A scripted build job walks through every state the Program Status Protocol
//! has: `working` with a rising `progress=`, `blocked` with `kind=question`
//! while it waits for an answer, then `done` (or `error` if the answer is no).
//! The app only sets one value, [`Runner::program_status`]; the runner sends a
//! report when it changes and applies the exit rule — quitting mid-job clears
//! the record, quitting after `done` or `error` leaves it for the user to find.
//! Each step is also reported as a child record through
//! [`Runner::task_status`], and the window title follows the job through
//! [`Runner::window_title`] (the terminal's own title comes back on exit). The lower panel prints the exact bytes of every
//! root report, so what the terminal receives is on screen.
//!
//! In a terminal that reads the protocol (such as tuios) the pane's status follows
//! along; anywhere else the sequences are swallowed and only the UI changes.

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use tuika::live::Live;
use tuika::term::program_status::{self, BlockKind, Report, State, StatusSupport, StatusTask};
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
    /// The last report logged, so the log gains a line per change.
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

    /// Hand the current report to the runner, and log it if it changed. The
    /// runner does its own change detection; the log only mirrors it.
    /// Each step as a child record (`id=Compiling` and so on), so a terminal
    /// that groups records shows the job's steps under it.
    fn tasks(&self) -> Vec<StatusTask> {
        let mut start = 0;
        STEPS
            .iter()
            .map(|(name, end)| {
                let state = if self.progress >= *end {
                    State::Done
                } else if self.progress >= start && self.phase == Phase::Failed {
                    State::Error
                } else if self.progress >= start {
                    State::Working
                } else {
                    State::Idle
                };
                start = *end;
                StatusTask::new(name, Report::new(state).title(name))
            })
            .collect()
    }

    /// The window title for the job's current state.
    fn title(&self) -> String {
        match self.phase {
            Phase::Running => format!("build: {} {}%", self.step(), self.progress),
            Phase::Asking => "build: waiting for you".to_string(),
            Phase::Done => "build: done".to_string(),
            Phase::Failed => "build: declined".to_string(),
        }
    }

    fn sync(
        &mut self,
        status: &Live<Option<Report>>,
        tasks: &Live<Vec<StatusTask>>,
        title: &Live<Option<String>>,
    ) {
        tasks.set(self.tasks());
        title.set(Some(self.title()));
        let report = self.report();
        if self.last.as_ref() == Some(&report) {
            return;
        }
        status.set(Some(report.clone()));
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
        self.last = Some(report);
    }
}

struct App {
    job: Job,
    theme: Theme,
    status: Live<Option<Report>>,
    tasks: Live<Vec<StatusTask>>,
    title: Live<Option<String>>,
    /// What the startup probe found, shown in the footer.
    detected: String,
}

impl Application for App {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        match signal {
            Signal::Tick => self.job.tick(),
            Signal::Event(Event::Key(key)) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return UpdateResult::Exit,
                KeyCode::Char('y') => self.job.answer(true),
                KeyCode::Char('n') => self.job.answer(false),
                KeyCode::Char('r') => {
                    let log = std::mem::take(&mut self.job.log);
                    self.job = Job::new();
                    self.job.log = log;
                }
                _ => return UpdateResult::Clean,
            },
            _ => return UpdateResult::Clean,
        }
        self.job.sync(&self.status, &self.tasks, &self.title);
        UpdateResult::Dirty
    }

    fn view(&self, frame: u64) -> tuika::ScopedElement<'_> {
        build(&self.job, frame, &self.theme, &self.detected)
    }
}

fn build(job: &Job, frame: u64, theme: &Theme, detected: &str) -> tuika::Element {
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
                    format!("y / n answer   r restart   q quit   · {detected}"),
                    theme.muted_style(),
                ))]))
            }
        }
    }
}

/// Ask the terminal what it accepts. The probe reads its reply from stdin,
/// so it runs in raw mode and before the runner's event loop starts reading.
fn probe() -> (Option<StatusSupport>, String) {
    if crossterm::terminal::enable_raw_mode().is_err() {
        return (None, "OSC 7501 not probed".to_string());
    }
    let support = program_status::query(Duration::from_millis(200));
    let _ = crossterm::terminal::disable_raw_mode();
    let detected = match support {
        None => "terminal does not report OSC 7501".to_string(),
        Some(support) => {
            let states: Vec<&str> = [
                (State::Idle, "idle"),
                (State::Working, "working"),
                (State::Done, "done"),
                (State::Blocked, "blocked"),
                (State::Error, "error"),
            ]
            .into_iter()
            .filter(|(state, _)| support.supports(*state))
            .map(|(_, name)| name)
            .collect();
            format!("terminal accepts {}", states.join(","))
        }
    };
    let rex = if tuika::term::capabilities::Capabilities::from_env().rex {
        " · in Rex"
    } else {
        ""
    };
    (support, format!("{detected}{rex}"))
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let (support, detected) = probe();
    let mut runner = Runner::new(RunnerConfig {
        tick_rate: Duration::from_millis(80),
        ..RunnerConfig::default()
    });
    if let Some(support) = support {
        // A state the terminal lacks goes out as its nearest supported one.
        runner = runner.with_status_support(support);
    }
    let mut app = App {
        job: Job::new(),
        theme: cli.theme,
        status: runner.program_status(),
        tasks: runner.task_status(),
        title: runner.window_title(),
        detected,
    };
    app.job.sync(&app.status, &app.tasks, &app.title);
    runner.run(&cli.theme, &mut app)
}
