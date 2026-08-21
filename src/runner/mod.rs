//! Small full-screen run loops.
//!
//! Both runners tie the same host primitives together — [`TerminalSession`] for
//! the screen, [`translate_event`] for input, [`crate::paint`] for the frame — so a
//! host that wants lifecycle, redraw scheduling, and event translation in one
//! place does not have to assemble them itself. Either [`ScreenMode`] works:
//! pick one in [`RunnerConfig::screen_mode`] and the loop reserves, keeps, and
//! releases a split footer for you.
//! On real terminals they also detect image support, supply a per-frame image
//! layer through [`RenderCtx`], emit it after the cell frame, and bound resize
//! redraws to one frame every 16 ms.
//! When their session captures the mouse, they also restore plain drag text
//! selection over the final rendered cells and copy it through OSC 52.
//!
//! [`Runner`] is the synchronous loop and is always available. [`AsyncRunner`]
//! (`feature = "async"`) is the same loop for hosts already on Tokio; it lives
//! behind the feature so a sync-only host never pulls a runtime into its build.
//! They are one module because they are one concept — picking between them is a
//! question about the host's existing runtime, not about which part of tuika to
//! reach for.
//!
//! A host with its own event loop needs neither: call [`crate::paint`] directly.
//!
//! # Choosing a `run` method
//!
//! There is one loop and one boundary, so the surface is small. Every run method
//! takes a [`FrameSource`] — `&mut app` for an [`Application`], or [`from_fn`]
//! for the closure form — and the remaining names say only where the terminal
//! and the input come from:
//!
//! | Method | Terminal | Input |
//! | --- | --- | --- |
//! | [`Runner::run`] / [`AsyncRunner::run`] | the runner enters a [`TerminalSession`] | terminal events and ticks |
//! | [`Runner::run_with_backend`] / [`AsyncRunner::run_with_backend`] | the runner enters a session over your backend | terminal events and ticks |
//! | [`AsyncRunner::run_with_messages`] | the runner enters a session | plus a typed application stream |
//! | [`Runner::run_driven_by`] / [`AsyncRunner::run_driven_by`] | yours, no lifecycle | your event source (async: your event and message streams) |
//!
//! The runtime axis is the runner you construct, and everything else — screen
//! mode, tick rate, session policy, text selection — is [`RunnerConfig`] or a
//! builder method rather than another name.
//!
//! `run_driven_by` is the loop with nothing around it: no session, no terminal
//! construction, no stdout-facing work. Both runners build their other entry
//! points on it, and a test can drive a whole application through it over a
//! [`TestBackend`](crate::term::testbackend::TestBackend) — with
//! [`scripted_events`] on the synchronous side — so the loop is testable
//! without a tty.
//!
//! Messages ride the same [`Signal`] the loop always delivered:
//! `Signal<M>::Message` carries them, and `M` defaults to the uninhabited
//! [`Infallible`], so a source that never sees a message stream has no variant
//! to handle. That is also why the message axis is [`AsyncRunner`]-only — it
//! needs a Tokio `Stream` to select over, and a synchronous loop has nothing to
//! select with. A synchronous host whose background work produces data keeps
//! that data behind a [`Live`](crate::live::Live) (or any shared value) and
//! marks the frame stale through [`Runner::redraw_handle`]; both runners expose
//! that handle.
//!
//! # The two frame sources
//!
//! [`FrameSource`] has exactly two implementors, and neither is legacy:
//!
//! - **An application** — [`Application`] (or [`AsyncApplication`]), passed as
//!   `&mut app`. `view(&self)` returns a [`ScopedElement<'_>`](crate::ScopedElement):
//!   a tree that borrows the application for the frame, so a large transcript,
//!   table, or log is painted in place instead of being cloned into an owned
//!   tree or shared through `Rc<RefCell<_>>`. The `&self` receiver also states
//!   the contract the closure form only documents: rendering is pure.
//! - **Closures** — [`from_fn`]`(&mut state, view, update)`. It renders an
//!   *owned* [`Element`], but the view closure is `FnMut`, so it may keep
//!   scratch state of its own.
//!
//! Since `Element` is `ScopedElement<'static>`, the application form is the more
//! general of the two in what it can *return*; the closure form is the more
//! permissive in how the view may *capture*. Start with closures, implement
//! [`Application`] when a frame wants to borrow host data.

#[cfg(feature = "async")]
mod asynchronous;
#[cfg(feature = "async")]
mod stream;

#[cfg(feature = "async")]
#[allow(deprecated)]
pub use asynchronous::AsyncSignal;
#[cfg(feature = "async")]
pub use asynchronous::{
    AsyncApplication, AsyncFrameSource, AsyncFromFn, AsyncRunner, async_from_fn, no_messages,
};

use std::convert::Infallible;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::term::backend::CrosstermBackend;
use crate::term::terminal::{Terminal, TerminalOptions};
use crate::term::traits::Backend;
use crossterm::event;

use crate::host::paint_with_context_and_selection;
use crate::live::RedrawHandle;
use crate::mouse::{SelectionState, paint_selection, selected_text_from_sources};
use crate::screen::{ScreenMode, Scrollback, close_footer, pin_footer};
use crate::term::clipboard;
use crate::term::image::{ImageLayer, ImageSupport};
use crate::view::SelectionFrame;
use crate::{
    Clock, Element, Event, RenderCtx, ScopedElement, SystemClock, TerminalSession, Theme, View,
    translate_event,
};

/// Receives a frame's root view for painting.
///
/// A frame source hands its root to this callback instead of returning it,
/// which is what lets one loop serve an owned [`Element`] and a tree that
/// borrows application state: the borrow only has to outlive the call.
pub(crate) type PaintRoot<'a> = &'a mut dyn FnMut(&dyn View);

const RESIZE_FRAME_INTERVAL: Duration = Duration::from_millis(16);

fn resize_redraw_at(last_frame: Instant, now: Instant) -> Instant {
    last_frame
        .checked_add(RESIZE_FRAME_INTERVAL)
        .map_or(now, |deadline| deadline.max(now))
}

fn schedule_redraw(deadline: &mut Option<Instant>, at: Instant) {
    *deadline = Some(deadline.map_or(at, |current| current.min(at)));
}

struct FrameGraphics {
    support: ImageSupport,
    layer: ImageLayer,
}

impl FrameGraphics {
    fn detected() -> Self {
        Self {
            support: ImageSupport::detect(),
            layer: ImageLayer::new(),
        }
    }

    fn render_context<'a>(&'a self, theme: &'a Theme) -> RenderCtx<'a> {
        RenderCtx::new(theme).with_image_graphics(self.support, &self.layer)
    }

    fn finish_frame(&self) -> io::Result<()> {
        let mut output = io::stdout();
        self.finish_frame_to(&mut output)
    }

