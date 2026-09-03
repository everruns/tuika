//! An asynchronous full-screen runner (`feature = "async"`).
//!
//! [`AsyncRunner`] ties tuika's public host primitives — [`TerminalSession`],
//! [`paint`], [`translate_event`] — to crossterm's async
//! [`EventStream`] and a tick timer in a single
//! `tokio::select!` loop. An application that already has a Tokio runtime (it is
//! doing network or disk I/O) gets lifecycle, redraw scheduling, and event
//! translation in one place, and its data becomes a plain local value the loop
//! owns — no `spawn_blocking`, no shared `RwLock`/`Notify`/stop flag bolted onto
//! the synchronous [`Runner`](crate::Runner) just to feed it.
//!
//! The loop pulls every frame and update decision from one
//! [`AsyncFrameSource`]: either `&mut app` for an [`AsyncApplication`], or
//! [`async_from_fn`] over a state value and a pair of closures. In the closure
//! form `view` builds the frame from `&state` while `update` mutates
//! `&mut state` and may `.await`; state is an argument rather than a capture
//! because a closure cannot hold `&state` and `&mut state` at once.
//!
//! ```no_run
//! use std::time::Duration;
//! use tuika::prelude::*;
//!
//! # async fn fetch_stats() -> std::io::Result<u64> { Ok(0) }
//! // Call from inside your own `#[tokio::main]` (or any Tokio runtime).
//! async fn dashboard() -> std::io::Result<()> {
//!     let runner = AsyncRunner::new(RunnerConfig {
//!         tick_rate: Duration::from_secs(2),
//!         ..RunnerConfig::default()
//!     });
//!     let mut requests = 0u64;
//!
//!     runner
//!         .run(
//!             &Theme::default(),
//!             async_from_fn(
//!                 &mut requests,
//!                 |requests, _frame| element(Text::raw(format!("requests: {requests}"))),
//!                 async |requests, signal| match signal {
//!                     // Poll on every tick and on `r`; both may await.
//!                     Signal::Tick => {
//!                         *requests = fetch_stats().await.unwrap_or(*requests);
//!                         UpdateResult::Dirty
//!                     }
//!                     Signal::Event(Event::Key(k)) if k.plain() => match k.code {
//!                         KeyCode::Char('q') | KeyCode::Esc => UpdateResult::Exit,
//!                         KeyCode::Char('r') => {
//!                             *requests = fetch_stats().await.unwrap_or(*requests);
//!                             UpdateResult::Dirty
//!                         }
//!                         _ => UpdateResult::Clean,
//!                     },
//!                     _ => UpdateResult::Clean,
//!                 },
//!             ),
//!         )
//!         .await
//! }
//! ```

use std::convert::Infallible;
use std::io;
use std::time::Duration;

use super::stream::{self, Stream};
use crate::term::backend::CrosstermBackend;
use crate::term::terminal::{Terminal, TerminalOptions};
use crate::term::traits::Backend;
use crossterm::event::EventStream;
use tokio::time::{Instant as TokioInstant, MissedTickBehavior, interval, sleep_until};

use super::{
    FrameGraphics, FrameGraphicsCleanup, PaintRoot, RESIZE_FRAME_INTERVAL, RunnerAction,
    RunnerCore, RunnerSelection, Signal,
};
use crate::host::paint_with_context_and_selection;
use crate::live::RedrawHandle;
use crate::screen::{Scrollback, close_footer, pin_footer};
use crate::view::{ScopedElement, SelectionFrame};
use crate::{
    Element, Event, RenderCtx, RunnerConfig, SystemClock, TerminalSession, Theme, UpdateResult,
    translate_event,
};

/// The signal type an [`AsyncRunner`] delivers when it also listens to a typed
/// application message stream.
#[deprecated(since = "0.9.0", note = "use `Signal<M>`, which now carries messages")]
pub type AsyncSignal<M> = Signal<M>;

/// A data-driven asynchronous terminal application.
///
/// The asynchronous counterpart to [`Application`](crate::runner::Application),
/// with the same contract: the runner mutably borrows the application to deliver
/// a signal, then immutably borrows it to build the next frame, so a frame can
/// read application data directly instead of cloning it into an owned
/// [`Element`] or sharing it through `Rc<RefCell<_>>`. Only `update` may
/// `.await`; rendering stays pure and synchronous.
///
/// `M` is the application-message type, and defaults to the uninhabited
/// [`Infallible`] — so an application that consumes a message stream is this
/// same trait at `AsyncApplication<MyMessage>`, not a different one.
///
/// ```no_run
/// use tuika::prelude::*;
///
/// struct Counter {
///     hits: Vec<String>,
/// }
///
/// impl AsyncApplication for Counter {
///     async fn update(&mut self, signal: Signal) -> UpdateResult {
///         match signal {
///             Signal::Event(Event::Key(key)) if key.code == KeyCode::Esc => UpdateResult::Exit,
///             Signal::Event(Event::Key(_)) => {
///                 self.hits.push("hit".into());
///                 UpdateResult::Dirty
///             }
///             _ => UpdateResult::Clean,
///         }
///     }
///
///     // The frame borrows `self.hits` — no clone, no `'static` bound.
///     fn view(&self, _frame: u64) -> ScopedElement<'_> {
///         element(Text::raw(format!("{} hits", self.hits.len())))
///     }
/// }
/// ```
pub trait AsyncApplication<M = Infallible> {
    /// Update application state in response to a signal, possibly awaiting.
    fn update(&mut self, signal: Signal<M>) -> impl Future<Output = UpdateResult>;

    /// Build the ephemeral view tree for one numbered frame.
    fn view(&self, frame: u64) -> ScopedElement<'_>;
}

/// What an [`AsyncRunner`] pulls frames and update decisions from.
///
/// The asynchronous [`FrameSource`](crate::runner::FrameSource): `&mut app` for
/// an [`AsyncApplication`], or [`async_from_fn`] for the closure form.
/// Implement [`AsyncApplication`] rather than this trait.
pub trait AsyncFrameSource<M = Infallible> {
    /// Deliver one signal, possibly awaiting.
    fn update(&mut self, signal: Signal<M>) -> impl Future<Output = UpdateResult>;

