//! OSC 7501 program status reports.
//!
//! A host emits these while it works so the terminal can show something
//! better than a spinning cursor: idle, working (with optional 0-100
//! progress), done, blocked on the user, or errored. Records form a tree
//! via [`Report::id`]: absent means the root record, `parent/child` means
//! a child of `parent`. The host owns the semantics; tuika only encodes.
//!
//! ```rust
//! use tuika::term::program_status::{Report, State};
//!
//! let report = Report::new(State::Working)
//!     .id("build")
//!     .progress(42)
//!     .app("cargo");
//! let bytes = report.encode().expect("valid report");
//! assert!(bytes.starts_with("\x1b]7501;"));
//! ```
//!
//! A host that keeps one status value rather than a stream of reports hands
//! it to the runner instead — see
//! [`Runner::program_status`](crate::Runner::program_status) — or drives a
//! [`StatusReporter`] from its own loop. Either way a report goes out only on
//! change, and exit clears only a `working` or `blocked` record, leaving
//! `done` and `error` for the user as the spec asks.
//!
//! Anything tuika did not produce itself is untrusted: ids, app names,
//! titles, and messages come from the host and are validated here before
//! they reach the terminal. Invalid reports return `None` from
//! [`Report::encode`] and [`crate::term::program_status::write`] emits nothing rather than a
//! half-valid sequence. Unknown terminals swallow the OSC whole, so no
//! capability detection is needed.
//!
//! Spec: <https://www.superlogical.com/rex/docs/build/program-status>
//!
//! ![program status in tuios](https://raw.githubusercontent.com/everruns/tuika/main/examples/program_status.gif)

use std::io::{self, Write};

use crate::term::progress::{self, ProgressState};

/// Program activity states (`state=` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing running. Also the reset for the root record.
    Idle,
    /// Work in progress. May carry `progress=`, `title=`, `msg=`.
    Working,
    /// Finished successfully.
    Done,
    /// Waiting on the user. May carry `kind=`.
    Blocked,
    /// Failed.
    Error,
    /// Clear this record (and its children).
    Clear,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Blocked => "blocked",
            State::Error => "error",
            State::Clear => "clear",
        }
    }
}

/// Why a record is blocked (`kind=` key, only with [`State::Blocked`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Needs a permission grant.
    Permission,
    /// Needs an answer.
    Question,
    /// Needs authentication.
    Auth,
}

impl BlockKind {
    fn as_str(self) -> &'static str {
        match self {
            BlockKind::Permission => "permission",
            BlockKind::Question => "question",
            BlockKind::Auth => "auth",
        }
    }
}

/// One OSC 7501 status report.
///
/// Built with the builder methods, then either [`Report::encode`] for the
/// raw bytes or [`crate::term::program_status::write`] to push them at an I/O handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    state: State,
    id: Option<String>,
    kind: Option<BlockKind>,
    progress: Option<u8>,
    app: Option<String>,
    title: Option<String>,
    msg: Option<String>,
}

impl Report {
    /// A report in `state` for the root record.
    pub fn new(state: State) -> Self {
        Report {
            state,
            id: None,
            kind: None,
            progress: None,
            app: None,
            title: None,
            msg: None,
        }
    }

    /// Clear the record `id` (root when `None`).
    pub fn clear(id: Option<&str>) -> Self {
        let mut report = Report::new(State::Clear);
        if let Some(id) = id {
            report.id = Some(id.to_string());
        }
        report
    }