    fn finish_frame_to(&self, output: &mut impl Write) -> io::Result<()> {
        self.layer.emit(output)?;
        output.flush()?;
        self.layer.clear();
        Ok(())
    }
}

struct FrameGraphicsCleanup<'a>(&'a FrameGraphics);

impl Drop for FrameGraphicsCleanup<'_> {
    fn drop(&mut self) {
        let _ = self.0.finish_frame();
    }
}

#[derive(Clone, Copy, Debug)]
/// Options for [`Runner`] and [`AsyncRunner`].
pub struct RunnerConfig {
    /// Maximum time between frames and data-driven redraw checks.
    pub tick_rate: Duration,
    /// Which part of the terminal the frame owns. Defaults to
    /// [`ScreenMode::Alternate`].
    pub screen_mode: ScreenMode,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            tick_rate: Duration::from_millis(100),
            screen_mode: ScreenMode::default(),
        }
    }
}

/// Why a runner is calling `update`: a tick, terminal input, or a value the
/// host's own program produced.
///
/// `M` is that last one's type, for a loop that also selects over a message
/// stream. It defaults to [`Infallible`], the uninhabited type: a plain
/// `Signal` therefore *has* no [`Message`](Self::Message) variant to construct
/// or match, and code that handles only ticks and events stays exhaustive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal<M = Infallible> {
    /// The configured tick interval elapsed.
    Tick,
    /// A translated terminal input event arrived. Resize events force a redraw
    /// after the update unless it exits, even when the update is clean.
    Event(Event),
    /// A value from the host's own program — a background task, a worker, a
    /// feed — delivered through the runner's message stream. Unreachable unless
    /// the runner was given one.
    ///
    /// **This is not the Elm/Iced `Msg`.** There, one message type carries
    /// *everything* that reaches `update`, key presses included. Here a key
    /// press is [`Event`](Self::Event) and the clock is [`Tick`](Self::Tick);
    /// this variant is only for what the terminal did not produce. See
    /// [`AsyncRunner::run_with_messages`] for when to reach for it.
    Message(M),
}

impl<M> Signal<M> {
    fn requires_redraw(&self) -> bool {
        matches!(self, Self::Event(Event::Resize { .. }))
    }
}

/// What a runner should do after an update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateResult {
    /// The signal was not handled. Keep waiting without rebuilding or
    /// repainting; the runner may apply a default interaction such as text
    /// selection.
    #[default]
    Clean,
    /// The signal was handled without changing persistent state or repainting.
    /// This prevents runner-provided default interactions.
    Consumed,
    /// The signal was handled and the view must be rebuilt and repainted.
    Dirty,
    /// Stop the runner without painting another frame.
    Exit,
}

/// A data-driven terminal application: what [`Runner::run`] drives.
///
/// [`AsyncApplication`] is the same boundary for a host whose `update` awaits;
/// the split exists because only the runtime differs, not the contract.
///
/// The runner mutably borrows the application only while delivering a
/// [`Signal`], then immutably borrows it to build the next frame. Because the
/// returned tree is scoped to that immutable borrow, custom views can read
/// application data directly without cloning it into an owned [`Element`] or
/// sharing it through `Rc<RefCell<_>>`.
///
/// Rendering should be pure: persistent UI and domain state belongs on the
/// application and changes only in [`update`](Self::update).
pub trait Application<M = Infallible> {
    /// Update application state in response to a tick, terminal event, or
    /// message. Return [`UpdateResult::Clean`] only when the signal was
    /// unhandled, or [`UpdateResult::Consumed`] when it was handled without a
    /// repaint.
    fn update(&mut self, signal: Signal<M>) -> UpdateResult;

    /// Build the ephemeral view tree for one numbered frame.
    fn view(&self, frame: u64) -> ScopedElement<'_>;
}

/// What a run loop pulls frames and update decisions from.
///
/// This is the runner's single boundary, and the reason there is no separate `run`
/// method per frame source. Two things implement it:
///
/// - `&mut A` where `A: Application` — the borrowed-view boundary. Its `view(&self)`
///   may return a tree that borrows the application for the frame.
/// - [`from_fn`] — a `state` value beside `view`/`update` closures, for a host
///   that would rather not name a type.
///
/// The painting callback is what unifies them: a source hands its root to the
/// runner instead of returning it, so a borrowed tree only has to outlive the
/// call. Implement [`Application`] rather than this trait; this one exists to
/// be *dispatched* on.
pub trait FrameSource<M = Infallible> {
    /// Deliver one signal.
    fn update(&mut self, signal: Signal<M>) -> UpdateResult;

    /// Build one numbered frame and hand its root to `paint`.
    fn frame(&mut self, frame: u64, paint: PaintRoot<'_>);
}

impl<M, A: Application<M>> FrameSource<M> for &mut A {
    fn update(&mut self, signal: Signal<M>) -> UpdateResult {
        Application::update(*self, signal)
    }

    fn frame(&mut self, frame: u64, paint: PaintRoot<'_>) {
        paint(Application::view(*self, frame).as_ref());
    }
}

/// A [`FrameSource`] built from a state value and a pair of closures.
///
/// Returned by [`from_fn`]; see it for the trade against implementing
/// [`Application`].
pub struct FromFn<'state, S, V, U> {
    state: &'state mut S,
    view: V,
    update: U,
}

/// Build a [`FrameSource`] from `state` and closures over it.
///
/// The closure form of the boundary. State is a separate argument rather than a
/// capture because one closure needs `&state` while the other needs
/// `&mut state`, and a single closure cannot hold both.
///
/// It renders an *owned* [`Element`], so a frame that wants to borrow host data
/// wants [`Application`] instead. In exchange the view closure is `FnMut`, so
/// it may keep scratch state of its own.
///
/// ```no_run
/// use tuika::prelude::*;
/// use tuika::runner::from_fn;
///
/// # fn main() -> std::io::Result<()> {
/// let mut count = 0u32;
/// Runner::new(RunnerConfig::default()).run(
///     &Theme::default(),
///     from_fn(
///         &mut count,
///         |count, _frame| element(Text::raw(format!("count: {count}"))),
///         |count, signal| match signal {
///             Signal::Event(Event::Key(key)) if key.code == KeyCode::Esc => UpdateResult::Exit,
///             Signal::Event(Event::Key(_)) => {
///                 *count += 1;
///                 UpdateResult::Dirty
///             }
///             _ => UpdateResult::Clean,
///         },
///     ),
/// )
/// # }
/// ```
pub fn from_fn<S, V, U, M>(state: &mut S, view: V, update: U) -> FromFn<'_, S, V, U>
where
    V: FnMut(&S, u64) -> Element,
    U: FnMut(&mut S, Signal<M>) -> UpdateResult,
{
    FromFn {
        state,
        view,
        update,
    }
}

