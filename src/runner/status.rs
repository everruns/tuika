//! Runner-owned OSC 7501 program status: the glue between the host's one
//! status value and a [`StatusReporter`] that lives for one run.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use crate::live::{Live, RedrawHandle};
use crate::term::program_status::{Report, StatusReporter};

/// Where a run's status reports go when the host named a destination.
type StatusOutput = Arc<Mutex<Box<dyn Write + Send>>>;

/// The status configuration a runner carries between runs.
pub(crate) struct RunnerStatus {
    value: Live<Option<Report>>,
    native_progress: bool,
    output: Option<StatusOutput>,
}

impl RunnerStatus {
    pub(crate) fn new(redraw: RedrawHandle) -> Self {
        Self {
            // Setting the value wakes the loop, which is what gets a change
            // from a background thread reported without waiting for a tick.
            value: Live::with_redraw(None, redraw),
            native_progress: false,
            output: None,
        }
    }

    pub(crate) fn value(&self) -> Live<Option<Report>> {
        self.value.clone()
    }

    pub(crate) fn set_native_progress(&mut self, enabled: bool) {
        self.native_progress = enabled;
    }

    pub(crate) fn set_output(&mut self, output: impl Write + Send + 'static) {
        self.output = Some(Arc::new(Mutex::new(Box::new(output))));
    }

    /// Start one run's reporting. `owns_stdout` is true for the entry points
    /// that enter a terminal session, which report to stdout unless the host
    /// named an output; `run_driven_by` reports only to a named output.
    pub(crate) fn session(&self, owns_stdout: bool) -> StatusSession<'_> {
        StatusSession {
            status: self,
            reporter: Mutex::new(StatusReporter::new().with_native_progress(self.native_progress)),
            stdout: owns_stdout,
        }
    }
}

/// One run's reporter. Dropping it applies the exit rule, so every way out
/// of a run — `Exit`, an error, a panic unwinding — leaves the terminal's
/// record in the state the spec asks for.
pub(crate) struct StatusSession<'a> {
    status: &'a RunnerStatus,
    // A mutex rather than a cell so an async run's future stays `Send`.
    reporter: Mutex<StatusReporter>,
    stdout: bool,
}

impl StatusSession<'_> {
    fn with_output(&self, write: impl FnOnce(&mut StatusReporter, &mut dyn Write)) {
        let mut reporter = self
            .reporter
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(output) = &self.status.output {
            let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
            write(&mut reporter, &mut **output);
        } else if self.stdout {
            write(&mut reporter, &mut io::stdout());
        }
    }

    /// Report the current value if it changed. Best-effort: a failed status
    /// write must never end a session.
    pub(crate) fn sync(&self) {
        let current = self.status.value.with(Clone::clone);
        self.with_output(|reporter, out| {
            let _ = reporter.update(&mut { out }, current.as_ref());
        });
    }
}

impl Drop for StatusSession<'_> {
    fn drop(&mut self) {
        // An application that sets `done` and exits in the same update never
        // paints again, so report the final value before the exit rule.
        self.sync();
        self.with_output(|reporter, out| {
            let _ = reporter.finish(&mut { out });
        });
    }
}