    /// Build one numbered frame and hand its root to `paint`.
    fn frame(&mut self, frame: u64, paint: PaintRoot<'_>);
}

impl<M, A: AsyncApplication<M>> AsyncFrameSource<M> for &mut A {
    async fn update(&mut self, signal: Signal<M>) -> UpdateResult {
        AsyncApplication::update(*self, signal).await
    }

    fn frame(&mut self, frame: u64, paint: PaintRoot<'_>) {
        paint(AsyncApplication::view(*self, frame).as_ref());
    }
}

/// An [`AsyncFrameSource`] built from a state value and a pair of closures.
///
/// Returned by [`async_from_fn`].
pub struct AsyncFromFn<'state, S, V, U> {
    state: &'state mut S,
    view: V,
    update: U,
}

/// Build an [`AsyncFrameSource`] from `state` and closures over it.
///
/// The asynchronous [`from_fn`](crate::runner::from_fn): `view` stays a
/// synchronous `FnMut` returning an owned [`Element`], while `update` may
/// `.await`.
pub fn async_from_fn<S, V, U, M>(state: &mut S, view: V, update: U) -> AsyncFromFn<'_, S, V, U>
where
    V: FnMut(&S, u64) -> Element,
    U: AsyncFnMut(&mut S, Signal<M>) -> UpdateResult,
{
    AsyncFromFn {
        state,
        view,
        update,
    }
}

impl<M, S, V, U> AsyncFrameSource<M> for AsyncFromFn<'_, S, V, U>
where
    V: FnMut(&S, u64) -> Element,
    U: AsyncFnMut(&mut S, Signal<M>) -> UpdateResult,
{
    async fn update(&mut self, signal: Signal<M>) -> UpdateResult {
        (self.update)(&mut *self.state, signal).await
    }

    fn frame(&mut self, frame: u64, paint: PaintRoot<'_>) {
        paint((self.view)(self.state, frame).as_ref());
    }
}

/// An asynchronous Crossterm event and rendering loop.
///
/// See the [module documentation](crate::runner) for the full picture,
/// including how the `run*` names are composed from the loop's axes. Construct
/// one with a [`RunnerConfig`], then pick a method by what you are departing
/// from the default on: `_app` for an [`AsyncApplication`] whose frame borrows
/// itself instead of a view closure returning an owned tree,
/// `_with_backend` for a caller-supplied backend (such as
/// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend)),
/// `_with_events` for a caller-owned terminal *and* event stream (tests and
/// hosts that own the terminal lifecycle), and `_with_messages` /
/// `_and_messages` to select over a typed application stream as well.
pub struct AsyncRunner {
    config: RunnerConfig,
    redraw: RedrawHandle,
    scrollback: Scrollback,
    session_config: Option<crate::TerminalSessionConfig>,
    text_selection: bool,
}

impl AsyncRunner {
    /// Create a runner without touching the terminal.
    pub fn new(mut config: RunnerConfig) -> Self {
        // A zero interval would busy-spin the `select!` even when the app has no
        // events or updates. Enforce the same scheduling floor the synchronous
        // `Runner` does.
        config.tick_rate = config.tick_rate.max(Duration::from_millis(1));
        Self {
            config,
            redraw: RedrawHandle::default(),
            scrollback: Scrollback::new(),
            session_config: None,
            text_selection: true,
        }
    }

    /// Return a handle that background producers can use to request redraws.
    ///
    /// The counterpart to [`Runner::redraw_handle`](crate::Runner::redraw_handle),
    /// so a [`Live`](crate::live::Live) value works the same on either runner —
    /// here it wakes the parked `select!` rather than waiting for the next tick.
    ///
    /// A request says only *the next frame is stale*, and requests coalesce: the
    /// handle is a flag, so a burst produces one repaint. That is the right
    /// primitive when the producer owns data the view re-reads each frame. When
    /// each arrival instead needs `update` to decide something — mutate specific
    /// state, open a dialog, exit — send it as a message and let the runner
    /// deliver it: see [`run_with_messages`](Self::run_with_messages).
    pub fn redraw_handle(&self) -> RedrawHandle {
        self.redraw.clone()
    }