impl<M, S, V, U> FrameSource<M> for FromFn<'_, S, V, U>
where
    V: FnMut(&S, u64) -> Element,
    U: FnMut(&mut S, Signal<M>) -> UpdateResult,
{
    fn update(&mut self, signal: Signal<M>) -> UpdateResult {
        (self.update)(self.state, signal)
    }

    fn frame(&mut self, frame: u64, paint: PaintRoot<'_>) {
        paint((self.view)(self.state, frame).as_ref());
    }
}

/// Where a synchronous [`Runner`] gets its terminal input.
///
/// [`Runner::run`] uses the real terminal; [`Runner::run_driven_by`] takes one
/// of these instead, which is what lets a host — or a test — drive the loop
/// without a tty. The asynchronous runner takes a
/// [`Stream`](futures_core::Stream) for the same reason.
///
/// An implementation must **consume the timeout** when it has nothing to
/// deliver: the loop uses this call as its only sleep, so returning `Ok(None)`
/// immediately would spin the CPU. [`scripted_events`] handles that for a
/// finite script.
/// `Er` is the run's error type, shared with the backend: for the real terminal
/// that is [`io::Error`], but leaving it generic lets an infallible backend
/// ([`TestBackend`](crate::term::testbackend::TestBackend), whose error is
/// [`Infallible`]) pair with an infallible source.
pub trait EventSource<Er = io::Error> {
    /// Wait up to `timeout` for the next event.
    ///
    /// `Ok(None)` means the timeout elapsed with nothing to report, which is
    /// how a source stays idle without ending the run.
    fn poll_event(&mut self, timeout: Duration) -> Result<Option<Event>, Er>;
}

/// The real terminal, read through crossterm. What [`Runner::run`] uses.
struct CrosstermEvents;

impl EventSource<io::Error> for CrosstermEvents {
    fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? {
            // A crossterm event tuika does not model is not an idle timeout —
            // report it as nothing this round and let the loop re-poll.
            return Ok(translate_event(event::read()?));
        }
        Ok(None)
    }
}

/// A finite [`EventSource`] over a known sequence of events.
///
/// Returned by [`scripted_events`].
pub struct ScriptedEvents<I> {
    events: I,
    idle: fn(Duration),
}

/// Build an [`EventSource`] that replays `events` in order.
///
/// Each call takes the next event immediately, ignoring the timeout — a
/// scripted run should not wait on a clock. That also means there is no idle
/// gap *between* events, so a repaint the loop defers (the one a resize
/// schedules) may not land before the next event arrives; implement
/// [`EventSource`] yourself when a test needs that gap. Once the script is
/// exhausted the
/// source goes idle like a quiet terminal, sleeping out each timeout, so tick
/// pacing is unchanged and the loop still exits only on
/// [`UpdateResult::Exit`]. End a script with an event that exits, or set a
/// short [`RunnerConfig::tick_rate`] and exit on a tick.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::runner::scripted_events;
/// use tuika::term::testbackend::TestBackend;
/// use tuika::term::terminal::Terminal;
///
/// # fn main() {
/// let mut terminal = Terminal::new(TestBackend::new(20, 1)).unwrap();
/// let mut count = 0u32;
///
/// Runner::new(RunnerConfig::default())
///     .run_driven_by(
///         &mut terminal,
///         &Theme::default(),
///         from_fn(
///             &mut count,
///             |count, _frame| element(Text::raw(format!("count: {count}"))),
///             |count, signal| match signal {
///                 Signal::Event(Event::Key(key)) if key.code == KeyCode::Esc => {
///                     UpdateResult::Exit
///                 }
///                 Signal::Event(Event::Key(_)) => {
///                     *count += 1;
///                     UpdateResult::Dirty
///                 }
///                 _ => UpdateResult::Clean,
///             },
///         ),
///         scripted_events([
///             Event::Key(Key::new(KeyCode::Char('j'))),
///             Event::Key(Key::new(KeyCode::Esc)),
///         ]),
///     )
///     .unwrap();
///
/// assert_eq!(count, 1, "the whole loop ran with no terminal at all");
/// # }
/// ```
pub fn scripted_events<I>(events: I) -> ScriptedEvents<I::IntoIter>
where
    I: IntoIterator<Item = Event>,
{
    ScriptedEvents {
        events: events.into_iter(),
        idle: std::thread::sleep,
    }
}

impl<I: Iterator<Item = Event>, Er> EventSource<Er> for ScriptedEvents<I> {
    fn poll_event(&mut self, timeout: Duration) -> Result<Option<Event>, Er> {
        match self.events.next() {
            Some(event) => Ok(Some(event)),
            None => {
                (self.idle)(timeout);
                Ok(None)
            }
        }
    }
}

/// Runtime-neutral decision produced by [`RunnerCore`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerAction {
    /// Wait for another signal or an external redraw request.
    Wait,
    /// Render the given animation frame number.
    Render(u64),
    /// End the application loop.
    Exit,
}

/// Pure runner state machine shared by the synchronous and async runners and
/// available to custom runtimes and test hosts.
///
/// It knows nothing about Crossterm, Tokio, clocks, sleeping, or backends. A
/// host supplies signals to application code, passes the resulting
/// [`UpdateResult`] here, and performs the returned [`RunnerAction`].
#[derive(Clone, Debug)]
pub struct RunnerCore {
    next_frame: u64,
    dirty: bool,
    exited: bool,
}

impl Default for RunnerCore {
    fn default() -> Self {
        Self::new()
    }
}

impl RunnerCore {
    /// Create a core that requests an initial frame.
    pub const fn new() -> Self {
        Self {
            next_frame: 0,
            dirty: true,
            exited: false,
        }
    }

    /// Apply an application's update result.
    pub fn apply(&mut self, result: UpdateResult) {
        match result {
            UpdateResult::Clean | UpdateResult::Consumed => {}
            UpdateResult::Dirty => self.dirty = true,
            UpdateResult::Exit => self.exited = true,
        }
    }

    /// Request a frame independently of an application signal.
    pub fn request_redraw(&mut self) {
        self.dirty = true;
    }

    /// Whether an exit result has made this core terminal.
    pub const fn is_exited(&self) -> bool {
        self.exited
    }

    /// Take the next action. A render consumes the dirty flag and advances the
    /// wrapping animation frame counter.
    pub fn next_action(&mut self) -> RunnerAction {
        if self.exited {
            return RunnerAction::Exit;
        }
        if self.dirty {
            self.dirty = false;
            let frame = self.next_frame;
            self.next_frame = self.next_frame.wrapping_add(1);
            RunnerAction::Render(frame)
        } else {
            RunnerAction::Wait
        }
    }
}

/// A synchronous Crossterm event and rendering loop.
pub struct Runner {
    config: RunnerConfig,
    clock: Arc<dyn Clock + Send + Sync>,
    redraw: RedrawHandle,
    scrollback: Scrollback,
    session_config: Option<crate::host::TerminalSessionConfig>,
    text_selection: bool,
}