    /// Record id: `segment(/segment)*`, each 1-32 chars of
    /// `[A-Za-z0-9_.+-]`, at most 8 segments and 128 bytes in all. Absent
    /// means the root record.
    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(id.to_string());
        self
    }

    /// Block reason. Ignored unless the state is [`State::Blocked`].
    pub fn kind(mut self, kind: BlockKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Completion percent. Kept only with `working`/`blocked` and only
    /// when `0..=100`; anything else is dropped at encode time.
    pub fn progress(mut self, percent: u8) -> Self {
        self.progress = Some(percent);
        self
    }

    /// Application name, e.g. `cargo`. Same grammar as one id segment.
    pub fn app(mut self, app: &str) -> Self {
        self.app = Some(app.to_string());
        self
    }

    /// Short label for non-root records. Plain UTF-8; encoded to base64
    /// here. Must be free of control characters.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    /// One human-readable line. Plain UTF-8; encoded to base64 here.
    /// Must be free of control characters.
    pub fn msg(mut self, msg: &str) -> Self {
        self.msg = Some(msg.to_string());
        self
    }

    /// The same record reported `idle`: what the spec asks a program to send
    /// when the user interrupts or cancels its work. Keeps `id`, `app`, and
    /// `title` (a report replaces its record whole, so they must be repeated)
    /// and drops `kind`, `progress`, and `msg`, which described the work.
    pub fn cancelled(&self) -> Self {
        Report {
            state: State::Idle,
            id: self.id.clone(),
            kind: None,
            progress: None,
            app: self.app.clone(),
            title: self.title.clone(),
            msg: None,
        }
    }

    /// The report's state.
    pub fn state(&self) -> State {
        self.state
    }

    /// The record id, `None` for the root record.
    pub fn record_id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The block reason, as set (encoded only with [`State::Blocked`]).
    pub fn block_kind(&self) -> Option<BlockKind> {
        self.kind
    }

    /// The completion percent, as set (encoded only with `working`/`blocked`
    /// and only when `0..=100`).
    pub fn percent(&self) -> Option<u8> {
        self.progress
    }

    /// The application name, as set.
    pub fn app_name(&self) -> Option<&str> {
        self.app.as_deref()
    }

    /// The record label, as set.
    pub fn record_title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The human-readable line, as set.
    pub fn message(&self) -> Option<&str> {
        self.msg.as_deref()
    }

    /// The OSC 9;4 indicator that shows the same thing, so a host that drives
    /// both escapes from one status never shows two disagreeing answers:
    ///
    /// | Report | OSC 9;4 |
    /// | --- | --- |
    /// | `working` with progress | normal, at that percent |
    /// | `working` without | indeterminate |
    /// | `blocked` | warning, at its percent (0 without) |
    /// | `error` | error, full |
    /// | `idle`, `done`, `clear` | clear |
    pub fn native_progress(&self) -> (ProgressState, u8) {
        let percent = self.progress.filter(|p| *p <= 100);
        match self.state {
            State::Working => match percent {
                Some(p) => (ProgressState::Normal, p),
                None => (ProgressState::Indeterminate, 0),
            },
            State::Blocked => (ProgressState::Warning, percent.unwrap_or(0)),
            State::Error => (ProgressState::Error, 100),
            State::Idle | State::Done | State::Clear => (ProgressState::Clear, 0),
        }
    }

    /// Encode the report as `OSC 7501 … ST`, or `None` when invalid.
    ///
    /// Invalid means: bad `id`/`app` grammar, a control character in
    /// `title`/`msg`, or any spec length limit overflow. Combinations
    /// the spec ignores (`kind=` without `blocked`, `progress=` without
    /// `working`/`blocked`) are dropped, not rejected.
    pub fn encode(&self) -> Option<String> {
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(7);
        pairs.push(("state".to_string(), self.state.as_str().to_string()));

        if let Some(id) = &self.id {
            if !is_valid_id(id) {
                return None;
            }
            pairs.push(("id".to_string(), id.clone()));
        }

        if self.state == State::Blocked
            && let Some(kind) = self.kind
        {
            pairs.push(("kind".to_string(), kind.as_str().to_string()));
        }

        if matches!(self.state, State::Working | State::Blocked)
            && let Some(percent) = self.progress
            && percent <= 100
        {
            pairs.push(("progress".to_string(), percent.to_string()));
        }

        if let Some(app) = &self.app {
            if !is_valid_segment(app) {
                return None;
            }
            pairs.push(("app".to_string(), app.clone()));
        }

        if let Some(title) = &self.title {
            let encoded = encode_text(title, 192, 256)?;
            pairs.push(("title".to_string(), encoded));
        }

        if let Some(msg) = &self.msg {
            let encoded = encode_text(msg, 2048, 2732)?;
            pairs.push(("msg".to_string(), encoded));
        }

        let mut out = String::with_capacity(32 + pairs.len() * 16);
        out.push_str("\x1b]7501;");
        for (i, (key, value)) in pairs.iter().enumerate() {
            if i > 0 {
                out.push(':');
            }
            out.push_str(key);
            out.push('=');
            out.push_str(value);
        }
        out.push_str("\x1b\\");

        // Whole-sequence cap from the spec.
        if out.len() > 4096 {
            return None;
        }
        Some(out)
    }
}

/// Encode the `OSC 7501 ; ? ST` feature-detection query.
pub fn encode_query() -> String {
    "\x1b]7501;?\x1b\\".to_string()
}

/// What a terminal said it accepts, from its reply to [`encode_query`].
///
/// A terminal that implements the protocol answers
/// `OSC 7501 ; ? : states=… : kinds=… ST`. Either list may be missing, which
/// means the spec's fixed defaults: every state and every kind. A host that
/// probed adapts reports with [`adapt`](Self::adapt) (or hands the support to
/// [`StatusReporter::with_support`]) so a terminal that lacks a state gets the
/// nearest one it has rather than a report it drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusSupport {
    states: u8,
    kinds: u8,
}

impl Default for StatusSupport {
    /// The spec's defaults: all five states and all three kinds.
    fn default() -> Self {
        Self::ALL
    }
}

impl StatusSupport {
    /// Every state and every kind, the spec's default when a list is absent.
    pub const ALL: Self = Self {
        states: 0b1_1111,
        kinds: 0b111,
    };

    /// Parse a probe reply. `None` when it holds no `OSC 7501 ; ?` reply, or
    /// when the Device Attributes reply came first — the spec's fence: every
    /// terminal answers DA1, so a DA1 answer ahead of any 7501 answer means
    /// the protocol is unsupported, with no timeout to wait out.
    ///
    /// Unknown pairs and unknown values are ignored, as the spec requires.
    /// Bounded by the input: no allocation proportional to it.
    ///
    /// ```
    /// use tuika::term::program_status::{BlockKind, State, StatusSupport};
    ///
    /// let reply = b"\x1b]7501;?:states=idle,working,done\x1b\\\x1b[?62;22c";
    /// let support = StatusSupport::parse(reply).unwrap();
    /// assert!(support.supports(State::Done));
    /// assert!(!support.supports(State::Blocked));
    /// assert!(support.supports_kind(BlockKind::Auth)); // `kinds` absent: all
    ///
    /// assert_eq!(StatusSupport::parse(b"\x1b[?62;22c"), None);
    /// ```
    pub fn parse(reply: &[u8]) -> Option<Self> {
        const INTRO: &[u8] = b"\x1b]7501;?";
        let start = reply.windows(INTRO.len()).position(|w| w == INTRO)?;
        if da1_position(reply).is_some_and(|da1| da1 < start) {
            return None;
        }
        let body = &reply[start + INTRO.len()..];
        // ST is `ESC \`, or BEL; a reply cut off before either still yields
        // what arrived, since every complete pair is unambiguous.
        let end = body
            .iter()
            .position(|&b| b == 0x1b || b == 0x07)
            .unwrap_or(body.len());
        let body = std::str::from_utf8(&body[..end]).ok()?;

        let mut support = Self::ALL;
        for pair in body.split(':') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let values = value.split(',').map(str::trim).filter(|v| !v.is_empty());
            match key.trim() {
                "states" => {
                    support.states = values.filter_map(state_bit).fold(0, |acc, bit| acc | bit);
                }
                "kinds" => {
                    support.kinds = values.filter_map(kind_bit).fold(0, |acc, bit| acc | bit);
                }
                _ => {}
            }
        }
        Some(support)
    }

    /// Whether the terminal accepts `state`. `clear` is not a state and is
    /// always accepted.
    pub fn supports(&self, state: State) -> bool {
        state == State::Clear || state_bit(state.as_str()).is_some_and(|bit| self.states & bit != 0)
    }

    /// Whether the terminal accepts `kind`.
    pub fn supports_kind(&self, kind: BlockKind) -> bool {
        kind_bit(kind.as_str()).is_some_and(|bit| self.kinds & bit != 0)
    }

    /// `report` as this terminal can take it, or `None` when it accepts
    /// nothing close enough.
    ///
    /// A missing state falls back along the nearest meaning: `blocked` to
    /// `working` (both mean "not finished"), then `idle`; `error` to `done`
    /// (both mean "finished, look at me"), then `idle`; `done` and `working`
    /// to `idle`. An unsupported `kind` is dropped, which the spec reads as a
    /// generic block.
    pub fn adapt(&self, report: &Report) -> Option<Report> {
        let fallbacks: &[State] = match report.state {
            State::Blocked => &[State::Working, State::Idle],
            State::Error => &[State::Done, State::Idle],
            State::Done | State::Working => &[State::Idle],
            State::Idle | State::Clear => &[],
        };
        let state = std::iter::once(report.state)
            .chain(fallbacks.iter().copied())
            .find(|state| self.supports(*state))?;
        let mut adapted = report.clone();
        adapted.state = state;
        if adapted.kind.is_some_and(|kind| !self.supports_kind(kind)) {
            adapted.kind = None;
        }
        Some(adapted)
    }
}