    /// Override terminal lifecycle policy while retaining the async loop.
    pub fn with_session_config(mut self, config: crate::TerminalSessionConfig) -> Self {
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
                crate::TerminalSessionConfig::captures_mouse,
            )
    }

    /// Return a handle for publishing content above a
    /// [`ScreenMode::SplitFooter`](crate::ScreenMode::SplitFooter) — see
    /// [`Scrollback`]. Blocks queued while running in
    /// [`ScreenMode::Alternate`](crate::ScreenMode::Alternate) are discarded,
    /// since there is no scrollback of the host's to write into.
    pub fn scrollback(&self) -> Scrollback {
        self.scrollback.clone()
    }

    /// Run on the real terminal until the frame source returns
    /// [`UpdateResult::Exit`].
    ///
    /// `frames` is either `&mut app` for an [`AsyncApplication`] or
    /// [`async_from_fn`] over a state value and closures. Enters a
    /// [`TerminalSession`] (restored on return, including on error or panic) and
    /// reads input from crossterm's async [`EventStream`].
    pub async fn run<F: AsyncFrameSource>(&self, theme: &Theme, frames: F) -> io::Result<()> {
        self.run_with_messages(theme, frames, no_messages()).await
    }

    /// Run on the real terminal, waking on values from the host's own program
    /// as well as on input.
    ///
    /// # When to use this
    ///
    /// Ask one question: **does `update` need to decide something about each
    /// arrival, or does the frame just need to re-read the newest value?**
    ///
    /// - *Re-read* — the producer owns a value the view reads every frame (a
    ///   gauge, a cached snapshot). Keep it in a [`Live`](crate::live::Live)
    ///   and mark the frame stale with [`redraw_handle`](Self::redraw_handle).
    /// - *Decide* — reach for this. Each item becomes one
    ///   [`Signal::Message`] and one `update` call, so it can mutate specific
    ///   state, open a dialog, or return [`UpdateResult::Exit`].
    ///
    /// The sharp difference: **redraw requests coalesce, messages do not.** A
    /// [`RedrawHandle`] is a flag, so five requests between frames make one
    /// repaint. Every message gets its own `update`, which is what lets each
    /// arrival mean something different.
    ///
    /// Typical producers:
    ///
    /// - a child process's stdout, where each line appends to a transcript and
    ///   some lines also change state
    /// - a model or agent token stream, where tokens feed a
    ///   [`MarkdownState`](crate::components::markdown::MarkdownState) and a
    ///   final `Done` re-enables input
    /// - a websocket or SSE feed — CI results, prices, chat
    /// - a worker reporting `Progress` / `Failed` / `Finished`, the last of
    ///   which may end the run
    /// - another task asking the UI to do something ("open settings", "quit"),
    ///   which is otherwise faked with a synthetic key event
    ///
    /// Do *not* use it to poll on a schedule; that is what [`Signal::Tick`] is
    /// for.
    ///
    /// # Mechanics
    ///
    /// The producer sends through any [`Stream`](futures_core::Stream) — a
    /// `tokio_stream` wrapper, a `futures` channel, anything implementing the
    /// trait — so no shared mutable
    /// state, synthetic terminal events, or short polling interval is needed.
    /// The frame source is the one [`run`](Self::run) takes, at `M` instead of
    /// the default [`Infallible`]. A completed message stream disables only
    /// that input — ticks and terminal events continue, so a finite producer
    /// does not end the UI. A stream error ends the run and is propagated after
    /// terminal cleanup.
    ///
    /// ```no_run
    /// # use tuika::prelude::*;
    /// # use tokio_stream::wrappers::ReceiverStream;
    /// # struct Tail { lines: Vec<String>, failed: bool }
    /// impl AsyncApplication<String> for Tail {
    ///     async fn update(&mut self, signal: Signal<String>) -> UpdateResult {
    ///         match signal {
    ///             // Every line is seen, and one of them means more than "repaint".
    ///             Signal::Message(line) => {
    ///                 self.failed |= line.starts_with("error:");
    ///                 self.lines.push(line);
    ///                 UpdateResult::Dirty
    ///             }
    ///             Signal::Event(Event::Key(key)) if key.code == KeyCode::Esc => UpdateResult::Exit,
    ///             _ => UpdateResult::Clean,
    ///         }
    ///     }
    ///
    ///     fn view(&self, _frame: u64) -> ScopedElement<'_> {
    ///         element(Text::raw(format!("{} lines", self.lines.len())))
    ///     }
    /// }
    ///
    /// # async fn run(runner: AsyncRunner, mut app: Tail,
    /// #     rx: tokio::sync::mpsc::Receiver<std::io::Result<String>>) -> std::io::Result<()> {
    /// runner
    ///     .run_with_messages(&Theme::default(), &mut app, ReceiverStream::new(rx))
    ///     .await
    /// # }
    /// ```
    pub async fn run_with_messages<M, F, MS>(
        &self,
        theme: &Theme,
        mut frames: F,
        messages: MS,
    ) -> io::Result<()>
    where
        F: AsyncFrameSource<M>,
        MS: Stream<Item = io::Result<M>> + Unpin,
    {
        let graphics = FrameGraphics::detected();
        let mode = self.config.screen_mode;
        let _session = if let Some(config) = self.session_config {
            TerminalSession::enter_config(config)?
        } else {
            TerminalSession::enter_with(mode)?
        };
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: mode.viewport(),
            },
        )?;
        let _graphics_cleanup = FrameGraphicsCleanup(&graphics);
        let events = stream::filter_map(EventStream::new(), |result| match result {
            Ok(raw) => translate_event(raw).map(Ok),
            Err(error) => Some(Err(error)),
        });
        let result = self
            .run_inner(
                &mut terminal,
                theme,
                &mut frames,
                events,
                messages,
                Some(&graphics),
                |copied| {
                    graphics.finish_frame()?;
                    if let Some(text) = copied {
                        let _ = crate::term::clipboard::write(&mut io::stdout(), text)?;
                    }
                    Ok(())
                },
            )
            .await;

        // Some terminal emulators do not answer the cursor-position query used
        // by `clear`; a cosmetic cleanup failure must not turn a completed run
        // into an application error. (Session restoration is the `_session`
        // guard's job and happens regardless.) A split footer gives its rows
        // back instead — the scrollback above it is the user's.
        if mode.is_alternate() {
            let _ = terminal.clear();
        } else {
            let _ = close_footer(&mut terminal);
        }
        result
    }

    /// Run with a caller-provided backend, such as
    /// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend). Otherwise
    /// identical to [`run`](Self::run): it owns the [`TerminalSession`] and the
    /// [`EventStream`].
    pub async fn run_with_backend<F, B>(
        &self,
        theme: &Theme,
        backend: B,
        mut frames: F,
    ) -> io::Result<()>
    where
        F: AsyncFrameSource,
        B: Backend<Error = io::Error>,
    {
        let mode = self.config.screen_mode;
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
        let events = stream::filter_map(EventStream::new(), |result| match result {
            Ok(raw) => translate_event(raw).map(Ok),
            Err(error) => Some(Err(error)),
        });
        let result = self
            .run_inner(
                &mut terminal,
                theme,
                &mut frames,
                events,
                no_messages(),
                None,
                |_| Ok(()),
            )
            .await;
        if mode.is_alternate() {
            let _ = terminal.clear();
        } else {
            let _ = close_footer(&mut terminal);
        }
        result
    }

    /// Run against a caller-owned terminal and input streams, with no terminal
    /// lifecycle of the runner's own.
    ///
    /// This is the loop itself, and what [`run`](Self::run) builds on. The boundary
    /// tests drive it against a
    /// [`TestBackend`](crate::term::testbackend::TestBackend) with scripted
    /// streams; hosts that already own their terminal and event source (or want
    /// a non-crossterm one) can call it directly. Pass [`no_messages`] when
    /// there is no application stream.
    ///
    /// The backend error and both stream errors share one type `Er`, which is
    /// the run's error type: for the real terminal that is [`io::Error`], but
    /// leaving it generic lets an infallible backend
    /// ([`TestBackend`](crate::term::testbackend::TestBackend), whose error is
    /// [`Infallible`]) pair with infallible streams.
    ///
    /// `events` yields already-translated tuika [`Event`]s. An `Err` item from
    /// either stream ends the run by propagating out; `None` (a finite stream
    /// running dry) stops that source but leaves the tick timer running, so the
    /// loop still exits only on [`UpdateResult::Exit`].
    pub async fn run_driven_by<M, F, B, E, MS, Er>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        mut frames: F,
        events: E,
        messages: MS,
    ) -> Result<(), Er>
    where
        F: AsyncFrameSource<M>,
        B: Backend<Error = Er>,
        E: Stream<Item = Result<Event, Er>> + Unpin,
        MS: Stream<Item = Result<M, Er>> + Unpin,
    {
        self.run_inner(terminal, theme, &mut frames, events, messages, None, |_| {
            Ok(())
        })
        .await
    }

    /// Run with caller-owned terminal and event stream.
    #[deprecated(
        since = "0.9.0",
        note = "use `run_driven_by(terminal, theme, async_from_fn(state, view, update), events, no_messages())`"
    )]
    pub async fn run_with_events<S, B, V, U, E, Er>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        state: &mut S,
        events: E,
        view: V,
        update: U,
    ) -> Result<(), Er>
    where
        B: Backend<Error = Er>,
        E: Stream<Item = Result<Event, Er>> + Unpin,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, Signal) -> UpdateResult,
    {
        self.run_driven_by(
            terminal,
            theme,
            async_from_fn(state, view, update),
            events,
            no_messages(),
        )
        .await
    }

    /// Run with caller-owned terminal, terminal-event stream, and typed
    /// application-message stream.
    #[deprecated(
        since = "0.9.0",
        note = "use `run_driven_by(terminal, theme, async_from_fn(state, view, update), events, messages)`"
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_events_and_messages<S, M, B, V, U, E, MS, Er>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        state: &mut S,
        events: E,
        messages: MS,
        view: V,
        update: U,
    ) -> Result<(), Er>
    where
        B: Backend<Error = Er>,
        E: Stream<Item = Result<Event, Er>> + Unpin,
        MS: Stream<Item = Result<M, Er>> + Unpin,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, Signal<M>) -> UpdateResult,
    {
        self.run_driven_by(
            terminal,
            theme,
            async_from_fn(state, view, update),
            events,
            messages,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_inner<M, S, B, E, MS, Er, Finish>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        frames: &mut S,
        mut events: E,
        mut messages: MS,
        graphics: Option<&FrameGraphics>,
        mut finish_frame: Finish,
    ) -> Result<(), Er>
    where
        S: AsyncFrameSource<M>,
        B: Backend<Error = Er>,
        E: Stream<Item = Result<Event, Er>> + Unpin,
        MS: Stream<Item = Result<M, Er>> + Unpin,
        Finish: FnMut(Option<&str>) -> Result<(), Er>,
    {
        let mut events_done = false;
        let mut messages_done = false;
        let mut core = RunnerCore::new();
        let mut selection = RunnerSelection::new(self.selects_text());
        let split = !self.config.screen_mode.is_alternate();
        let mut ticker = interval(self.config.tick_rate);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        if split {
            // Same order as every later pin: the terminal may already have been
            // resized between its construction and this first frame, and
            // `pin_footer` inserts rows at the geometry the terminal last knew.
            terminal.autoresize()?;
            pin_footer(terminal)?;
        }
        if let RunnerAction::Render(frame) = core.next_action() {
            let copied = draw(terminal, theme, frames, frame, graphics, &mut selection)?;
            finish_frame(copied.as_deref())?;
        }
        let mut last_frame = TokioInstant::now();
        let mut redraw_at = None;

        loop {
            enum Wake<M> {
                Tick,
                Event(Event),
                Message(M),
                /// A scheduled repaint deadline came due.
                Redraw,
                /// A background producer asked for one through a
                /// [`RedrawHandle`].
                Requested,
            }

            let deadline = redraw_at.unwrap_or_else(TokioInstant::now);
            let wake = tokio::select! {
                _ = sleep_until(deadline), if redraw_at.is_some() => Wake::Redraw,
                () = self.redraw.requested() => Wake::Requested,
                _ = ticker.tick() => Wake::Tick,
                item = stream::next(&mut events), if !events_done => match item {
                    Some(Ok(event)) => Wake::Event(event),
                    Some(Err(error)) => return Err(error),
                    None => {
                        events_done = true;
                        continue;
                    }
                },
                item = stream::next(&mut messages), if !messages_done => match item {
                    Some(Ok(message)) => Wake::Message(message),
                    Some(Err(error)) => return Err(error),
                    None => {
                        messages_done = true;
                        continue;
                    }
                },
            };

            // An external request only marks the frame stale; the repaint
            // itself goes through the same deadline path as every other one, so
            // a burst of requests still coalesces into one frame.
            if matches!(&wake, Wake::Requested) {
                core.request_redraw();
                let now = TokioInstant::now();
                redraw_at = Some(redraw_at.map_or(now, |current: TokioInstant| current.min(now)));
                continue;
            }

            if matches!(&wake, Wake::Redraw) {
                if let RunnerAction::Render(frame) = core.next_action() {
                    if split {
                        terminal.autoresize()?;
                        pin_footer(terminal)?;
                    }
                    let copied = draw(terminal, theme, frames, frame, graphics, &mut selection)?;
                    finish_frame(copied.as_deref())?;
                    last_frame = TokioInstant::now();
                }
                redraw_at = None;
                continue;
            }

            let (signal, selection_event) = match wake {
                Wake::Tick => (Signal::Tick, None),
                Wake::Event(event) => {
                    let selection = Some(event.clone());
                    (Signal::Event(event), selection)
                }
                Wake::Message(message) => (Signal::Message(message), None),
                Wake::Redraw | Wake::Requested => unreachable!("handled above"),
            };
            let requires_redraw = signal.requires_redraw();
            let result = frames.update(signal).await;
            core.apply(result);
            if core.is_exited() {
                break;
            }
            let selection_changed = selection_event
                .is_some_and(|event| selection.handle_event(&event, result, &SystemClock));
            if requires_redraw || result == UpdateResult::Dirty || selection_changed {
                core.request_redraw();
                let now = TokioInstant::now();
                let deadline = if requires_redraw {
                    (last_frame + RESIZE_FRAME_INTERVAL).max(now)
                } else {
                    now
                };
                redraw_at =
                    Some(redraw_at.map_or(deadline, |current: TokioInstant| current.min(deadline)));
            }
            if split {
                // A block is rendered and inserted at the width the terminal
                // last knew, so an unobserved resize would publish it at the
                // old one. Learn the size before committing, not after.
                terminal.autoresize()?;
                if self.scrollback.flush(terminal, theme)? {
                    core.request_redraw();
                    let now = TokioInstant::now();
                    redraw_at = Some(redraw_at.map_or(now, |current| current.min(now)));
                }
            } else {
                self.scrollback.clear();
            }
            if redraw_at.is_some_and(|deadline| deadline <= TokioInstant::now()) {
                if let RunnerAction::Render(frame) = core.next_action() {
                    if split {
                        terminal.autoresize()?;
                        pin_footer(terminal)?;
                    }
                    let copied = draw(terminal, theme, frames, frame, graphics, &mut selection)?;
                    finish_frame(copied.as_deref())?;
                    last_frame = TokioInstant::now();
                }
                redraw_at = None;
            }
        }

        Ok(())
    }
}