impl Runner {
    /// Create a runner without touching the terminal.
    pub fn new(config: RunnerConfig) -> Self {
        Self::with_clock(config, SystemClock)
    }

    /// Create a runner driven by an explicit monotonic clock.
    ///
    /// The system clock remains the default. Supplying a virtual clock makes
    /// tick scheduling deterministic for replayable hosts and tests; advance a
    /// shared clock while the runner is waiting so time can progress.
    pub fn with_clock(mut config: RunnerConfig, clock: impl Clock + Send + Sync + 'static) -> Self {
        // A zero interval would busy-spin even when the application has no
        // events or updates. Keep the public config ergonomic while enforcing
        // a safe scheduling floor at the boundary.
        config.tick_rate = config.tick_rate.max(Duration::from_millis(1));
        Self {
            config,
            clock: Arc::new(clock),
            redraw: RedrawHandle::default(),
            scrollback: Scrollback::new(),
            session_config: None,
            text_selection: true,
        }
    }

    /// Return a handle for publishing content above a
    /// [`ScreenMode::SplitFooter`] — see [`Scrollback`]. Blocks queued while
    /// running in [`ScreenMode::Alternate`] are discarded, since there is no
    /// scrollback of the host's to write into.
    pub fn scrollback(&self) -> Scrollback {
        self.scrollback.clone()
    }

    /// Return a handle that background producers can use to request redraws.
    ///
    /// A request says only *the next frame is stale*, and requests coalesce: the
    /// handle is a flag, so a burst produces one repaint. Pair it with a
    /// [`Live`](crate::live::Live) (or any shared value) the view re-reads each
    /// frame.
    ///
    /// When each arrival instead needs `update` to decide something, an
    /// [`AsyncRunner`] can deliver it as a [`Signal::Message`] through
    /// [`run_with_messages`](AsyncRunner::run_with_messages). The synchronous
    /// loop has no equivalent: it blocks in `crossterm::event::poll`, which a
    /// channel send cannot wake, so a message would arrive no sooner than the
    /// next tick. A synchronous host wanting that shape drives its own loop with
    /// [`crate::paint`].
    pub fn redraw_handle(&self) -> RedrawHandle {
        self.redraw.clone()
    }

    /// Override terminal lifecycle policy while retaining the runner's loop.
    pub fn with_session_config(mut self, config: crate::host::TerminalSessionConfig) -> Self {
        self.config.screen_mode = config.screen_mode;
        self.session_config = Some(config);
        self
    }

    /// Enable or disable runner-provided drag selection over the final cell
    /// frame. It is enabled by default whenever the terminal session captures
    /// the mouse. Applications claim a gesture by returning
    /// [`UpdateResult::Consumed`] or [`UpdateResult::Dirty`] for its events.
    pub fn with_text_selection(mut self, enabled: bool) -> Self {
        self.text_selection = enabled;
        self
    }

    fn selects_text(&self) -> bool {
        self.text_selection
            && self.session_config.map_or_else(
                || self.config.screen_mode.captures_mouse(),
                crate::host::TerminalSessionConfig::captures_mouse,
            )
    }

    /// Run until the frame source returns [`UpdateResult::Exit`].
    ///
    /// `frames` is either `&mut app` for an [`Application`] or [`from_fn`] over
    /// a state value and closures — see [`FrameSource`].
    ///
    /// The runner paints once initially. It then delivers input and periodic
    /// [`Signal::Tick`] values, repainting only on [`UpdateResult::Dirty`] or
    /// when a [`RedrawHandle`] requests it.
    pub fn run<F: FrameSource>(&self, theme: &Theme, mut frames: F) -> io::Result<()> {
        let graphics = FrameGraphics::detected();
        self.run_inner(
            theme,
            CrosstermBackend::new(io::stdout()),
            &mut frames,
            Some(&graphics),
        )
    }

    /// Run with a caller-provided backend, such as
    /// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend).
    pub fn run_with_backend<F, B>(&self, theme: &Theme, backend: B, mut frames: F) -> io::Result<()>
    where
        F: FrameSource,
        B: Backend<Error = io::Error>,
    {
        self.run_inner(theme, backend, &mut frames, None)
    }

    /// Run a data-driven [`Application`] on the real terminal.
    #[deprecated(since = "0.9.0", note = "use `run(theme, &mut app)` instead")]
    pub fn run_app<A: Application>(&self, theme: &Theme, app: &mut A) -> io::Result<()> {
        self.run(theme, app)
    }