/// Where the Device Attributes reply (`CSI ? <params> c`) starts. Other CSI
/// `?` answers — the Kitty keyboard reply is `CSI ? <flags> u` — are not the
/// fence, so the final byte decides.
fn da1_position(reply: &[u8]) -> Option<usize> {
    (0..reply.len()).find(|&i| {
        reply[i..].starts_with(b"\x1b[?")
            && reply[i + 3..]
                .iter()
                .find(|b| !(b.is_ascii_digit() || **b == b';'))
                == Some(&b'c')
    })
}

fn state_bit(name: &str) -> Option<u8> {
    Some(match name {
        "idle" => 1,
        "working" => 1 << 1,
        "done" => 1 << 2,
        "blocked" => 1 << 3,
        "error" => 1 << 4,
        _ => return None,
    })
}

fn kind_bit(name: &str) -> Option<u8> {
    Some(match name {
        "permission" => 1,
        "question" => 1 << 1,
        "auth" => 1 << 2,
        _ => return None,
    })
}

/// Ask the terminal whether it reads OSC 7501, and what it accepts.
///
/// Sends [`encode_query`] followed by the Device Attributes request, which
/// every terminal answers — so an unsupporting terminal costs one round-trip,
/// not a timeout. `None` means unsupported, not a tty, or no answer within
/// `timeout`.
///
/// Same rules as [`Capabilities::query`](crate::term::capabilities::Capabilities::query):
/// **call once at startup, in raw mode, before the event loop reads stdin**.
/// Unix only; elsewhere it returns `None`. Reports are safe to send without
/// probing — unknown OSCs are swallowed — so probe only to adapt reports or
/// to decide whether to show status-related UI.
pub fn query(timeout: std::time::Duration) -> Option<StatusSupport> {
    let request = format!(
        "{}{}",
        encode_query(),
        crate::term::capabilities::DA1_REQUEST
    );
    crate::term::capabilities::probe_terminal(&request, timeout)
        .as_deref()
        .and_then(StatusSupport::parse)
}

/// One task in a tree of work — a sub-agent, a build step — reported to the
/// terminal as its own child record.
///
/// The host keeps the tree it already has (agents and their sub-agents, a
/// pipeline and its jobs) and maps each node to a task: a key, which becomes
/// one segment of the record id, and the [`Report`] that says what the task
/// is doing. [`task_reports`] flattens the tree into `id=parent/child`
/// records, and a [`TaskReporter`] (or
/// [`Runner::task_status`](crate::Runner::task_status)) sends only what
/// changed and clears tasks that disappeared.
///
/// Give each task a [`title`](Report::title): it is the label a terminal
/// shows for a child record, and a report replaces its record whole, so it
/// must be repeated every time. `app` need not be — a record without one
/// inherits its nearest ancestor's.
///
/// ```
/// use tuika::term::program_status::{task_reports, Report, State, StatusTask};
///
/// let tree = [StatusTask::new("agent-1", Report::new(State::Working).title("Planner"))
///     .child(StatusTask::new("search", Report::new(State::Blocked).title("Web search")))];
/// let ids: Vec<_> = task_reports(None, &tree)
///     .iter()
///     .map(|r| r.record_id().unwrap().to_string())
///     .collect();
/// assert_eq!(ids, ["agent-1", "agent-1/search"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusTask {
    segment: String,
    report: Report,
    children: Vec<StatusTask>,
}

impl StatusTask {
    /// A task keyed by `key`, which is made a valid id segment with
    /// [`segment`]: a host can pass its own ids (`agent #3`, a UUID) as they
    /// are. Any id already on `report` is replaced by the task's place in
    /// the tree.
    pub fn new(key: &str, report: Report) -> Self {
        Self {
            segment: segment(key),
            report,
            children: Vec::new(),
        }
    }

    /// Add a child task.
    pub fn child(mut self, task: StatusTask) -> Self {
        self.children.push(task);
        self
    }

    /// Add child tasks.
    pub fn children(mut self, tasks: impl IntoIterator<Item = StatusTask>) -> Self {
        self.children.extend(tasks);
        self
    }

    /// The id segment this task reports under.
    pub fn key(&self) -> &str {
        &self.segment
    }
}

