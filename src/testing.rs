//! Public helpers for hermetic view and resize tests.

use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::term::program_status::{Report, State};

use crate::{
    Application, Element, Event, RenderCtx, Signal, StyleSheet, Theme, UpdateResult, View, paint,
    paint_with_context, paint_with_sheet,
};

/// A deterministic, terminal-free application harness.
///
/// The harness owns application state, viewport size, and frame numbering. It
/// drives the same `Signal`/`UpdateResult` contract as [`Runner`](crate::Runner)
/// while rendering into an in-memory [`Buffer`]. No raw mode, terminal global,
/// sleeping, or OS event queue is involved.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::testing::{TestHarness, grid};
///
/// let mut app = TestHarness::new(0u8, 3, 1);
/// let (result, frame) = app.step(
///     Signal::Tick,
///     |count, _| { *count += 1; UpdateResult::Dirty },
///     |count, _frame| element(Text::raw(count.to_string())),
/// );
/// assert_eq!(result, UpdateResult::Dirty);
/// assert_eq!(grid(&frame.unwrap()), "1  ");
/// ```
pub struct TestHarness<S> {
    state: S,
    theme: Theme,
    width: u16,
    height: u16,
    frame: u64,
}

impl<S> TestHarness<S> {
    /// Create a harness with the default theme and a fixed viewport.
    pub fn new(state: S, width: u16, height: u16) -> Self {
        Self::with_theme(state, width, height, Theme::default())
    }

    /// Create a harness with an explicit theme.
    pub fn with_theme(state: S, width: u16, height: u16, theme: Theme) -> Self {
        Self {
            state,
            theme,
            width,
            height,
            frame: 0,
        }
    }

    /// Borrow the current application state.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Mutably borrow the current application state.
    pub fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    /// Change the headless viewport for subsequent frames.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// Build and render the next application frame.
    pub fn render(&mut self, view: impl FnOnce(&S, u64) -> Element) -> Buffer {
        let root = view(&self.state, self.frame);
        let buffer = render(root.as_ref(), self.width, self.height, &self.theme);
        self.frame = self.frame.wrapping_add(1);
        buffer
    }

    /// Deliver a signal, rendering only when the update marks the app dirty.
    ///
    /// Resize updates also render because layout depends on viewport geometry.
    /// Otherwise, `None` represents a clean/consumed update or exit; inspect
    /// the returned [`UpdateResult`] to distinguish them.
    pub fn step(
        &mut self,
        signal: Signal,
        update: impl FnOnce(&mut S, Signal) -> UpdateResult,
        view: impl FnOnce(&S, u64) -> Element,
    ) -> (UpdateResult, Option<Buffer>) {
        let resized = self.apply_resize_signal(&signal);
        let result = update(&mut self.state, signal);
        let frame = (result != UpdateResult::Exit && (result == UpdateResult::Dirty || resized))
            .then(|| self.render(view));
        (result, frame)
    }

    fn apply_resize_signal(&mut self, signal: &Signal) -> bool {
        if let Signal::Event(Event::Resize { width, height }) = signal {
            self.resize(*width, *height);
            true
        } else {
            false
        }
    }
}

impl<A: Application> TestHarness<A> {
    /// Build and render the application's next frame, including borrowed views.
    pub fn render_app(&mut self) -> Buffer {
        let root = self.state.view(self.frame);
        let buffer = render(root.as_ref(), self.width, self.height, &self.theme);
        drop(root);
        self.frame = self.frame.wrapping_add(1);
        buffer
    }

    /// Deliver a signal through [`Application::update`] and render when needed.
    ///
    /// Resize events always produce a frame unless the application exits,
    /// matching the terminal runners even when `update` returns
    /// [`UpdateResult::Clean`].
    pub fn step_app(&mut self, signal: Signal) -> (UpdateResult, Option<Buffer>) {
        let resized = self.apply_resize_signal(&signal);
        let result = self.state.update(signal);
        let frame = (result != UpdateResult::Exit && (result == UpdateResult::Dirty || resized))
            .then(|| self.render_app());
        (result, frame)
    }
}

/// Render `view` into a zero-origin in-memory buffer.
pub fn render(view: &dyn View, width: u16, height: u16, theme: &Theme) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paint(&mut buffer, area, theme, view, &[]);
    buffer
}