    /// Run a data-driven [`Application`] with a caller-provided backend.
    #[deprecated(
        since = "0.9.0",
        note = "use `run_with_backend(theme, backend, &mut app)` instead"
    )]
    pub fn run_app_with_backend<A, B>(
        &self,
        theme: &Theme,
        backend: B,
        app: &mut A,
    ) -> io::Result<()>
    where
        A: Application,
        B: Backend<Error = io::Error>,
    {
        self.run_with_backend(theme, backend, app)
    }

    /// Run against a caller-owned terminal and event source, with no terminal
    /// lifecycle of the runner's own.
    ///
    /// This is the loop itself, and what [`run`](Self::run) builds on — the
    /// synchronous counterpart to
    /// [`AsyncRunner::run_driven_by`](crate::AsyncRunner::run_driven_by). A host
    /// that already owns its terminal and input can call it directly, and a test
    /// can drive a whole application over a
    /// [`TestBackend`](crate::term::testbackend::TestBackend) with
    /// [`scripted_events`] — no tty, no raw mode, no real clock.
    ///
    /// It enters no [`TerminalSession`], so raw mode, the alternate screen, and
    /// mouse capture are the caller's to arrange. It also performs no graphics
    /// or clipboard I/O, since both target the process's own stdout.
    pub fn run_driven_by<F, B, E, Er>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        mut frames: F,
        mut events: E,
    ) -> Result<(), Er>
    where
        F: FrameSource,
        B: Backend<Error = Er>,
        E: EventSource<Er>,
    {
        self.drive(terminal, theme, &mut frames, &mut events, None, |_| Ok(()))
    }

    fn run_inner<F, B>(
        &self,
        theme: &Theme,
        backend: B,
        frames: &mut F,
        graphics: Option<&FrameGraphics>,
    ) -> io::Result<()>
    where
        F: FrameSource,
        B: Backend<Error = io::Error>,
    {
        let mode = self.config.screen_mode;
        let split = !mode.is_alternate();
        let _session = if let Some(config) = self.session_config {
            TerminalSession::enter_config(config)?
        } else {
            TerminalSession::enter_with(mode)?
        };
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: mode.viewport(),
            },
        )?;
        // Declared after the session so cleanup runs first on every exit path,
        // while placements still belong to the screen on which they were made.
        let _graphics_cleanup = graphics.map(FrameGraphicsCleanup);
        // This entry point owns the process's stdout, so it is the one that may
        // emit the image layer and the OSC 52 copy after each frame.
        let result = self.drive(
            &mut terminal,
            theme,
            frames,
            &mut CrosstermEvents,
            graphics,
            |copied| {
                if let Some(graphics) = graphics {
                    graphics.finish_frame()?;
                }
                if let Some(text) = copied {
                    let _ = clipboard::write(&mut io::stdout(), &text)?;
                }
                Ok(())
            },
        );

        // Some terminal emulators do not answer the cursor-position query used
        // by `clear`. Session restoration must still succeed and a cosmetic
        // cleanup failure must not turn a completed run into an application
        // error.
        if split {
            let _ = close_footer(&mut terminal);
        } else {
            let _ = terminal.clear();
        }
        result
    }

    /// The loop shared by every entry point: no session, no terminal
    /// construction, no cleanup — those belong to whoever owns the terminal.
    fn drive<F, B, E, Er, Finish>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        frames: &mut F,
        events: &mut E,
        graphics: Option<&FrameGraphics>,
        mut finish_frame: Finish,
    ) -> Result<(), Er>
    where
        F: FrameSource,
        B: Backend<Error = Er>,
        E: EventSource<Er>,
        Finish: FnMut(Option<String>) -> Result<(), Er>,
    {
        let split = !self.config.screen_mode.is_alternate();
        let mut core = RunnerCore::new();
        let mut selection = RunnerSelection::new(self.selects_text());
        let mut last_tick = self.clock.now();

        if split {
            // Same order as every later pin: the terminal may already have been
            // resized between its construction and this first frame, and
            // `pin_footer` inserts rows at the geometry the terminal last knew.
            terminal.autoresize()?;
            pin_footer(terminal)?;
        }
        if let RunnerAction::Render(frame) = core.next_action() {
            let copied = draw(terminal, theme, frames, frame, graphics, &mut selection)?;
            finish_frame(copied)?;
        }
        let mut last_frame = self.clock.now();
        let mut redraw_at = None;

        'running: loop {
            let now = self.clock.now();
            if self.redraw.take() {
                core.request_redraw();
                schedule_redraw(&mut redraw_at, now);
            }
            if split {
                // A block is rendered and inserted at the width the terminal
                // last knew, so an unobserved resize would publish it at the
                // old one. Learn the size before committing, not after.
                terminal.autoresize()?;
                // Publishing scrolls the terminal and may clear the viewport,
                // so a committed block always makes the footer dirty.
                if self.scrollback.flush(terminal, theme)? {
                    core.request_redraw();
                    schedule_redraw(&mut redraw_at, now);
                }
            } else {
                self.scrollback.clear();
            }

            if now.saturating_duration_since(last_tick) >= self.config.tick_rate {
                last_tick = now;
                let result = frames.update(Signal::Tick);
                core.apply(result);
                if core.is_exited() {
                    break;
                }
                if result == UpdateResult::Dirty {
                    schedule_redraw(&mut redraw_at, now);
                }
            }

            if redraw_at.is_some_and(|deadline| deadline <= now) {
                if let RunnerAction::Render(frame) = core.next_action() {
                    if split {
                        terminal.autoresize()?;
                        pin_footer(terminal)?;
                    }
                    let copied = draw(terminal, theme, frames, frame, graphics, &mut selection)?;
                    finish_frame(copied)?;
                    last_frame = self.clock.now();
                }
                redraw_at = None;
            }

            let elapsed = self.clock.now().saturating_duration_since(last_tick);
            let tick_timeout = self.config.tick_rate.saturating_sub(elapsed);
            let redraw_timeout = redraw_at.map_or(tick_timeout, |deadline| {
                deadline.saturating_duration_since(self.clock.now())
            });
            let timeout = tick_timeout.min(redraw_timeout);
            if let Some(event) = events.poll_event(timeout)? {
                let signal = Signal::Event(event);
                let requires_redraw = signal.requires_redraw();
                let selection_event = match &signal {
                    Signal::Event(event) => Some(event.clone()),
                    _ => None,
                };
                let result = frames.update(signal);
                core.apply(result);
                if core.is_exited() {
                    break 'running;
                }
                let selection_changed = selection_event.is_some_and(|event| {
                    selection.handle_event(&event, result, self.clock.as_ref())
                });
                if requires_redraw || result == UpdateResult::Dirty || selection_changed {
                    core.request_redraw();
                    let now = self.clock.now();
                    let deadline = if requires_redraw {
                        resize_redraw_at(last_frame, now)
                    } else {
                        now
                    };
                    schedule_redraw(&mut redraw_at, deadline);
                }
            }
        }

        Ok(())
    }
}

/// Paint one numbered frame from immutable state.
///
/// Emitting the image layer and the OSC 52 copy is deliberately *not* done
/// here: both target the process's own stdout, which only an entry point that
/// owns the terminal may write to. `draw` reports the copied text and lets the
/// caller decide.
fn draw<F, B, Er>(
    terminal: &mut Terminal<B>,
    theme: &Theme,
    frames: &mut F,
    frame: u64,
    graphics: Option<&FrameGraphics>,
    selection: &mut RunnerSelection,
) -> Result<Option<String>, Er>
where
    B: Backend<Error = Er>,
    F: FrameSource,
{
    let mut copied = None;
    let selection_frame = SelectionFrame::default();
    terminal.draw(|terminal_frame| {
        let area = terminal_frame.area();
        let ctx = graphics.map_or_else(|| RenderCtx::new(theme), |g| g.render_context(theme));
        frames.frame(frame, &mut |root| {
            paint_with_context_and_selection(
                terminal_frame.buffer_mut(),
                area,
                &ctx,
                root,
                &[],
                &selection_frame,
            );
        });
        copied = selection.finish_frame(terminal_frame.buffer_mut(), area, theme, &selection_frame);
    })?;
    Ok(copied)
}

struct RunnerSelection {
    enabled: bool,
    state: SelectionState,
    pending_copy: bool,
    /// Panel regions from the last painted frame, resolved on the next `Down`.
    regions: Vec<Rect>,
}