/// Make `key` a valid id segment: bytes outside `[A-Za-z0-9_.+-]` become
/// `-`, it is cut to 32 bytes, and an empty key becomes `_`. Distinct keys
/// can map to the same segment; [`task_reports`] keeps the first of any
/// duplicate id.
pub fn segment(key: &str) -> String {
    let mut out: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Flatten `tasks` into one report per task, depth first, each with its
/// `parent/child` id. `parent` is the id the tree hangs under; `None` puts
/// top-level tasks directly under the root record.
///
/// A subtree that would break the spec's id limits (8 levels, 128 bytes) is
/// left out whole — the terminal would discard those records anyway, and a
/// truncated id would land on the wrong record. A duplicate id keeps its
/// first task.
pub fn task_reports(parent: Option<&str>, tasks: &[StatusTask]) -> Vec<Report> {
    fn walk(
        parent: Option<&str>,
        tasks: &[StatusTask],
        out: &mut Vec<Report>,
        seen: &mut std::collections::BTreeSet<String>,
    ) {
        for task in tasks {
            let id = match parent {
                Some(parent) => format!("{parent}/{}", task.segment),
                None => task.segment.clone(),
            };
            if !is_valid_id(&id) || !seen.insert(id.clone()) {
                continue;
            }
            let mut report = task.report.clone();
            report.id = Some(id.clone());
            out.push(report);
            walk(Some(&id), &task.children, out, seen);
        }
    }
    let mut out = Vec::new();
    if parent.is_some_and(|parent| !is_valid_id(parent)) {
        return out;
    }
    walk(
        parent,
        tasks,
        &mut out,
        &mut std::collections::BTreeSet::new(),
    );
    out
}

/// Keeps a set of child records in step with the terminal: the
/// many-record counterpart of [`StatusReporter`].
///
/// [`update`](Self::update) sends each record whose report changed and
/// clears records that are no longer in the set; [`finish`](Self::finish)
/// clears those still `working` or `blocked`, leaving `done` and `error`.
/// Reports without an id are ignored — the root record is a
/// [`StatusReporter`]'s.
#[derive(Debug, Clone, Default)]
pub struct TaskReporter {
    /// Record id → (state, bytes) last written.
    sent: std::collections::BTreeMap<String, (State, String)>,
    support: StatusSupport,
}

impl TaskReporter {
    /// A reporter that has sent nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adapt reports to what the terminal accepts. See
    /// [`StatusSupport::adapt`].
    pub fn with_support(mut self, support: StatusSupport) -> Self {
        self.support = support;
        self
    }

    /// The ids currently reported, in id order.
    pub fn records(&self) -> impl Iterator<Item = &str> {
        self.sent.keys().map(String::as_str)
    }

    /// Bring the terminal's child records in line with `reports`. Returns
    /// whether anything was written.
    pub fn update(&mut self, out: &mut impl Write, reports: &[Report]) -> io::Result<bool> {
        let mut wrote = false;
        let mut current = std::collections::BTreeSet::new();
        for report in reports {
            let Some(id) = report.id.as_deref() else {
                continue;
            };
            let Some(report) = self.support.adapt(report) else {
                continue;
            };
            let Some(bytes) = report.encode() else {
                continue;
            };
            current.insert(id.to_string());
            if self.sent.get(id).is_some_and(|(_, sent)| *sent == bytes) {
                continue;
            }
            out.write_all(bytes.as_bytes())?;
            wrote = true;
            self.sent.insert(id.to_string(), (report.state, bytes));
        }
        let gone: Vec<String> = self
            .sent
            .keys()
            .filter(|id| !current.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            self.sent.remove(&id);
            write(out, &Report::clear(Some(&id)))?;
            wrote = true;
        }
        if wrote {
            out.flush()?;
        }
        Ok(wrote)
    }

    /// Apply the exit rule to every record: clear the ones still `working`
    /// or `blocked`. Idempotent.
    pub fn finish(&mut self, out: &mut impl Write) -> io::Result<()> {
        let live: Vec<String> = self
            .sent
            .iter()
            .filter(|(_, (state, _))| matches!(state, State::Working | State::Blocked))
            .map(|(id, _)| id.clone())
            .collect();
        for id in live {
            self.sent.remove(&id);
            write(out, &Report::clear(Some(&id)))?;
        }
        out.flush()
    }
}

/// Write one report. Invalid reports emit nothing and still succeed, so a
/// host can fire-and-forget without branching on validation.
pub fn write(out: &mut impl Write, report: &Report) -> std::io::Result<()> {
    if let Some(bytes) = report.encode() {
        out.write_all(bytes.as_bytes())?;
    }
    Ok(())
}

/// Turns "the program's status is now X" into the reports the spec wants,
/// for a host that keeps one status value rather than emitting reports
/// itself. The runners own one per run (see
/// [`Runner::program_status`](crate::Runner::program_status)); a host with its
/// own loop can drive one directly.
///
/// - [`update`](Self::update) writes a report only when the value changed, so
///   it can be called every frame.
/// - [`finish`](Self::finish) is the exit rule: it clears the record only when
///   it was last reported `working` or `blocked`. `done`, `error`, and `idle`
///   are left for the user to find, which is the point of reporting them —
///   a program that sends `clear` on exit wipes the `done` record the spec
///   asks it to leave behind.
/// - With [`with_native_progress`](Self::with_native_progress) the same value
///   also drives OSC 9;4 through [`Report::native_progress`], and `finish`
///   hides that indicator whatever the state, since a stuck taskbar bar after
///   exit is a defect, not a record.
///
/// ```
/// use tuika::term::program_status::{Report, State, StatusReporter};
///
/// let mut out = Vec::new();
/// let mut reporter = StatusReporter::new();
/// let working = Report::new(State::Working).app("build");
/// reporter.update(&mut out, Some(&working)).unwrap();
/// reporter.update(&mut out, Some(&working)).unwrap(); // unchanged: nothing sent
/// reporter.finish(&mut out).unwrap(); // was working: cleared
/// assert_eq!(out, b"\x1b]7501;state=working:app=build\x1b\\\x1b]7501;state=clear\x1b\\");
/// ```
#[derive(Debug, Clone, Default)]
pub struct StatusReporter {
    /// The value last passed to `update`, to skip unchanged ones.
    seen: Option<Report>,
    /// The last report that was valid and written, and its bytes: two values
    /// can encode alike once adapted (progress is dropped from `idle`), and
    /// the terminal needs only one.
    sent: Option<Report>,
    sent_bytes: Option<String>,
    native_progress: bool,
    native_active: bool,
    support: StatusSupport,
}

impl StatusReporter {
    /// A reporter that has sent nothing, with OSC 9;4 mirroring off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Also drive the terminal's OSC 9;4 indicator from the same value.
    pub fn with_native_progress(mut self, enabled: bool) -> Self {
        self.native_progress = enabled;
        self
    }

    /// Adapt every report to what the terminal said it accepts — see
    /// [`StatusSupport::adapt`]. Defaults to [`StatusSupport::ALL`].
    pub fn with_support(mut self, support: StatusSupport) -> Self {
        self.support = support;
        self
    }

    /// The last report written, if any.
    pub fn last_sent(&self) -> Option<&Report> {
        self.sent.as_ref()
    }

    /// Report `current` if it differs from the last value seen. `None` means
    /// the host has no status: a record it reported earlier is cleared.
    /// Returns whether anything was written. An invalid report writes nothing
    /// and is not retried until the value changes.
    pub fn update(&mut self, out: &mut impl Write, current: Option<&Report>) -> io::Result<bool> {
        if self.seen.as_ref() == current {
            return Ok(false);
        }
        self.seen = current.cloned();
        let report = match current {
            Some(report) => report.clone(),
            None => match self.sent.take() {
                Some(previous) => Report::clear(previous.id.as_deref()),
                None => return Ok(false),
            },
        };
        let Some(report) = self.support.adapt(&report) else {
            return Ok(false);
        };
        let Some(bytes) = report.encode() else {
            return Ok(false);
        };
        if self.sent_bytes.as_deref() == Some(bytes.as_str()) {
            return Ok(false);
        }
        // A record moved to a different id leaves the old one behind unless
        // it is cleared: one status value is one record.
        if let Some(previous) = &self.sent
            && previous.id != report.id
            && matches!(previous.state, State::Working | State::Blocked)
            && let Some(clear) = Report::clear(previous.id.as_deref()).encode()
        {
            out.write_all(clear.as_bytes())?;
        }
        out.write_all(bytes.as_bytes())?;
        if self.native_progress {
            let (state, percent) = report.native_progress();
            if state != ProgressState::Clear || self.native_active {
                out.write_all(progress::encode(state, percent).as_bytes())?;
            }
            self.native_active = state != ProgressState::Clear;
        }
        self.sent = (report.state != State::Clear).then_some(report);
        self.sent_bytes = self.sent.is_some().then_some(bytes);
        out.flush()?;
        Ok(true)
    }

    /// Apply the exit rule: clear the record if it was last reported
    /// `working` or `blocked`, and hide a mirrored OSC 9;4 indicator.
    /// Idempotent.
    pub fn finish(&mut self, out: &mut impl Write) -> io::Result<()> {
        if let Some(previous) = self.sent.take() {
            if matches!(previous.state, State::Working | State::Blocked) {
                write(out, &Report::clear(previous.id.as_deref()))?;
                self.sent_bytes = None;
            } else {
                self.sent = Some(previous);
            }
        }
        if self.native_active {
            out.write_all(progress::encode(ProgressState::Clear, 0).as_bytes())?;
            self.native_active = false;
        }
        // A later `update` with the same value must report it again.
        self.seen = self.sent.clone();
        out.flush()
    }
}

fn is_valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 32
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
}