/// Paint one numbered frame from the current frame source.
fn draw<M, S, B, Er>(
    terminal: &mut Terminal<B>,
    theme: &Theme,
    frames: &mut S,
    frame: u64,
    graphics: Option<&FrameGraphics>,
    selection: &mut RunnerSelection,
) -> Result<Option<String>, Er>
where
    B: Backend<Error = Er>,
    S: AsyncFrameSource<M>,
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

/// An empty application-message stream, for a loop that has none.
///
/// The message type is [`Infallible`], so [`Signal::Message`] is uninhabited and
/// an `update` that handles only ticks and events stays exhaustive.
pub fn no_messages<Er>() -> impl Stream<Item = Result<Infallible, Er>> + Unpin {
    stream::empty()
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use crate::ScreenMode;
    use crate::components::Text;
    use crate::event::{Key, KeyCode, Mouse, MouseButton, MouseKind};
    use crate::geometry::Rect;
    use crate::term::testbackend::TestBackend;
    use crate::view::element;

    use crate::View;
    use crate::geometry::Size;
    use crate::surface::Surface;

    /// A key event as the infallible-stream item the `TestBackend` tests use.
    fn key(code: KeyCode) -> Result<Event, Infallible> {
        Ok(Event::Key(Key {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }))
    }

    fn mouse(kind: MouseKind, column: u16, row: u16) -> Result<Event, Infallible> {
        Ok(Event::Mouse(Mouse::at(kind, column, row)))
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
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn zero_tick_rate_is_clamped() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::ZERO,
            ..RunnerConfig::default()
        });
        assert_eq!(runner.config.tick_rate, Duration::from_millis(1));
    }

    // Events flow through `update`, mutate the owned local state, and a quit key
    // breaks the loop with `Ok(())`. A far-off tick rate keeps the timer out of
    // this test so it stays timing-independent.
    #[tokio::test]
    async fn events_drive_state_and_quit_breaks() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(24, 1);
        let mut count = 0u64;
        let events = stream::iter([
            key(KeyCode::Char('a')),
            key(KeyCode::Char('a')),
            key(KeyCode::Char('q')),
        ]);

        let result = runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut count,
                    |count, _frame| element(Text::raw(format!("count={count}"))),
                    async |count, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Char('q') => {
                            UpdateResult::Exit
                        }
                        Signal::Event(Event::Key(_)) => {
                            *count += 1;
                            UpdateResult::Dirty
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(count, 2, "both 'a' presses counted, 'q' quit");
        assert!(
            buffer_text(&terminal).contains("count=2"),
            "final frame reflects state: {:?}",
            buffer_text(&terminal)
        );
    }

    #[tokio::test]
    async fn clean_updates_do_not_rebuild_or_repaint() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(16, 1);
        let mut state = 0u8;
        let views = std::cell::Cell::new(0usize);
        let events = stream::iter([key(KeyCode::Char('a')), key(KeyCode::Esc)]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |state, _frame| {
                        views.set(views.get() + 1);
                        element(Text::raw(format!("state={state}")))
                    },
                    async |_state, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        assert_eq!(
            views.get(),
            1,
            "clean input leaves the initial frame intact"
        );
    }

    #[tokio::test]
    async fn clean_mouse_drag_uses_selection_when_mouse_is_captured() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            screen_mode: ScreenMode::Alternate.with_mouse_capture(),
        });
        let mut terminal = terminal(11, 1);
        let mut mouse_events = 0usize;
        let events = stream::iter([
            mouse(MouseKind::Down(MouseButton::Left), 0, 0),
            mouse(MouseKind::Drag(MouseButton::Left), 4, 0),
            key(KeyCode::Esc),
        ]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut mouse_events,
                    |_state, _frame| element(Text::raw("hello world")),
                    async |mouse_events, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        Signal::Event(Event::Mouse(_)) => {
                            *mouse_events += 1;
                            UpdateResult::Clean
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        assert_eq!(
            mouse_events, 2,
            "selection does not hide events from the app"
        );
        let buffer = terminal.backend().buffer();
        for column in 0..=4 {
            assert_eq!(
                buffer[(column, 0)].bg,
                Theme::default().selection_bg,
                "dragged cell {column} is highlighted"
            );
        }
    }

    #[tokio::test]
    async fn consumed_mouse_drag_claims_the_gesture() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(11, 1);
        let mut state = ();
        let events = stream::iter([
            mouse(MouseKind::Down(MouseButton::Left), 0, 0),
            mouse(MouseKind::Drag(MouseButton::Left), 4, 0),
            key(KeyCode::Esc),
        ]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |_state, _frame| element(Text::raw("hello world")),
                    async |_state, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        Signal::Event(Event::Mouse(_)) => UpdateResult::Consumed,
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert!(
            (0..=4).all(|column| buffer[(column, 0)].bg != Theme::default().selection_bg),
            "claimed drag is left entirely to the application"
        );
    }

    #[tokio::test]
    async fn mouse_wheel_still_drives_application_scrolling() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(12, 1);
        let mut scrolled = false;
        let events = stream::iter([mouse(MouseKind::ScrollDown, 0, 0), key(KeyCode::Esc)]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut scrolled,
                    |scrolled, _frame| {
                        element(Text::raw(if *scrolled { "line two" } else { "line one" }))
                    },
                    async |scrolled, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        Signal::Event(Event::Mouse(Mouse {
                            kind: MouseKind::ScrollDown,
                            ..
                        })) => {
                            *scrolled = true;
                            UpdateResult::Dirty
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        assert!(buffer_text(&terminal).contains("line two"));
    }

    #[tokio::test(start_paused = true)]
    async fn resize_bursts_share_one_repaint() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_millis(20),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(16, 1);
        let mut state = ();
        let views = std::cell::Cell::new(0usize);
        let events = stream::iter((0..3).map(|_| {
            Ok(Event::Resize {
                width: 16,
                height: 1,
            })
        }));

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |_state, _frame| {
                        views.set(views.get() + 1);
                        element(Text::raw("frame"))
                    },
                    async |_state, signal| match signal {
                        Signal::Tick if views.get() >= 2 => UpdateResult::Exit,
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        assert_eq!(
            views.get(),
            2,
            "three queued resizes share one frame after the initial paint"
        );
    }

    // The initial frame is painted before any signal is handled, so a run that
    // quits on the very first event still shows state on screen.
    #[tokio::test]
    async fn initial_frame_paints_before_first_signal() {
        let runner = AsyncRunner::new(RunnerConfig::default());
        let mut terminal = terminal(16, 1);
        let mut state = "hello";
        let events = stream::iter([key(KeyCode::Esc)]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |state, _frame| element(Text::raw(*state)),
                    async |_state, _signal| UpdateResult::Exit,
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        assert!(buffer_text(&terminal).contains("hello"));
    }

    // Ticks fire on the interval and can await. Under a paused clock the runtime
    // auto-advances time whenever it goes idle, so the interval fires
    // deterministically with no wall-clock wait; the empty event stream runs dry
    // and the tick-only loop keeps going until the third tick breaks it.
    #[tokio::test(start_paused = true)]
    async fn ticks_fire_and_can_await() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_millis(50),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(20, 1);
        let mut ticks = 0u64;
        let events = stream::iter(Vec::<Result<Event, Infallible>>::new());

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut ticks,
                    |ticks, _frame| element(Text::raw(format!("ticks={ticks}"))),
                    async |ticks, signal| {
                        if let Signal::Tick = signal {
                            // Prove an await inside tick handling is fine.
                            tokio::task::yield_now().await;
                            *ticks += 1;
                        }
                        if *ticks >= 3 {
                            UpdateResult::Exit
                        } else {
                            UpdateResult::Dirty
                        }
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .unwrap();

        assert_eq!(ticks, 3);
        // The break tick is not painted (the loop exits before the redraw), so
        // the last frame on screen is the preceding tick's `ticks=2`. This is
        // the same "no redraw after Break" rule the event test relies on.
        assert!(
            buffer_text(&terminal).contains("ticks=2"),
            "last painted frame: {:?}",
            buffer_text(&terminal)
        );
    }

    // The whole split-footer loop, hermetically: the footer is pinned to the
    // bottom rows, a block published from the update callback is committed to
    // the scrollback above it, and the footer is repainted afterwards.
    #[tokio::test]
    async fn split_footer_publishes_above_a_pinned_footer() {
        use crate::geometry::Position;
        use crate::screen::ScreenMode;

        let runner = AsyncRunner::new(RunnerConfig {
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
        let events = stream::iter([key(KeyCode::Char('a')), key(KeyCode::Char('q'))]);
        let mut state = ();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |_state, _frame| {
                        element(Text::new(vec![
                            crate::text::Line::from("FOOTER"),
                            crate::text::Line::from("FOOTER"),
                        ]))
                    },
                    async |_state, signal| match signal {
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
                events,
                no_messages(),
            )
            .await
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

    // A producer running beside the loop — the shape every real host has — gets
    // its blocks published without touching the runner's state.
    #[tokio::test]
    async fn a_background_task_publishes_while_the_loop_runs() {
        use crate::geometry::Position;
        use crate::screen::ScreenMode;

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_millis(5),
            screen_mode: ScreenMode::split_footer(2),
        });
        let scrollback = runner.scrollback();
        let mut backend = TestBackend::new(14, 8);
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

        // The producer is a task, not the update callback: it publishes on its
        // own schedule and the loop picks the blocks up on its next tick.
        let producer = scrollback.clone();
        tokio::spawn(async move {
            for i in 0..3u32 {
                producer.write(move |_width| element(Text::raw(format!("task-{i}"))));
                tokio::task::yield_now().await;
            }
        });

        let mut ticks = 0u32;
        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut ticks,
                    |_ticks, _frame| element(Text::raw("FOOTER")),
                    async |ticks, _signal| {
                        *ticks += 1;
                        // Enough ticks for the spawned task to be polled and its
                        // blocks flushed.
                        if *ticks >= 8 {
                            UpdateResult::Exit
                        } else {
                            UpdateResult::Clean
                        }
                    },
                ),
                stream::iter(Vec::<Result<Event, Infallible>>::new()),
                no_messages(),
            )
            .await
            .expect("run");

        let buffer = terminal.backend().buffer();
        let lines: Vec<String> = (0..8)
            .map(|y| crate::tests::support::row(buffer, y))
            .collect();
        for i in 0..3 {
            assert!(
                lines.contains(&format!("task-{i}")),
                "block {i} from the background task reached the scrollback: {lines:?}"
            );
        }
        assert!(scrollback.is_empty(), "the queue drains as the loop runs");
    }

    // Nothing above the frame owns scrollback on the alternate screen, so a
    // queued block is dropped instead of accumulating for a flush that can
    // never happen.
    #[tokio::test]
    async fn alternate_screen_discards_queued_blocks() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let scrollback = runner.scrollback();
        scrollback.write(|_width| element(Text::raw("dropped")));
        let mut terminal = terminal(16, 2);
        let events = stream::iter([key(KeyCode::Char('a')), key(KeyCode::Esc)]);
        let mut state = ();

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |_state, _frame| element(Text::raw("frame")),
                    async |_state, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                events,
                no_messages(),
            )
            .await
            .expect("run");

        assert!(scrollback.is_empty(), "the queue is drained, not retained");
        assert!(!buffer_text(&terminal).contains("dropped"));
    }

    // A read error from the event stream propagates out of the run. This uses a
    // real `io::Error` backend (crossterm over a `Vec` sink) so the run's error
    // type is `io::Error` rather than the `Infallible` of `TestBackend`. A fixed
    // viewport is required: `Terminal::new`/`Fullscreen` queries the terminal
    // size, which has no TTY to answer under CI and would panic; `Fixed` uses the
    // given rect and never touches the terminal.
    #[tokio::test]
    async fn stream_error_propagates() {
        use crate::geometry::Rect;
        use crate::term::backend::CrosstermBackend;

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(Vec::<u8>::new()),
            TerminalOptions {
                viewport: crate::term::terminal::Viewport::Fixed(Rect::new(0, 0, 10, 1)),
            },
        )
        .expect("test terminal");
        let mut state = ();
        let events = stream::iter([Err::<Event, io::Error>(io::Error::other("boom"))]);

        let result = runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |_state, _frame| element(Text::raw("x")),
                    async |_state, _signal| UpdateResult::Dirty,
                ),
                events,
                no_messages(),
            )
            .await;

        let error = result.expect_err("stream error should propagate");
        assert_eq!(error.to_string(), "boom");
    }

    // The run ends when the tally is complete, not on a third `Exit` message:
    // `tokio::select!` polls its branches in a random order, so nothing orders
    // the message stream against the event stream, and an `Exit` racing the key
    // event ended the loop with the key still unconsumed. Exiting on the total
    // makes the outcome the same for every interleaving.
    //
    // The frame is deliberately not asserted here. `Exit` breaks the loop
    // before the repaint it scheduled, so the last update is not on screen —
    // whether that is the behavior hosts want is a question about the runner,
    // not about this test, and `renders_the_initial_frame_before_any_signal`
    // and the redraw tests cover the drawing path.
    #[tokio::test]
    async fn terminal_events_and_external_messages_share_one_loop() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Message {
            Add(u64),
        }

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(24, 1);
        let mut count = 0u64;
        let events = stream::iter([key(KeyCode::Char('a'))]);
        let messages = stream::iter([Ok::<_, Infallible>(Message::Add(7))]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut count,
                    |count, _| element(Text::raw(format!("count={count}"))),
                    async |count, signal| {
                        // Both inputs are in once the tally reaches 8, whichever
                        // of them the loop happened to poll first.
                        let done = |count: u64| {
                            if count == 8 {
                                UpdateResult::Exit
                            } else {
                                UpdateResult::Dirty
                            }
                        };
                        match signal {
                            Signal::Event(Event::Key(_)) => {
                                *count += 1;
                                done(*count)
                            }
                            Signal::Message(Message::Add(value)) => {
                                *count += value;
                                done(*count)
                            }
                            Signal::Tick | Signal::Event(_) => UpdateResult::Clean,
                        }
                    },
                ),
                events,
                messages,
            )
            .await
            .unwrap();

        assert_eq!(count, 8);
    }

    #[tokio::test(start_paused = true)]
    async fn completed_message_stream_leaves_ticks_active() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_millis(20),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(16, 1);
        let mut ticks = 0u8;

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut ticks,
                    |ticks, _| element(Text::raw(format!("ticks={ticks}"))),
                    async |ticks, signal| match signal {
                        Signal::Tick => {
                            *ticks += 1;
                            if *ticks == 2 {
                                UpdateResult::Exit
                            } else {
                                UpdateResult::Dirty
                            }
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                stream::empty::<Result<Event, Infallible>>(),
                stream::empty::<Result<(), Infallible>>(),
            )
            .await
            .unwrap();

        assert_eq!(ticks, 2);
    }

    #[tokio::test]
    async fn message_redraw_and_exit_results_are_respected() {
        #[derive(Clone, Copy)]
        enum Message {
            Clean,
            Dirty,
            Exit,
        }

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(16, 1);
        let views = std::cell::Cell::new(0usize);
        let mut state = 0u8;
        let messages = stream::iter([
            Ok::<_, Infallible>(Message::Clean),
            Ok(Message::Dirty),
            Ok(Message::Exit),
        ]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |state, _| {
                        views.set(views.get() + 1);
                        element(Text::raw(format!("state={state}")))
                    },
                    async |state, signal| match signal {
                        Signal::Message(Message::Clean) => {
                            *state = 1;
                            UpdateResult::Clean
                        }
                        Signal::Message(Message::Dirty) => {
                            *state = 2;
                            UpdateResult::Dirty
                        }
                        Signal::Message(Message::Exit) => {
                            *state = 3;
                            UpdateResult::Exit
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                stream::empty::<Result<Event, Infallible>>(),
                messages,
            )
            .await
            .unwrap();

        assert_eq!(views.get(), 2, "initial frame plus the dirty message");
        assert!(buffer_text(&terminal).contains("state=2"));
        assert_eq!(state, 3, "exit mutates state but does not repaint");
    }

    #[tokio::test]
    async fn message_stream_error_propagates() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(Vec::<u8>::new()),
            TerminalOptions {
                viewport: crate::term::terminal::Viewport::Fixed(Rect::new(0, 0, 10, 1)),
            },
        )
        .expect("test terminal");
        let mut state = ();
        let messages = stream::iter([Err::<(), io::Error>(io::Error::other("message boom"))]);

        let result = runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |_state, _| element(Text::raw("x")),
                    async |_state, _signal| UpdateResult::Clean,
                ),
                stream::empty::<Result<Event, io::Error>>(),
                messages,
            )
            .await;

        assert_eq!(
            result.expect_err("message error").to_string(),
            "message boom"
        );
    }

    /// An application whose frame borrows its own data, used to prove the
    /// borrowed boundary reaches the async loop unchanged.
    struct Notes {
        title: String,
        hits: Vec<char>,
    }

    struct NotesView<'app> {
        title: &'app str,
        hits: &'app [char],
    }

    impl View for NotesView<'_> {
        fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
            Size::new(available.width, 1.min(available.height))
        }

        fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
            let line = format!("{}:{}", self.title, self.hits.iter().collect::<String>());
            surface.set_string(area.x, area.y, &line, ctx.theme.text_style());
        }
    }

    impl AsyncApplication for Notes {
        async fn update(&mut self, signal: Signal) -> UpdateResult {
            match signal {
                Signal::Event(Event::Key(key)) if key.code == KeyCode::Esc => UpdateResult::Exit,
                Signal::Event(Event::Key(Key {
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

    // The async runner drives the same `Application`-shaped boundary the sync runner
    // does: events mutate through `&mut self`, and the frame borrows `&self`
    // without cloning into an owned tree.
    #[tokio::test]
    async fn an_application_paints_a_borrowed_view() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(12, 1);
        let mut app = Notes {
            title: "n".into(),
            hits: Vec::new(),
        };
        let events = stream::iter([
            key(KeyCode::Char('a')),
            key(KeyCode::Char('b')),
            key(KeyCode::Esc),
        ]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                &mut app,
                events,
                no_messages(),
            )
            .await
            .expect("run");

        assert_eq!(app.hits, vec!['a', 'b']);
        assert!(
            buffer_text(&terminal).contains("n:ab"),
            "final frame borrows application data: {:?}",
            buffer_text(&terminal)
        );
    }

    /// The message-carrying twin of `Notes`: the same boundary with a different
    /// signal type rather than a different trait.
    struct Feed {
        items: Vec<u8>,
    }

    impl AsyncApplication<u8> for Feed {
        async fn update(&mut self, signal: Signal<u8>) -> UpdateResult {
            match signal {
                Signal::Message(0) => UpdateResult::Exit,
                Signal::Message(item) => {
                    self.items.push(item);
                    UpdateResult::Dirty
                }
                _ => UpdateResult::Clean,
            }
        }

        fn view(&self, _frame: u64) -> ScopedElement<'_> {
            element(Text::raw(format!("items={}", self.items.len())))
        }
    }

    #[tokio::test]
    async fn an_application_receives_messages_through_the_same_channel() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(12, 1);
        let mut app = Feed { items: Vec::new() };
        let messages = stream::iter([Ok::<u8, Infallible>(7), Ok(9), Ok(0)]);

        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                &mut app,
                stream::empty::<Result<Event, Infallible>>(),
                messages,
            )
            .await
            .expect("run");

        assert_eq!(app.items, vec![7, 9]);
        assert!(buffer_text(&terminal).contains("items=2"));
    }

    // A background producer that only owns a `RedrawHandle` — the `Live` case —
    // must be able to wake the select loop, which has no polling iteration of
    // its own to notice the flag from. Time is paused, so the producer's sleep
    // cannot advance until the runner is parked again: the repaint is ordered
    // before the quit key without a wall-clock race.
    #[tokio::test(start_paused = true)]
    async fn redraw_handle_wakes_the_loop_and_repaints() {
        use tokio_stream::wrappers::ReceiverStream;

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(12, 1);
        let redraw = runner.redraw_handle();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let views = std::cell::Cell::new(0usize);
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(4);

        let producer = {
            let counter = std::sync::Arc::clone(&counter);
            tokio::spawn(async move {
                counter.store(41, std::sync::atomic::Ordering::Release);
                redraw.request();
                tokio::time::sleep(Duration::from_millis(1)).await;
                sender.send(key(KeyCode::Esc)).await.expect("send quit");
            })
        };

        let mut state = std::sync::Arc::clone(&counter);
        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |state, _frame| {
                        views.set(views.get() + 1);
                        let value = state.load(std::sync::atomic::Ordering::Acquire);
                        element(Text::raw(format!("value={value}")))
                    },
                    async |_state, signal| match signal {
                        Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => {
                            UpdateResult::Exit
                        }
                        _ => UpdateResult::Clean,
                    },
                ),
                ReceiverStream::new(receiver),
                no_messages(),
            )
            .await
            .expect("run");
        producer.await.expect("producer");

        assert_eq!(views.get(), 2, "initial frame plus the requested repaint");
        assert!(
            buffer_text(&terminal).contains("value=41"),
            "the requested repaint shows the producer's data: {:?}",
            buffer_text(&terminal)
        );
    }
}