/// Render `view` with an explicit stylesheet into a zero-origin in-memory buffer.
pub fn render_with_sheet(
    view: &dyn View,
    width: u16,
    height: u16,
    theme: &Theme,
    sheet: StyleSheet,
) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paint_with_sheet(&mut buffer, area, theme, sheet, view, &[]);
    buffer
}

/// Render `view` with a fully configured context into a zero-origin buffer.
pub fn render_with_context(view: &dyn View, width: u16, height: u16, ctx: &RenderCtx) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paint_with_context(&mut buffer, area, ctx, view, &[]);
    buffer
}

/// Render `view` and read it back as a screen reader would hear it.
///
/// Returns the frame's [`Document`](crate::access::Document) as text, one
/// spoken line per line: recorded [`Node`](crate::access::Node)s in reading
/// order, with unlabelled cells read as plain text. Assert on it the way
/// [`grid`] is asserted on for cells.
pub fn screen_reader(view: &dyn View, width: u16, height: u16, theme: &Theme) -> String {
    let frame = crate::access::AccessFrame::new();
    let ctx = RenderCtx::new(theme).with_access(&frame);
    let buffer = render_with_context(view, width, height, &ctx);
    frame.document(&buffer).to_string()
}

/// Convert a buffer to a stable glyph grid suitable for snapshot assertions.
pub fn grid(buffer: &Buffer) -> String {
    let mut output = String::new();
    for y in buffer.area.y..buffer.area.bottom() {
        for x in buffer.area.x..buffer.area.right() {
            output.push_str(buffer[(x, y)].symbol());
        }
        if y + 1 < buffer.area.bottom() {
            output.push('\n');
        }
    }
    output
}

/// Render the same view at several sizes, useful for resize and degenerate-size tests.
pub fn render_sizes(
    view: &dyn View,
    sizes: impl IntoIterator<Item = (u16, u16)>,
    theme: &Theme,
) -> Vec<Buffer> {
    sizes
        .into_iter()
        .map(|(width, height)| render(view, width, height, theme))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::style::Theme;

    #[test]
    fn public_testing_grid_is_stable_and_rectangular() {
        let theme = Theme::default();
        let rendered = render(&Text::raw("hi"), 3, 2, &theme);
        assert_eq!(grid(&rendered), "hi \n   ");
    }

    #[test]
    fn harness_drives_state_and_only_renders_dirty_updates() {
        let mut harness = TestHarness::new(0u8, 2, 1);
        let (result, frame) = harness.step(
            Signal::Tick,
            |state, _| {
                *state += 1;
                UpdateResult::Dirty
            },
            |state, _| crate::element(Text::raw(state.to_string())),
        );
        assert_eq!(result, UpdateResult::Dirty);
        assert_eq!(grid(&frame.unwrap()), "1 ");

        let (_, frame) = harness.step(
            Signal::Tick,
            |_, _| UpdateResult::Clean,
            |_, _| crate::element(Text::raw("unused")),
        );
        assert!(frame.is_none());
    }
}

/// Records the OSC 7501 program-status reports an application sends, so a
/// test can assert status changes the way it asserts cells.
///
/// It is a cloneable [`Write`](std::io::Write): hand one clone to
/// [`Runner::with_status_output`](crate::Runner::with_status_output) (or any
/// writer the host reports through) and keep the other. Everything written is
/// kept; [`reports`](Self::reports) reads the 7501 sequences back as
/// [`Report`]s, skipping anything else on the stream (OSC 9;4, cell output),
/// and [`records`](Self::records) replays them the way a terminal keeps
/// records, `clear` included.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::runner::scripted_events;
/// use tuika::term::program_status::{Report, State};
/// use tuika::term::terminal::Terminal;
/// use tuika::term::testbackend::TestBackend;
/// use tuika::testing::StatusRecorder;
///
/// let recorder = StatusRecorder::new();
/// let runner = Runner::new(RunnerConfig::default()).with_status_output(recorder.clone());
/// let status = runner.program_status();
/// let mut terminal = Terminal::new(TestBackend::new(10, 1)).unwrap();
/// runner
///     .run_driven_by(
///         &mut terminal,
///         &Theme::default(),
///         from_fn(
///             &mut (),
///             |(), _| element(Text::raw("")),
///             |(), signal| match signal {
///                 Signal::Event(Event::Key(k)) if k.code == KeyCode::Enter => {
///                     status.set(Some(Report::new(State::Done)));
///                     UpdateResult::Exit
///                 }
///                 Signal::Event(Event::Key(_)) => {
///                     status.set(Some(Report::new(State::Working)));
///                     UpdateResult::Clean
///                 }
///                 _ => UpdateResult::Clean,
///             },
///         ),
///         scripted_events([
///             Event::Key(Key::new(KeyCode::Char('x'))),
///             Event::Key(Key::new(KeyCode::Enter)),
///         ]),
///     )
///     .unwrap();
///
/// assert_eq!(recorder.states(), [State::Working, State::Done]);
/// assert_eq!(recorder.records()[0].state(), State::Done, "done survives exit");
/// ```
#[derive(Clone, Debug, Default)]
pub struct StatusRecorder {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

impl StatusRecorder {
    /// An empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything written so far, raw.
    pub fn bytes(&self) -> Vec<u8> {
        self.lock().clone()
    }

