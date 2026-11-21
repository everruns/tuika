//! Runner-owned out-of-band state — OSC 7501 program status, its child task
//! records, and the window title: the glue between the values a host sets and
//! the drivers that report them for one run.

use crate::term::title::WindowTitle;
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
    title: Live<Option<String>>,
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
            tasks: Live::with_redraw(Vec::new(), redraw.clone()),
            title: Live::with_redraw(None, redraw),
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

    pub(crate) fn title(&self) -> Live<Option<String>> {
        self.title.clone()
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
            drivers: Mutex::new(Drivers {
                status: StatusReporter::new()
                    .with_native_progress(self.native_progress)
                    .with_support(self.support),
                tasks: TaskReporter::new().with_support(self.support),
                title: WindowTitle::new(),
            }),
            stdout: owns_stdout,
        }
    }
}

struct Drivers {
    status: StatusReporter,
    tasks: TaskReporter,
    title: WindowTitle,
}

/// One run's drivers. Dropping it applies the exit rules — clear live status
/// records, restore the window title — so every way out of a run (`Exit`, an
/// error, a panic unwinding) leaves the terminal as the user expects.
pub(crate) struct StatusSession<'a> {
    status: &'a RunnerStatus,
    // A mutex rather than a cell so an async run's future stays `Send`.
    drivers: Mutex<Drivers>,
    stdout: bool,
}

impl StatusSession<'_> {
    fn with_output(&self, write: impl FnOnce(&mut Drivers, &mut dyn Write)) {
        let mut drivers = self
            .drivers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(output) = &self.status.output {
            let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
            write(&mut drivers, &mut **output);
        } else if self.stdout {
            write(&mut drivers, &mut io::stdout());
        }
    }

    /// Report the current values if they changed. Best-effort: a failed
    /// write must never end a session.
    pub(crate) fn sync(&self) {
        let current = self.status.value.with(Clone::clone);
        let children = self.status.tasks.with(|tasks| task_reports(None, tasks));
        let title = self.status.title.with(Clone::clone);
        self.with_output(|drivers, mut out| {
            let _ = drivers.status.update(&mut out, current.as_ref());
            let _ = drivers.tasks.update(&mut out, &children);
            let _ = drivers.title.set(&mut out, title.as_deref());
        });
    }
}

impl Drop for StatusSession<'_> {
    fn drop(&mut self) {
        // An application that sets `done` and exits in the same update never
        // paints again, so report the final values before the exit rules.
        self.sync();
        self.with_output(|drivers, mut out| {
            let _ = drivers.tasks.finish(&mut out);
            let _ = drivers.status.finish(&mut out);
            let _ = drivers.title.finish(&mut out);
        });
    }
}