/// Spec limits: at most 128 bytes in all and 8 segments deep. A longer or
/// deeper id is discarded whole by the terminal, never truncated, so it must
/// not be emitted at all.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('/').count() <= 8
        && id.split('/').all(is_valid_segment)
}

/// Check plain text, then base64 it. `None` on control characters or
/// over either the decoded or the encoded limit.
///
/// The spec's control set is C0, DEL, *and* C1 (U+0080..=U+009F): a
/// terminal discards a report whose decoded text holds any of them, so a
/// byte-level C0 check alone would let e.g. U+0085 (NEL) through.
fn encode_text(text: &str, max_decoded: usize, max_encoded: usize) -> Option<String> {
    if text
        .chars()
        .any(|c| c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c))
    {
        return None;
    }
    if text.len() > max_decoded {
        return None;
    }
    let encoded = base64_encode(text.as_bytes());
    if encoded.len() > max_encoded {
        return None;
    }
    Some(encoded)
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((triple >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_working_encodes_with_state_only() {
        assert_eq!(
            Report::new(State::Working).encode(),
            Some("\x1b]7501;state=working\x1b\\".to_string())
        );
    }

    #[test]
    fn full_record_pairs_in_fixed_order() {
        let report = Report::new(State::Working)
            .id("build/lint")
            .progress(42)
            .app("cargo")
            .title("Lint")
            .msg("Deny warnings");
        assert_eq!(
            report.encode(),
            Some(
                "\x1b]7501;state=working:id=build/lint:progress=42:app=cargo:title=TGludA==:msg=RGVueSB3YXJuaW5ncw==\x1b\\"
                    .to_string()
            )
        );
    }

    #[test]
    fn kind_kept_only_with_blocked() {
        let blocked = Report::new(State::Blocked)
            .id("deploy")
            .kind(BlockKind::Permission)
            .encode()
            .unwrap();
        assert!(blocked.contains(":kind=permission"));

        let working = Report::new(State::Working)
            .kind(BlockKind::Permission)
            .encode()
            .unwrap();
        assert!(!working.contains("kind="));
    }

    #[test]
    fn progress_kept_only_with_working_or_blocked() {
        let working = Report::new(State::Working).progress(7).encode().unwrap();
        assert!(working.contains(":progress=7"));

        let done = Report::new(State::Done).progress(7).encode().unwrap();
        assert!(!done.contains("progress="));

        let out_of_range = Report::new(State::Working).progress(101).encode().unwrap();
        assert!(!out_of_range.contains("progress="));
    }

    #[test]
    fn bad_id_or_app_rejects_report() {
        assert_eq!(Report::new(State::Working).id("bad id").encode(), None);
        assert_eq!(Report::new(State::Working).id("").encode(), None);
        assert_eq!(Report::new(State::Working).app("not valid!").encode(), None);
    }

    #[test]
    fn id_over_spec_limits_rejects_report() {
        // 8 segments of 15 bytes plus separators: 127 bytes, the deepest legal id.
        let deepest = vec!["a".repeat(15); 8].join("/");
        assert!(Report::new(State::Working).id(&deepest).encode().is_some());

        let too_deep = ["a"; 9].join("/");
        assert_eq!(Report::new(State::Working).id(&too_deep).encode(), None);

        let too_long = vec!["a".repeat(32); 4].join("/"); // 131 bytes, 4 segments
        assert_eq!(Report::new(State::Working).id(&too_long).encode(), None);
    }

    #[test]
    fn control_chars_in_text_reject_report() {
        assert_eq!(
            Report::new(State::Working).msg("line\nbreak").encode(),
            None
        );
        assert_eq!(
            Report::new(State::Working).title("tab\there").encode(),
            None
        );
        // C1 controls are controls too, though their UTF-8 bytes are >= 0x80.
        assert_eq!(
            Report::new(State::Working).msg("next\u{85}line").encode(),
            None
        );
        assert_eq!(
            Report::new(State::Working).title("csi\u{9b}").encode(),
            None
        );
        assert!(
            Report::new(State::Working)
                .msg("caf\u{e9} \u{a0}ok")
                .encode()
                .is_some()
        );
    }

    #[test]
    fn text_limits_reject_report() {
        assert_eq!(
            Report::new(State::Working).title(&"x".repeat(193)).encode(),
            None
        );
        assert_eq!(
            Report::new(State::Working).msg(&"x".repeat(2049)).encode(),
            None
        );
    }

    #[test]
    fn clear_helper_targets_record() {
        assert_eq!(
            Report::clear(Some("build")).encode(),
            Some("\x1b]7501;state=clear:id=build\x1b\\".to_string())
        );
        assert_eq!(
            Report::clear(None).encode(),
            Some("\x1b]7501;state=clear\x1b\\".to_string())
        );
    }

    #[test]
    fn query_has_expected_shape() {
        assert_eq!(encode_query(), "\x1b]7501;?\x1b\\");
    }

    #[test]
    fn write_skips_invalid_reports() {
        let mut out = Vec::new();
        write(&mut out, &Report::new(State::Working).id("bad id")).unwrap();
        assert!(out.is_empty());

        write(&mut out, &Report::new(State::Done)).unwrap();
        assert_eq!(out, b"\x1b]7501;state=done\x1b\\");
    }

    fn reports(bytes: &[u8]) -> Vec<String> {
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .split("\x1b]")
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.trim_end_matches("\x1b\\")
                    .trim_end_matches('\x07')
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn reporter_sends_only_changes() {
        let mut out = Vec::new();
        let mut reporter = StatusReporter::new();
        let working = Report::new(State::Working).progress(10);
        assert!(reporter.update(&mut out, Some(&working)).unwrap());
        assert!(!reporter.update(&mut out, Some(&working)).unwrap());
        assert!(!reporter.update(&mut out, Some(&working.clone())).unwrap());
        let more = Report::new(State::Working).progress(20);
        assert!(reporter.update(&mut out, Some(&more)).unwrap());
        assert_eq!(
            reports(&out),
            [
                "7501;state=working:progress=10",
                "7501;state=working:progress=20"
            ]
        );
    }

    #[test]
    fn reporter_exit_clears_only_working_and_blocked() {
        for (state, cleared) in [
            (State::Working, true),
            (State::Blocked, true),
            (State::Done, false),
            (State::Error, false),
            (State::Idle, false),
        ] {
            let mut out = Vec::new();
            let mut reporter = StatusReporter::new();
            reporter
                .update(&mut out, Some(&Report::new(state).id("job")))
                .unwrap();
            out.clear();
            reporter.finish(&mut out).unwrap();
            let expected: &[&str] = if cleared {
                &["7501;state=clear:id=job"]
            } else {
                &[]
            };
            assert_eq!(reports(&out), expected, "{state:?}");
            // Idempotent.
            out.clear();
            reporter.finish(&mut out).unwrap();
            assert!(out.is_empty(), "{state:?}");
        }
    }

    #[test]
    fn reporter_none_clears_what_was_reported() {
        let mut out = Vec::new();
        let mut reporter = StatusReporter::new();
        assert!(!reporter.update(&mut out, None).unwrap());
        reporter
            .update(&mut out, Some(&Report::new(State::Done).id("a")))
            .unwrap();
        reporter.update(&mut out, None).unwrap();
        reporter.finish(&mut out).unwrap();
        assert_eq!(
            reports(&out),
            ["7501;state=done:id=a", "7501;state=clear:id=a"]
        );
    }

    #[test]
    fn reporter_moving_ids_clears_the_old_live_record() {
        let mut out = Vec::new();
        let mut reporter = StatusReporter::new();
        reporter
            .update(&mut out, Some(&Report::new(State::Working).id("a")))
            .unwrap();
        reporter
            .update(&mut out, Some(&Report::new(State::Working).id("b")))
            .unwrap();
        assert_eq!(
            reports(&out),
            [
                "7501;state=working:id=a",
                "7501;state=clear:id=a",
                "7501;state=working:id=b"
            ]
        );
    }

    #[test]
    fn reporter_skips_invalid_reports_without_retrying() {
        let mut out = Vec::new();
        let mut reporter = StatusReporter::new();
        let bad = Report::new(State::Working).id("bad id");
        assert!(!reporter.update(&mut out, Some(&bad)).unwrap());
        assert!(!reporter.update(&mut out, Some(&bad)).unwrap());
        reporter.finish(&mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn reporter_mirrors_native_progress_and_hides_it_on_exit() {
        let mut out = Vec::new();
        let mut reporter = StatusReporter::new().with_native_progress(true);
        // Idle before anything is shown sends no stray 9;4 clear.
        reporter
            .update(&mut out, Some(&Report::new(State::Idle)))
            .unwrap();
        reporter
            .update(&mut out, Some(&Report::new(State::Working)))
            .unwrap();
        reporter
            .update(&mut out, Some(&Report::new(State::Working).progress(40)))
            .unwrap();
        reporter
            .update(&mut out, Some(&Report::new(State::Error)))
            .unwrap();
        reporter.finish(&mut out).unwrap();
        assert_eq!(
            reports(&out),
            [
                "7501;state=idle",
                "7501;state=working",
                "9;4;3;0",
                "7501;state=working:progress=40",
                "9;4;1;40",
                "7501;state=error",
                "9;4;2;100",
                "9;4;0;0",
            ]
        );
    }

    #[test]
    fn native_progress_mapping() {
        use ProgressState::*;
        let cases = [
            (Report::new(State::Working).progress(30), (Normal, 30)),
            (Report::new(State::Working), (Indeterminate, 0)),
            (Report::new(State::Blocked).progress(70), (Warning, 70)),
            (Report::new(State::Blocked), (Warning, 0)),
            (Report::new(State::Error), (Error, 100)),
            (Report::new(State::Idle), (Clear, 0)),
            (Report::new(State::Done), (Clear, 0)),
            (Report::clear(None), (Clear, 0)),
            (
                Report::new(State::Working).progress(150),
                (Indeterminate, 0),
            ),
        ];
        for (report, expected) in cases {
            assert_eq!(report.native_progress(), expected, "{report:?}");
        }
    }

    #[test]
    fn cancelled_keeps_identity_and_drops_work() {
        let report = Report::new(State::Blocked)
            .id("job")
            .app("build")
            .title("Build")
            .kind(BlockKind::Question)
            .progress(50)
            .msg("Continue?");
        let cancelled = report.cancelled();
        assert_eq!(cancelled.state(), State::Idle);
        assert_eq!(cancelled.record_id(), Some("job"));
        assert_eq!(cancelled.app_name(), Some("build"));
        assert_eq!(cancelled.record_title(), Some("Build"));
        assert_eq!(cancelled.block_kind(), None);
        assert_eq!(cancelled.percent(), None);
        assert_eq!(cancelled.message(), None);
    }

    #[test]
    fn support_parses_both_terminators_and_whitespace() {
        let st = StatusSupport::parse(
            b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\",
        )
        .unwrap();
        assert_eq!(st, StatusSupport::ALL);
        let bel = StatusSupport::parse(b"\x1b]7501;? : states = idle , done \x07").unwrap();
        assert!(bel.supports(State::Idle) && bel.supports(State::Done));
        assert!(!bel.supports(State::Working));
        assert!(bel.supports(State::Clear), "clear is always accepted");
    }

    #[test]
    fn support_defaults_when_lists_are_absent() {
        assert_eq!(
            StatusSupport::parse(b"\x1b]7501;?\x1b\\"),
            Some(StatusSupport::ALL)
        );
        let kinds_only = StatusSupport::parse(b"\x1b]7501;?:kinds=auth\x1b\\").unwrap();
        assert!(kinds_only.supports(State::Blocked));
        assert!(kinds_only.supports_kind(BlockKind::Auth));
        assert!(!kinds_only.supports_kind(BlockKind::Question));
    }

    #[test]
    fn support_ignores_unknown_pairs_and_values() {
        let st = StatusSupport::parse(
            b"\x1b]7501;?:version=9:states=working,sleeping,idle:junk:kinds=\x1b\\",
        )
        .unwrap();
        assert!(st.supports(State::Working) && st.supports(State::Idle));
        assert!(!st.supports(State::Done));
        assert!(
            !st.supports_kind(BlockKind::Permission),
            "an empty list is empty"
        );
    }

    #[test]
    fn support_uses_the_da1_fence() {
        // DA1 first: the terminal did not answer 7501.
        assert_eq!(StatusSupport::parse(b"\x1b[?62;4c\x1b]7501;?\x1b\\"), None);
        assert_eq!(StatusSupport::parse(b"\x1b[?62;4c"), None);
        assert_eq!(StatusSupport::parse(b""), None);
        // 7501 first, then the fence: supported.
        assert!(StatusSupport::parse(b"\x1b]7501;?\x1b\\\x1b[?62;4c").is_some());
        // A Kitty keyboard answer is a CSI `?` frame but not the fence.
        assert!(StatusSupport::parse(b"\x1b[?1u\x1b]7501;?\x1b\\\x1b[?62c").is_some());
        // Other replies (a palette answer) ahead of both are fine.
        assert!(
            StatusSupport::parse(b"\x1b]11;rgb:0000/0000/0000\x1b\\\x1b]7501;?\x07\x1b[?1c")
                .is_some()
        );
    }

    #[test]
    fn support_parse_survives_garbage() {
        for input in [
            &b"\x1b]7501;?\xff\xfe"[..],
            b"\x1b]7501;?:states=::::",
            b"\x1b]7501;?states",
            b"\x1b]7501",
        ] {
            let _ = StatusSupport::parse(input);
        }
    }

    #[test]
    fn adapt_falls_back_along_meaning() {
        let only_idle_working =
            StatusSupport::parse(b"\x1b]7501;?:states=idle,working\x1b\\").unwrap();
        let blocked = Report::new(State::Blocked)
            .kind(BlockKind::Question)
            .progress(40)
            .msg("Continue?");
        let adapted = only_idle_working.adapt(&blocked).unwrap();
        assert_eq!(adapted.state(), State::Working);
        assert_eq!(adapted.percent(), Some(40));
        assert_eq!(
            only_idle_working
                .adapt(&Report::new(State::Error))
                .unwrap()
                .state(),
            State::Idle
        );
        let no_error = StatusSupport::parse(b"\x1b]7501;?:states=idle,done\x1b\\").unwrap();
        assert_eq!(
            no_error.adapt(&Report::new(State::Error)).unwrap().state(),
            State::Done
        );
        let nothing = StatusSupport::parse(b"\x1b]7501;?:states=\x1b\\").unwrap();
        assert_eq!(nothing.adapt(&Report::new(State::Working)), None);
        assert!(nothing.adapt(&Report::clear(None)).is_some());
    }

    #[test]
    fn adapt_drops_unsupported_kinds() {
        let st = StatusSupport::parse(b"\x1b]7501;?:kinds=permission\x1b\\").unwrap();
        let report = Report::new(State::Blocked).kind(BlockKind::Auth);
        assert_eq!(st.adapt(&report).unwrap().block_kind(), None);
        let report = Report::new(State::Blocked).kind(BlockKind::Permission);
        assert_eq!(
            st.adapt(&report).unwrap().block_kind(),
            Some(BlockKind::Permission)
        );
    }

    #[test]
    fn reporter_adapts_to_support() {
        let support = StatusSupport::parse(b"\x1b]7501;?:states=idle,working\x1b\\").unwrap();
        let mut out = Vec::new();
        let mut reporter = StatusReporter::new().with_support(support);
        reporter
            .update(&mut out, Some(&Report::new(State::Blocked)))
            .unwrap();
        reporter.finish(&mut out).unwrap();
        assert_eq!(
            reports(&out),
            ["7501;state=working", "7501;state=clear"],
            "blocked went out as working, and working is cleared on exit"
        );
    }

    #[test]
    fn reporter_skips_values_that_encode_alike() {
        let mut out = Vec::new();
        let support = StatusSupport::parse(b"\x1b]7501;?:states=idle\x1b\\").unwrap();
        let mut reporter = StatusReporter::new().with_support(support);
        for percent in [10, 20, 30] {
            reporter
                .update(
                    &mut out,
                    Some(&Report::new(State::Working).progress(percent)),
                )
                .unwrap();
        }
        assert_eq!(reports(&out), ["7501;state=idle"]);
    }

    #[test]
    fn segment_makes_any_key_valid() {
        assert_eq!(segment("agent #3"), "agent--3");
        assert_eq!(segment(""), "_");
        assert_eq!(segment("ü"), "-");
        assert_eq!(segment(&"x".repeat(40)).len(), 32);
        assert!(is_valid_segment(&segment("a/b:c;d=e")));
    }

    #[test]
    fn task_reports_flatten_depth_first_with_paths() {
        let tree = [
            StatusTask::new("a", Report::new(State::Working).title("A"))
                .child(StatusTask::new("x", Report::new(State::Done).id("ignored")))
                .child(
                    StatusTask::new("y", Report::new(State::Blocked))
                        .child(StatusTask::new("z", Report::new(State::Working))),
                ),
            StatusTask::new("b", Report::new(State::Idle)),
        ];
        let ids: Vec<String> = task_reports(None, &tree)
            .iter()
            .map(|r| r.record_id().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["a", "a/x", "a/y", "a/y/z", "b"]);
        let nested: Vec<String> = task_reports(Some("run"), &tree[1..])
            .iter()
            .map(|r| r.record_id().unwrap().to_string())
            .collect();
        assert_eq!(nested, ["run/b"]);
        assert!(task_reports(Some("bad id"), &tree).is_empty());
    }

    #[test]
    fn task_reports_drop_subtrees_past_the_limits_and_duplicates() {
        let mut deep = StatusTask::new("l8", Report::new(State::Working));
        for level in (1..=8).rev() {
            deep = StatusTask::new(&format!("l{level}"), Report::new(State::Working)).child(deep);
        }
        let reports = task_reports(None, &[deep]);
        assert_eq!(reports.len(), 8, "the ninth level is past the spec's depth");
        assert!(reports.iter().all(|r| r.encode().is_some()));

        let dupes = [
            StatusTask::new("a b", Report::new(State::Working)),
            StatusTask::new("a:b", Report::new(State::Done)),
        ];
        let reports = task_reports(None, &dupes);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].state(), State::Working);
    }

    #[test]
    fn task_reporter_sends_changes_and_clears_vanished_tasks() {
        let mut out = Vec::new();
        let mut reporter = TaskReporter::new();
        let two = task_reports(
            None,
            &[
                StatusTask::new("a", Report::new(State::Working)),
                StatusTask::new("b", Report::new(State::Working)),
            ],
        );
        assert!(reporter.update(&mut out, &two).unwrap());
        assert!(!reporter.update(&mut out, &two).unwrap());
        let one = task_reports(None, &[StatusTask::new("a", Report::new(State::Done))]);
        reporter.update(&mut out, &one).unwrap();
        assert_eq!(
            reports(&out),
            [
                "7501;state=working:id=a",
                "7501;state=working:id=b",
                "7501;state=done:id=a",
                "7501;state=clear:id=b",
            ]
        );
        assert_eq!(reporter.records().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn task_reporter_exit_clears_only_live_records() {
        let mut out = Vec::new();
        let mut reporter = TaskReporter::new();
        let tree = [StatusTask::new("a", Report::new(State::Done))
            .child(StatusTask::new("b", Report::new(State::Blocked)))
            .child(StatusTask::new("c", Report::new(State::Error)))];
        // The root report is a `StatusReporter`'s, not a task.
        assert!(
            !reporter
                .update(&mut out, &[Report::new(State::Working)])
                .unwrap()
        );
        reporter
            .update(&mut out, &task_reports(None, &tree))
            .unwrap();
        out.clear();
        reporter.finish(&mut out).unwrap();
        assert_eq!(reports(&out), ["7501;state=clear:id=a/b"]);
        out.clear();
        reporter.finish(&mut out).unwrap();
        assert!(out.is_empty());
    }
}
