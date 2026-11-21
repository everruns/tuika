//! Runner-owned OSC 7501 program status: the glue between the host's one
//! status value and a [`StatusReporter`] that lives for one run.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use crate::live::{Live, RedrawHandle};
use crate::term::program_status::{
    Report, StatusReporter, StatusSupport, StatusTask, TaskReporter, task_reports,
};

/// Where a run's status reports go when the host named a destination.
type StatusOutput = Arc<Mutex<Box<dyn Write + Send>>>;

/// The status configuration a runner carries between runs.
pub(crate) struct RunnerStatus {
    value: Live<Option<Report>>,
    tasks: Live<Vec<StatusTask>>,
    native_progress: bool,
    support: StatusSupport,
    output: Option<StatusOutput>,
}

impl RunnerStatus {
    pub(crate) fn new(redraw: RedrawHandle) -> Self {
        Self {
            // Setting the value wakes the loop, which is what gets a change
            // from a background thread reported without waiting for a tick.
            value: Live::with_redraw(None, redraw.clone()),
            tasks: Live::with_redraw(Vec::new(), redraw),
            native_progress: false,
            support: StatusSupport::ALL,
            output: None,
        }
    }

    pub(crate) fn value(&self) -> Live<Option<Report>> {
        self.value.clone()
    }

    pub(crate) fn tasks(&self) -> Live<Vec<StatusTask>> {
        self.tasks.clone()
    }

    pub(crate) fn set_native_progress(&mut self, enabled: bool) {
        self.native_progress = enabled;
    }

    pub(crate) fn set_support(&mut self, support: StatusSupport) {
        self.support = support;
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
            reporter: Mutex::new(
                StatusReporter::new()
                    .with_native_progress(self.native_progress)
                    .with_support(self.support),
            ),
            tasks: Mutex::new(TaskReporter::new().with_support(self.support)),
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
    tasks: Mutex<TaskReporter>,
    stdout: bool,
}

impl StatusSession<'_> {
    fn with_output(
        &self,
        write: impl FnOnce(&mut StatusReporter, &mut TaskReporter, &mut dyn Write),
    ) {
        let mut reporter = self
            .reporter
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut tasks = self.tasks.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(output) = &self.status.output {
            let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
            write(&mut reporter, &mut tasks, &mut **output);
        } else if self.stdout {
            write(&mut reporter, &mut tasks, &mut io::stdout());
        }
    }

    /// Report the current values if they changed. Best-effort: a failed
    /// status write must never end a session.
    pub(crate) fn sync(&self) {
        let current = self.status.value.with(Clone::clone);
        let children = self.status.tasks.with(|tasks| task_reports(None, tasks));
        self.with_output(|reporter, tasks, mut out| {
            let _ = reporter.update(&mut out, current.as_ref());
            let _ = tasks.update(&mut out, &children);
        });
    }
}

impl Drop for StatusSession<'_> {
    fn drop(&mut self) {
        // An application that sets `done` and exits in the same update never
        // paints again, so report the final values before the exit rule.
        self.sync();
        self.with_output(|reporter, tasks, mut out| {
            let _ = tasks.finish(&mut out);
            let _ = reporter.finish(&mut out);
        });
    }
}