    /// Forget everything written so far.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Every OSC 7501 report written, in order. Sequences a terminal would
    /// discard are skipped, as a terminal would skip them.
    pub fn reports(&self) -> Vec<Report> {
        let bytes = self.lock();
        let mut out = Vec::new();
        let mut rest: &[u8] = &bytes;
        const INTRO: &[u8] = b"\x1b]7501;";
        while let Some(start) = rest.windows(INTRO.len()).position(|w| w == INTRO) {
            let body = &rest[start + INTRO.len()..];
            let end = body
                .iter()
                .position(|&b| b == 0x1b || b == 0x07)
                .unwrap_or(body.len());
            if let Some(report) = Report::decode(&body[..end]) {
                out.push(report);
            }
            rest = &body[end..];
        }
        out
    }

    /// The state of every report, in order.
    pub fn states(&self) -> Vec<State> {
        self.reports().iter().map(Report::state).collect()
    }

    /// The records a terminal would hold after these reports, in id order
    /// (the root record first): each report replaces its record, and `clear`
    /// removes the addressed record and everything beneath it — every record
    /// when it names no id.
    pub fn records(&self) -> Vec<Report> {
        let mut records: std::collections::BTreeMap<String, Report> = Default::default();
        for report in self.reports() {
            let id = report.record_id().unwrap_or_default().to_string();
            if report.state() == State::Clear {
                let prefix = format!("{id}/");
                records.retain(|key, _| !id.is_empty() && *key != id && !key.starts_with(&prefix));
            } else {
                records.insert(id, report);
            }
        }
        records.into_values().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
        self.bytes.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl std::io::Write for StatusRecorder {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod status_recorder_tests {
    use std::io::Write;

    use super::StatusRecorder;
    use crate::term::program_status::{Report, State};

    #[test]
    fn reads_reports_out_of_a_mixed_stream() {
        let mut recorder = StatusRecorder::new();
        recorder
            .write_all(
                b"cells\x1b]9;4;3;0\x07\x1b]7501;state=working:id=a\x1b\\more\x1b]7501;state=bogus\x07\x1b]7501;state=done\x07",
            )
            .unwrap();
        assert_eq!(recorder.states(), [State::Working, State::Done]);
        assert_eq!(recorder.reports()[0].record_id(), Some("a"));
        recorder.clear();
        assert!(recorder.reports().is_empty());
    }

    #[test]
    fn records_replay_clear_like_a_terminal() {
        let mut recorder = StatusRecorder::new();
        for report in [
            Report::new(State::Idle),
            Report::new(State::Working).id("a"),
            Report::new(State::Blocked).id("a/b"),
            Report::new(State::Done).id("ab"),
            Report::clear(Some("a")),
        ] {
            recorder
                .write_all(report.encode().unwrap().as_bytes())
                .unwrap();
        }
        let ids: Vec<_> = recorder
            .records()
            .iter()
            .map(|r| r.record_id().map(str::to_string))
            .collect();
        assert_eq!(
            ids,
            [None, Some("ab".to_string())],
            "a and a/b are gone, ab stays"
        );

        recorder
            .write_all(Report::clear(None).encode().unwrap().as_bytes())
            .unwrap();
        assert!(
            recorder.records().is_empty(),
            "a bare clear removes every record"
        );
    }
}
