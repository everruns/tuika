//! OSC 7501 program-status reports for the scripted agent.
//!
//! A terminal that reads the protocol (such as tuios) can then show the pane as
//! working, waiting on an approval, or finished, without scraping the screen.
//! Terminals that do not read it swallow the sequence. tuika only encodes;
//! deciding which state the app is in is the host's job, done here.

use std::io::{self, Write};

use tuika::term::program_status::{BlockKind, Report, State, write};

use crate::agent::Agent;

/// The app name every report carries.
const APP: &str = "codex";

/// What the last report said, so one is sent per change rather than per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Idle,
    Working,
    Blocked(String),
    Done,
}

pub struct ProgramStatus {
    last: Option<Phase>,
}

impl ProgramStatus {
    pub fn new() -> Self {
        ProgramStatus { last: None }
    }

    /// Report the agent's phase if it changed since the last call. A turn that
    /// was running and now is not is reported done, which a terminal keeps
    /// until the user types in the pane.
    pub fn sync(&mut self, agent: &Agent) -> io::Result<()> {
        let phase = if let Some(command) = agent.pending_approval() {
            Phase::Blocked(command.to_string())
        } else if agent.is_running() {
            Phase::Working
        } else if matches!(
            self.last,
            Some(Phase::Working | Phase::Blocked(_) | Phase::Done)
        ) {
            Phase::Done
        } else {
            Phase::Idle
        };
        if self.last.as_ref() == Some(&phase) {
            return Ok(());
        }
        let report = match &phase {
            Phase::Idle => Report::new(State::Idle).app(APP),
            Phase::Working => Report::new(State::Working).app(APP).msg("Working"),
            Phase::Blocked(command) => Report::new(State::Blocked)
                .kind(BlockKind::Permission)
                .app(APP)
                .msg(&format!("Allow command? {command}")),
            Phase::Done => Report::new(State::Done).app(APP).msg("Turn complete"),
        };
        self.last = Some(phase);
        let mut out = io::stdout();
        write(&mut out, &report)?;
        out.flush()
    }

    /// Remove the record on exit, so the pane stops showing an agent.
    pub fn clear(&mut self) {
        let mut out = io::stdout();
        let _ = write(&mut out, &Report::clear(None));
        let _ = out.flush();
    }
}