impl RunnerSelection {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            state: SelectionState::new(),
            pending_copy: false,
            regions: Vec::new(),
        }
    }

    fn handle_event(&mut self, event: &Event, result: UpdateResult, clock: &dyn Clock) -> bool {
        if !self.enabled {
            return false;
        }
        let Event::Mouse(mouse) = event else {
            if matches!(event, Event::Resize { .. }) {
                return self.clear();
            }
            return false;
        };
        if !mouse.plain() {
            return false;
        }

        if matches!(
            mouse.kind,
            crate::MouseKind::ScrollUp
                | crate::MouseKind::ScrollDown
                | crate::MouseKind::ScrollLeft
                | crate::MouseKind::ScrollRight
        ) && result == UpdateResult::Dirty
        {
            return self.clear();
        }

        let left_gesture = matches!(
            mouse.kind,
            crate::MouseKind::Down(crate::MouseButton::Left)
                | crate::MouseKind::Drag(crate::MouseButton::Left)
                | crate::MouseKind::Up(crate::MouseButton::Left)
        );
        if !left_gesture {
            return false;
        }
        if result != UpdateResult::Clean {
            return self.clear();
        }

        if matches!(mouse.kind, crate::MouseKind::Down(crate::MouseButton::Left)) {
            // A gesture belongs to the panel it starts in, for its whole life.
            self.state.confine(crate::mouse::region_at(
                &self.regions,
                mouse.column,
                mouse.row,
            ));
        }
        let changed = self.state.handle_with_clock(mouse, clock);
        if changed
            && matches!(mouse.kind, crate::MouseKind::Up(crate::MouseButton::Left))
            && self.state.range().is_some()
        {
            self.pending_copy = true;
        }
        changed
    }

    fn finish_frame(
        &mut self,
        buffer: &mut Buffer,
        area: Rect,
        theme: &Theme,
        frame: &crate::view::SelectionFrame,
    ) -> Option<String> {
        if !self.enabled {
            return None;
        }
        self.regions = frame.take_regions();
        let sources = frame.take_sources();
        // A confined gesture reads, wraps, and paints inside its panel. The
        // region was recorded by the previous frame, so intersect it with this
        // one: a layout change between the press and this paint must not point
        // the selection at geometry that no longer exists.
        let area = self
            .state
            .region()
            .map(|panel| panel.intersection(area))
            .filter(|panel| panel.width > 0 && panel.height > 0)
            .unwrap_or(area);
        if self.state.resolve(buffer, area) {
            self.pending_copy = true;
        }
        let Some(range) = self.state.range() else {
            self.pending_copy = false;
            return None;
        };
        let copied = self
            .pending_copy
            .then(|| selected_text_from_sources(buffer, area, range, &sources));
        self.pending_copy = false;
        paint_selection(buffer, area, range, theme.selection_style());
        copied
    }

    fn clear(&mut self) -> bool {
        let changed = self.state.is_active();
        self.state.clear();
        self.pending_copy = false;
        changed
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[derive(Clone, Copy)]
    struct FixedClock(Instant);

    impl Clock for FixedClock {
        fn now(&self) -> Instant {
            self.0
        }
    }

    #[test]
    fn zero_tick_rate_is_clamped() {
        let runner = Runner::new(RunnerConfig {
            tick_rate: Duration::ZERO,
            ..RunnerConfig::default()
        });
        assert_eq!(runner.config.tick_rate, Duration::from_millis(1));
    }

    #[test]
    fn explicit_clock_drives_the_runner() {
        let now = Instant::now();
        let runner = Runner::with_clock(RunnerConfig::default(), FixedClock(now));
        assert_eq!(runner.clock.now(), now);
    }

    #[test]
    fn resize_redraws_are_limited_to_one_frame_interval() {
        let last_frame = Instant::now();
        let during_frame = last_frame + Duration::from_millis(4);
        let after_frame = last_frame + Duration::from_millis(20);

        assert_eq!(
            resize_redraw_at(last_frame, during_frame),
            last_frame + RESIZE_FRAME_INTERVAL
        );
        assert_eq!(resize_redraw_at(last_frame, after_frame), after_frame);
    }

    #[test]
    fn finishing_a_graphics_frame_emits_and_clears_placements() {
        let graphics = FrameGraphics {
            support: ImageSupport::Kitty,
            layer: ImageLayer::new(),
        };
        let data = crate::term::image::ImageData::from_rgba(1, 1, vec![1, 2, 3, 255]).unwrap();
        graphics.layer.record(
            crate::geometry::Rect::new(2, 3, 4, 5),
            data,
            graphics.support,
        );
        let mut output = Vec::new();

        graphics.finish_frame_to(&mut output).unwrap();

        assert!(!output.is_empty());
        assert!(graphics.layer.is_empty());
    }

    #[test]
    fn runner_core_is_deterministic_and_runtime_free() {
        let mut core = RunnerCore::new();
        assert_eq!(core.next_action(), RunnerAction::Render(0));
        assert_eq!(core.next_action(), RunnerAction::Wait);
        core.apply(UpdateResult::Dirty);
        assert_eq!(core.next_action(), RunnerAction::Render(1));
        core.apply(UpdateResult::Consumed);
        assert_eq!(core.next_action(), RunnerAction::Wait);
        core.apply(UpdateResult::Exit);
        assert_eq!(core.next_action(), RunnerAction::Exit);
    }

    #[test]
    fn default_selection_highlights_and_returns_dragged_text() {
        let clock = FixedClock(Instant::now());
        let mut selection = RunnerSelection::new(true);
        let mut buffer =
            crate::testing::render(&Text::raw("hello world"), 11, 1, &Theme::default());
        let area = buffer.area;
        let down = crate::Mouse::at(crate::MouseKind::Down(crate::MouseButton::Left), 0, 0);
        let drag = crate::Mouse::at(crate::MouseKind::Drag(crate::MouseButton::Left), 4, 0);
        let up = crate::Mouse::at(crate::MouseKind::Up(crate::MouseButton::Left), 4, 0);

        assert!(!selection.handle_event(&Event::Mouse(down), UpdateResult::Clean, &clock));
        assert!(selection.handle_event(&Event::Mouse(drag), UpdateResult::Clean, &clock));
        assert!(selection.handle_event(&Event::Mouse(up), UpdateResult::Clean, &clock));

        let copied = selection.finish_frame(
            &mut buffer,
            area,
            &Theme::default(),
            &crate::view::SelectionFrame::default(),
        );
        assert_eq!(copied.as_deref(), Some("hello"));
        for column in 0..=4 {
            assert_eq!(buffer[(column, 0)].bg, Theme::default().selection_bg);
        }
    }

    /// Two bordered panels side by side, each with its own text.
    struct TwoPanels;

    impl crate::View for TwoPanels {
        fn measure(&self, available: crate::Size, _ctx: &crate::RenderCtx) -> crate::Size {
            available
        }

        fn render(&self, _area: Rect, surface: &mut crate::Surface, ctx: &crate::RenderCtx) {
            for (rect, label) in [
                (Rect::new(0, 0, 10, 3), "alpha"),
                (Rect::new(10, 0, 10, 3), "bravo"),
            ] {
                let panel = crate::components::Boxed::new(crate::element(Text::raw(label)));
                let mut child = surface.child(rect);
                crate::View::render(&panel, rect, &mut child, ctx);
            }
        }
    }

    fn paint_panels(selection: &mut RunnerSelection, theme: &Theme) -> (Buffer, Option<String>) {
        let area = Rect::new(0, 0, 20, 3);
        let mut buffer = Buffer::empty(area);
        let frame = crate::view::SelectionFrame::default();
        crate::host::paint_with_context_and_selection(
            &mut buffer,
            area,
            &crate::RenderCtx::new(theme),
            &TwoPanels,
            &[],
            &frame,
        );
        let copied = selection.finish_frame(&mut buffer, area, theme, &frame);
        (buffer, copied)
    }

    #[test]
    fn selection_stays_inside_the_panel_it_started_in() {
        let clock = FixedClock(Instant::now());
        let theme = Theme::default();
        let mut selection = RunnerSelection::new(true);
        // A first frame publishes the panels' regions.
        paint_panels(&mut selection, &theme);

        // Press inside the left panel, drag deep into the right one.
        for mouse in [
            crate::Mouse::at(crate::MouseKind::Down(crate::MouseButton::Left), 2, 1),
            crate::Mouse::at(crate::MouseKind::Drag(crate::MouseButton::Left), 16, 1),
            crate::Mouse::at(crate::MouseKind::Up(crate::MouseButton::Left), 16, 1),
        ] {
            selection.handle_event(&Event::Mouse(mouse), UpdateResult::Clean, &clock);
        }

        let (buffer, copied) = paint_panels(&mut selection, &theme);
        assert_eq!(copied.as_deref(), Some("alpha"));
        for column in 2..8 {
            assert_eq!(
                buffer[(column, 1)].bg,
                theme.selection_bg,
                "left panel cell {column} is selected"
            );
        }
        for column in 8..20 {
            assert_ne!(
                buffer[(column, 1)].bg,
                theme.selection_bg,
                "cell {column} outside the panel is untouched"
            );
        }
    }

    #[test]
    fn selection_outside_every_panel_still_spans_the_screen() {
        let clock = FixedClock(Instant::now());
        let theme = Theme::default();
        let mut selection = RunnerSelection::new(true);
        paint_panels(&mut selection, &theme);

        // Row 0 is the panels' borders — no region covers it.
        for mouse in [
            crate::Mouse::at(crate::MouseKind::Down(crate::MouseButton::Left), 1, 0),
            crate::Mouse::at(crate::MouseKind::Drag(crate::MouseButton::Left), 15, 0),
            crate::Mouse::at(crate::MouseKind::Up(crate::MouseButton::Left), 15, 0),
        ] {
            selection.handle_event(&Event::Mouse(mouse), UpdateResult::Clean, &clock);
        }

        assert!(selection.state.region().is_none());
        let (buffer, _) = paint_panels(&mut selection, &theme);
        assert_eq!(buffer[(15, 0)].bg, theme.selection_bg);
    }

    #[test]
    fn consumed_mouse_gesture_is_not_selected() {
        let clock = FixedClock(Instant::now());
        let mut selection = RunnerSelection::new(true);
        let down = crate::Mouse::at(crate::MouseKind::Down(crate::MouseButton::Left), 0, 0);
        let drag = crate::Mouse::at(crate::MouseKind::Drag(crate::MouseButton::Left), 4, 0);

        assert!(!selection.handle_event(&Event::Mouse(down), UpdateResult::Consumed, &clock,));
        assert!(!selection.handle_event(&Event::Mouse(drag), UpdateResult::Consumed, &clock,));
        assert!(selection.state.range().is_none());
    }

    #[test]
    fn the_default_config_owns_the_alternate_screen() {
        assert_eq!(RunnerConfig::default().screen_mode, ScreenMode::Alternate);
    }

    #[test]
    fn text_selection_follows_capture_and_can_be_disabled() {
        assert!(!Runner::new(RunnerConfig::default()).selects_text());
        assert!(
            Runner::new(RunnerConfig {
                screen_mode: ScreenMode::Alternate.with_mouse_capture(),
                ..RunnerConfig::default()
            })
            .selects_text()
        );
        assert!(
            !Runner::new(RunnerConfig {
                screen_mode: ScreenMode::split_footer(3),
                ..RunnerConfig::default()
            })
            .selects_text()
        );
        assert!(
            !Runner::new(RunnerConfig::default())
                .with_text_selection(false)
                .selects_text()
        );
        assert!(
            !Runner::new(RunnerConfig::default())
                .with_session_config(crate::TerminalSessionConfig {
                    mouse_capture: crate::MouseCapture::Disabled,
                    ..crate::TerminalSessionConfig::default()
                })
                .selects_text()
        );
    }

    #[test]
    fn the_scrollback_handle_shares_one_queue() {
        let runner = Runner::new(RunnerConfig {
            screen_mode: ScreenMode::split_footer(4),
            ..RunnerConfig::default()
        });
        let handle = runner.scrollback();
        assert!(handle.is_empty());
        handle.write(|_width| crate::element(Text::raw("queued")));
        assert!(
            !runner.scrollback().is_empty(),
            "every handle sees the same queue"
        );
    }

    // ---- End-to-end loop coverage, via `run_driven_by`. ----
    //
    // Before this boundary existed the synchronous loop had no test at all: every
    // entry point reached crossterm and a real terminal, so only its pieces
    // (`RunnerCore`, the clock, `RunnerSelection`) could be exercised. These
    // drive the whole thing over a `TestBackend`.

    use crate::term::testbackend::TestBackend;

    use crate::components::Text;
    use crate::event::KeyCode;
    use crate::view::element;

    fn key_event(code: KeyCode) -> Event {
        Event::Key(crate::event::Key::new(code))
    }

    fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(width, height)).expect("test terminal")
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(crate::buffer::Cell::symbol)
            .collect()
    }

    /// A quiet runner: the tick timer is far enough out that only scripted
    /// events drive these tests.
    fn quiet_runner() -> Runner {
        Runner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        })
    }

    #[test]
    fn the_initial_frame_paints_before_any_event() {
        let runner = quiet_runner();
        let mut terminal = terminal(12, 1);
        let mut state = ();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut state,
                    |(), _frame| element(Text::raw("ready")),
                    |(), signal| match signal {
                        Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                        _ => UpdateResult::Clean,
                    },
                ),
                scripted_events([key_event(KeyCode::Esc)]),
            )
            .expect("run");

        assert!(
            buffer_text(&terminal).contains("ready"),
            "a frame is painted before the first signal: {:?}",
            buffer_text(&terminal)
        );
    }

    #[test]
    fn events_drive_state_and_exit_ends_the_run() {
        let runner = quiet_runner();
        let mut terminal = terminal(12, 1);
        let mut count = 0u32;

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut count,
                    |count, _frame| element(Text::raw(format!("n={count}"))),
                    |count, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        Signal::Event(Event::Key(_)) => {
                            *count += 1;
                            UpdateResult::Dirty
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                scripted_events([
                    key_event(KeyCode::Char('a')),
                    key_event(KeyCode::Char('a')),
                    key_event(KeyCode::Esc),
                ]),
            )
            .expect("run");

        assert_eq!(count, 2, "both keys counted, Esc exited");
        assert!(buffer_text(&terminal).contains("n=2"));
    }

    #[test]
    fn clean_updates_neither_rebuild_nor_repaint() {
        let runner = quiet_runner();
        let mut terminal = terminal(12, 1);
        let views = std::cell::Cell::new(0usize);
        let mut state = ();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut state,
                    |(), _frame| {
                        views.set(views.get() + 1);
                        element(Text::raw("idle"))
                    },
                    |(), signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                scripted_events([key_event(KeyCode::Char('a')), key_event(KeyCode::Esc)]),
            )
            .expect("run");

        assert_eq!(views.get(), 1, "only the initial frame was built");
    }

    /// The borrowed boundary over the synchronous loop: no clone into an owned tree.
    struct Notes {
        title: String,
        hits: Vec<char>,
    }

    struct NotesView<'app> {
        title: &'app str,
        hits: &'app [char],
    }

    impl View for NotesView<'_> {
        fn measure(&self, available: crate::Size, _ctx: &RenderCtx) -> crate::Size {
            crate::Size::new(available.width, 1.min(available.height))
        }

        fn render(&self, area: Rect, surface: &mut crate::Surface, ctx: &RenderCtx) {
            let line = format!("{}:{}", self.title, self.hits.iter().collect::<String>());
            surface.set_string(area.x, area.y, &line, ctx.theme.text_style());
        }
    }

    impl Application for Notes {
        fn update(&mut self, signal: Signal) -> UpdateResult {
            match signal {
                Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => UpdateResult::Exit,
                Signal::Event(Event::Key(crate::event::Key {
                    code: KeyCode::Char(c),
                    ..
                })) => {
                    self.hits.push(c);
                    UpdateResult::Dirty
                }
                _ => UpdateResult::Clean,
            }
        }

        fn view(&self, _frame: u64) -> ScopedElement<'_> {
            element(NotesView {
                title: &self.title,
                hits: &self.hits,
            })
        }
    }

    #[test]
    fn an_application_paints_a_borrowed_view() {
        let runner = quiet_runner();
        let mut terminal = terminal(12, 1);
        let mut app = Notes {
            title: "n".into(),
            hits: Vec::new(),
        };

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                &mut app,
                scripted_events([
                    key_event(KeyCode::Char('a')),
                    key_event(KeyCode::Char('b')),
                    key_event(KeyCode::Esc),
                ]),
            )
            .expect("run");

        assert_eq!(app.hits, vec!['a', 'b']);
        assert!(
            buffer_text(&terminal).contains("n:ab"),
            "the frame borrows application data: {:?}",
            buffer_text(&terminal)
        );
    }

    #[test]
    fn a_redraw_request_repaints_without_an_event() {
        let runner = quiet_runner();
        let mut terminal = terminal(12, 1);
        let redraw = runner.redraw_handle();
        let value = std::cell::Cell::new(0u32);
        let views = std::cell::Cell::new(0usize);
        let mut state = ();

        // A background producer's write, landing before the loop starts: the
        // handle is a flag, so when it is set does not matter, only that the
        // loop notices it without an event of its own.
        value.set(41);
        redraw.request();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut state,
                    |(), _frame| {
                        views.set(views.get() + 1);
                        element(Text::raw(format!("v={}", value.get())))
                    },
                    |(), signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                scripted_events([key_event(KeyCode::Esc)]),
            )
            .expect("run");

        assert_eq!(views.get(), 2, "initial frame plus the requested repaint");
        assert!(buffer_text(&terminal).contains("v=41"));
    }

    /// A script that goes quiet for one round between events, the way a real
    /// terminal is quiet between keystrokes.
    ///
    /// [`scripted_events`] delivers back-to-back, which is what a test usually
    /// wants; a repaint the loop *defers* needs an idle gap to land in, and
    /// waiting out the loop's own timeout is what creates one. Implementing
    /// [`EventSource`] for that is the intended escape hatch.
    struct QuietBetween<I> {
        events: I,
        idle_next: bool,
    }

    impl<I: Iterator<Item = Event>, Er> EventSource<Er> for QuietBetween<I> {
        fn poll_event(&mut self, timeout: Duration) -> Result<Option<Event>, Er> {
            if self.idle_next {
                self.idle_next = false;
                std::thread::sleep(timeout);
                return Ok(None);
            }
            match self.events.next() {
                Some(event) => {
                    self.idle_next = true;
                    Ok(Some(event))
                }
                None => {
                    std::thread::sleep(timeout);
                    Ok(None)
                }
            }
        }
    }

    #[test]
    fn a_resize_repaints_even_when_the_update_stays_clean() {
        let runner = quiet_runner();
        let mut terminal = terminal(12, 1);
        let views = std::cell::Cell::new(0usize);
        let mut state = ();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut state,
                    |(), _frame| {
                        views.set(views.get() + 1);
                        element(Text::raw("sized"))
                    },
                    |(), signal| match signal {
                        Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                        // Deliberately clean: layout must still be repainted.
                        _ => UpdateResult::Clean,
                    },
                ),
                // The idle round after the resize waits out exactly the loop's
                // own redraw deadline, so the deferred frame lands before the
                // quit key regardless of how loaded the machine is.
                QuietBetween {
                    events: [
                        Event::Resize {
                            width: 12,
                            height: 1,
                        },
                        key_event(KeyCode::Esc),
                    ]
                    .into_iter(),
                    idle_next: false,
                },
            )
            .expect("run");

        assert_eq!(
            views.get(),
            2,
            "the initial frame plus one forced by the resize, despite a clean update"
        );
    }

    #[test]
    fn a_split_footer_publishes_above_the_pinned_footer() {
        use crate::geometry::Position;

        let runner = Runner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            screen_mode: ScreenMode::split_footer(2),
        });
        let scrollback = runner.scrollback();
        let mut backend = TestBackend::new(12, 6);
        backend
            .set_cursor_position(Position::new(0, 0))
            .expect("place cursor");
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: runner.config.screen_mode.viewport(),
            },
        )
        .expect("inline terminal");
        let mut state = ();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut state,
                    |(), _frame| {
                        element(Text::new(vec![
                            crate::text::Line::from("FOOTER"),
                            crate::text::Line::from("FOOTER"),
                        ]))
                    },
                    |(), signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Char('q') => {
                            UpdateResult::Exit
                        }
                        Signal::Event(_) => {
                            scrollback.write(|_width| element(Text::raw("published")));
                            UpdateResult::Dirty
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                scripted_events([key_event(KeyCode::Char('a')), key_event(KeyCode::Char('q'))]),
            )
            .expect("run");

        let buffer = terminal.backend().buffer();
        let lines: Vec<String> = (0..6)
            .map(|y| crate::tests::support::row(buffer, y))
            .collect();
        assert_eq!(
            &lines[3..],
            &["published", "FOOTER", "FOOTER"],
            "the block sits directly above the repainted footer: {lines:?}"
        );
    }
}
