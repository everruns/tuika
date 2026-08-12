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
//! The loop threads a single `state` value through two callbacks: `view` builds
//! the frame from `&state`, and `update` mutates `&mut state` in response to a
//! [`Signal`] (a tick or an input event) and may `.await` while doing so. Both
//! borrow the same value at different times, which is why it is a runner
//! argument rather than a capture — a closure cannot hold `&state` and
//! `&mut state` at once.
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
//!             &mut requests,
//!             |requests, _frame| element(Text::raw(format!("requests: {requests}"))),
//!             async |requests, signal| match signal {
//!                 // Poll on every tick and on `r`; both may await.
//!                 Signal::Tick => {
//!                     *requests = fetch_stats().await.unwrap_or(*requests);
//!                     UpdateResult::Dirty
//!                 }
//!                 Signal::Event(Event::Key(k)) if k.plain() => match k.code {
//!                     KeyCode::Char('q') | KeyCode::Esc => UpdateResult::Exit,
//!                     KeyCode::Char('r') => {
//!                         *requests = fetch_stats().await.unwrap_or(*requests);
//!                         UpdateResult::Dirty
//!                     }
//!                     _ => UpdateResult::Clean,
//!                 },
//!                 _ => UpdateResult::Clean,
//!             },
//!         )
//!         .await
//! }
//! ```

use std::io;
use std::time::Duration;

use crossterm::event::EventStream;
use ratatui_core::backend::Backend;
use ratatui_core::terminal::{Terminal, TerminalOptions};
use ratatui_crossterm::CrosstermBackend;
use tokio::time::{Instant as TokioInstant, MissedTickBehavior, interval, sleep_until};
use tokio_stream::{Stream, StreamExt};

use super::{
    FrameGraphics, FrameGraphicsCleanup, RESIZE_FRAME_INTERVAL, RunnerAction, RunnerCore,
    RunnerSelection, Signal,
};
use crate::screen::{Scrollback, close_footer, pin_footer};
use crate::{
    Element, Event, RenderCtx, RunnerConfig, SystemClock, TerminalSession, Theme, UpdateResult,
    paint_with_context, translate_event,
};

/// A signal delivered by an [`AsyncRunner`] that also listens to a typed
/// application message stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncSignal<M> {
    /// The configured tick interval elapsed.
    Tick,
    /// A translated terminal input event arrived.
    Event(Event),
    /// A host or background producer sent an application message.
    Message(M),
}

impl<M> AsyncSignal<M> {
    fn requires_redraw(&self) -> bool {
        matches!(self, Self::Event(Event::Resize { .. }))
    }
}

/// An asynchronous Crossterm event and rendering loop.
///
/// See the [module documentation](crate::runner) for the full picture. Construct one with
/// a [`RunnerConfig`], then drive it with [`run`](Self::run) (real terminal),
/// [`run_with_backend`](Self::run_with_backend) (caller-supplied backend, such as
/// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend)), or
/// [`run_with_events`](Self::run_with_events) (caller-supplied backend *and*
/// event stream, for tests and hosts that own the terminal lifecycle).
pub struct AsyncRunner {
    config: RunnerConfig,
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
            scrollback: Scrollback::new(),
            session_config: None,
            text_selection: true,
        }
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

    /// Run on the real terminal until `update` returns [`UpdateResult::Exit`].
    ///
    /// Enters a [`TerminalSession`] (restored on return, including on error or
    /// panic) and reads input from crossterm's async
    /// [`EventStream`].
    pub async fn run<S, V, U>(
        &self,
        theme: &Theme,
        state: &mut S,
        view: V,
        update: U,
    ) -> io::Result<()>
    where
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, Signal) -> UpdateResult,
    {
        let graphics = FrameGraphics::detected();
        self.run_with_backend_inner(
            theme,
            CrosstermBackend::new(io::stdout()),
            state,
            view,
            update,
            Some(&graphics),
        )
        .await
    }

    /// Run on the real terminal while also receiving typed application
    /// `messages`.
    ///
    /// A background producer can send domain values through any Tokio
    /// [`Stream`] without shared mutable state, synthetic terminal events, or
    /// polling ticks. A completed message stream disables only that input;
    /// terminal events and ticks continue. Stream errors are propagated after
    /// terminal cleanup.
    pub async fn run_with_messages<S, M, V, U, MS>(
        &self,
        theme: &Theme,
        state: &mut S,
        messages: MS,
        view: V,
        update: U,
    ) -> io::Result<()>
    where
        MS: Stream<Item = io::Result<M>> + Unpin,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, AsyncSignal<M>) -> UpdateResult,
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
        let events = EventStream::new().filter_map(|result| match result {
            Ok(raw) => translate_event(raw).map(Ok),
            Err(error) => Some(Err(error)),
        });
        let result = self
            .run_with_events_and_messages_inner(
                &mut terminal,
                theme,
                state,
                events,
                messages,
                view,
                update,
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
        if mode.is_alternate() {
            let _ = terminal.clear();
        } else {
            let _ = close_footer(&mut terminal);
        }
        result
    }

    /// Run with a caller-provided backend, such as
    /// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend). Otherwise identical to
    /// [`run`](Self::run): it owns the [`TerminalSession`] and the
    /// [`EventStream`].
    pub async fn run_with_backend<S, B, V, U>(
        &self,
        theme: &Theme,
        backend: B,
        state: &mut S,
        view: V,
        update: U,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error>,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, Signal) -> UpdateResult,
    {
        self.run_with_backend_inner(theme, backend, state, view, update, None)
            .await
    }

    async fn run_with_backend_inner<S, B, V, U>(
        &self,
        theme: &Theme,
        backend: B,
        state: &mut S,
        view: V,
        update: U,
        graphics: Option<&FrameGraphics>,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error>,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, Signal) -> UpdateResult,
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
        let _graphics_cleanup = graphics.map(FrameGraphicsCleanup);
        // Translate crossterm events to tuika events at the stream boundary,
        // dropping the ones tuika does not model and surfacing read errors.
        let events = EventStream::new().filter_map(|result| match result {
            Ok(raw) => translate_event(raw).map(Ok),
            Err(error) => Some(Err(error)),
        });
        let result = self
            .run_with_events_inner(
                &mut terminal,
                theme,
                state,
                events,
                view,
                update,
                graphics,
                |copied| {
                    if let Some(graphics) = graphics {
                        graphics.finish_frame()?;
                    }
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

    /// The core loop: caller-owned `terminal` and `events`, no terminal
    /// lifecycle. This is what [`run`](Self::run) builds on, and the seam tests
    /// use to drive the runner against a
    /// [`TestBackend`](ratatui_core::backend::TestBackend) with a scripted event
    /// stream. Hosts that already own their terminal and event source (or want a
    /// non-crossterm one) can call it directly.
    ///
    /// The backend error and the event-stream error share one type `Er`, which
    /// is the run's error type: for the real terminal that is [`io::Error`], but
    /// leaving it generic lets an infallible backend
    /// ([`TestBackend`](ratatui_core::backend::TestBackend), whose error is
    /// [`Infallible`](std::convert::Infallible)) pair with an infallible stream.
    ///
    /// `events` yields already-translated tuika [`Event`]s. An `Err` item ends
    /// the run by propagating out; `None` (a finite stream running dry) stops
    /// event delivery but leaves the tick timer running, so the loop still exits
    /// only when `update` returns [`UpdateResult::Exit`].
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
        self.run_with_events_inner(terminal, theme, state, events, view, update, None, |_| {
            Ok(())
        })
        .await
    }

    /// Run with caller-owned terminal, terminal-event stream, and typed
    /// application-message stream.
    ///
    /// Both streams yield the backend's error type. Completion of either one
    /// disables that source without busy-spinning; an error from either source
    /// ends the run. Each delivered [`AsyncSignal`] independently controls
    /// redraw and exit through [`UpdateResult`].
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
        U: AsyncFnMut(&mut S, AsyncSignal<M>) -> UpdateResult,
    {
        self.run_with_events_and_messages_inner(
            terminal,
            theme,
            state,
            events,
            messages,
            view,
            update,
            None,
            |_| Ok(()),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_with_events_and_messages_inner<S, M, B, V, U, E, MS, Er, F>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        state: &mut S,
        mut events: E,
        mut messages: MS,
        mut view: V,
        mut update: U,
        graphics: Option<&FrameGraphics>,
        mut finish_frame: F,
    ) -> Result<(), Er>
    where
        B: Backend<Error = Er>,
        E: Stream<Item = Result<Event, Er>> + Unpin,
        MS: Stream<Item = Result<M, Er>> + Unpin,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, AsyncSignal<M>) -> UpdateResult,
        F: FnMut(Option<&str>) -> Result<(), Er>,
    {
        let mut events_done = false;
        let mut messages_done = false;
        let mut core = RunnerCore::new();
        let mut selection = RunnerSelection::new(self.selects_text());
        let split = !self.config.screen_mode.is_alternate();
        let mut ticker = interval(self.config.tick_rate);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        if split {
            pin_footer(terminal)?;
        }
        if let RunnerAction::Render(frame) = core.next_action() {
            let copied = draw(
                terminal,
                theme,
                &mut view,
                state,
                frame,
                graphics,
                &mut selection,
            )?;
            finish_frame(copied.as_deref())?;
        }
        let mut last_frame = TokioInstant::now();
        let mut redraw_at = None;

        loop {
            enum Wake<M> {
                Tick,
                Event(Event),
                Message(M),
                Redraw,
            }

            let deadline = redraw_at.unwrap_or_else(TokioInstant::now);
            let wake = tokio::select! {
                biased;
                _ = sleep_until(deadline), if redraw_at.is_some() => Wake::Redraw,
                _ = ticker.tick() => Wake::Tick,
                item = events.next(), if !events_done => match item {
                    Some(Ok(event)) => Wake::Event(event),
                    Some(Err(error)) => return Err(error),
                    None => {
                        events_done = true;
                        continue;
                    }
                },
                item = messages.next(), if !messages_done => match item {
                    Some(Ok(message)) => Wake::Message(message),
                    Some(Err(error)) => return Err(error),
                    None => {
                        messages_done = true;
                        continue;
                    }
                },
            };

            if matches!(&wake, Wake::Redraw) {
                if let RunnerAction::Render(frame) = core.next_action() {
                    if split {
                        terminal.autoresize()?;
                        pin_footer(terminal)?;
                    }
                    let copied = draw(
                        terminal,
                        theme,
                        &mut view,
                        state,
                        frame,
                        graphics,
                        &mut selection,
                    )?;
                    finish_frame(copied.as_deref())?;
                    last_frame = TokioInstant::now();
                }
                redraw_at = None;
                continue;
            }

            let (signal, selection_event) = match wake {
                Wake::Tick => (AsyncSignal::Tick, None),
                Wake::Event(event) => {
                    let selection = Some(event.clone());
                    (AsyncSignal::Event(event), selection)
                }
                Wake::Message(message) => (AsyncSignal::Message(message), None),
                Wake::Redraw => unreachable!(),
            };
            let requires_redraw = signal.requires_redraw();
            let result = update(state, signal).await;
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
                    let copied = draw(
                        terminal,
                        theme,
                        &mut view,
                        state,
                        frame,
                        graphics,
                        &mut selection,
                    )?;
                    finish_frame(copied.as_deref())?;
                    last_frame = TokioInstant::now();
                }
                redraw_at = None;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_with_events_inner<S, B, V, U, E, Er, F>(
        &self,
        terminal: &mut Terminal<B>,
        theme: &Theme,
        state: &mut S,
        events: E,
        view: V,
        mut update: U,
        graphics: Option<&FrameGraphics>,
        finish_frame: F,
    ) -> Result<(), Er>
    where
        B: Backend<Error = Er>,
        E: Stream<Item = Result<Event, Er>> + Unpin,
        V: FnMut(&S, u64) -> Element,
        U: AsyncFnMut(&mut S, Signal) -> UpdateResult,
        F: FnMut(Option<&str>) -> Result<(), Er>,
    {
        self.run_with_events_and_messages_inner(
            terminal,
            theme,
            state,
            events,
            tokio_stream::empty::<Result<std::convert::Infallible, Er>>(),
            view,
            async move |state, signal| match signal {
                AsyncSignal::Tick => update(state, Signal::Tick).await,
                AsyncSignal::Event(event) => update(state, Signal::Event(event)).await,
                AsyncSignal::Message(never) => match never {},
            },
            graphics,
            finish_frame,
        )
        .await
    }
}

/// Paint one numbered frame from the current `state`.
fn draw<S, B, V, Er>(
    terminal: &mut Terminal<B>,
    theme: &Theme,
    view: &mut V,
    state: &S,
    frame: u64,
    graphics: Option<&FrameGraphics>,
    selection: &mut RunnerSelection,
) -> Result<Option<String>, Er>
where
    B: Backend<Error = Er>,
    V: FnMut(&S, u64) -> Element,
{
    let mut copied = None;
    terminal.draw(|terminal_frame| {
        let area = terminal_frame.area();
        let root = view(state, frame);
        let ctx = graphics.map_or_else(|| RenderCtx::new(theme), |g| g.render_context(theme));
        paint_with_context(terminal_frame.buffer_mut(), area, &ctx, root.as_ref(), &[]);
        copied = selection.finish_frame(terminal_frame.buffer_mut(), area, theme);
    })?;
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use crate::components::Text;
    use crate::event::{Key, KeyCode, Mouse, MouseButton, MouseKind};
    use crate::view::element;
    use ratatui_core::backend::TestBackend;

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
        let events = tokio_stream::iter([
            key(KeyCode::Char('a')),
            key(KeyCode::Char('a')),
            key(KeyCode::Char('q')),
        ]);

        let result = runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut count,
                events,
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
        let events = tokio_stream::iter([key(KeyCode::Char('a')), key(KeyCode::Esc)]);

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |state, _frame| {
                    views.set(views.get() + 1);
                    element(Text::raw(format!("state={state}")))
                },
                async |_state, signal| match signal {
                    Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => UpdateResult::Exit,
                    _ => UpdateResult::Clean,
                },
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
    async fn clean_mouse_drag_uses_default_text_selection() {
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(11, 1);
        let mut mouse_events = 0usize;
        let events = tokio_stream::iter([
            mouse(MouseKind::Down(MouseButton::Left), 0, 0),
            mouse(MouseKind::Drag(MouseButton::Left), 4, 0),
            key(KeyCode::Esc),
        ]);

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut mouse_events,
                events,
                |_state, _frame| element(Text::raw("hello world")),
                async |mouse_events, signal| match signal {
                    Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => UpdateResult::Exit,
                    Signal::Event(Event::Mouse(_)) => {
                        *mouse_events += 1;
                        UpdateResult::Clean
                    }
                    _ => UpdateResult::Clean,
                },
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
        let events = tokio_stream::iter([
            mouse(MouseKind::Down(MouseButton::Left), 0, 0),
            mouse(MouseKind::Drag(MouseButton::Left), 4, 0),
            key(KeyCode::Esc),
        ]);

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |_state, _frame| element(Text::raw("hello world")),
                async |_state, signal| match signal {
                    Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => UpdateResult::Exit,
                    Signal::Event(Event::Mouse(_)) => UpdateResult::Consumed,
                    _ => UpdateResult::Clean,
                },
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
        let events = tokio_stream::iter([mouse(MouseKind::ScrollDown, 0, 0), key(KeyCode::Esc)]);

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut scrolled,
                events,
                |scrolled, _frame| {
                    element(Text::raw(if *scrolled { "line two" } else { "line one" }))
                },
                async |scrolled, signal| match signal {
                    Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => UpdateResult::Exit,
                    Signal::Event(Event::Mouse(Mouse {
                        kind: MouseKind::ScrollDown,
                        ..
                    })) => {
                        *scrolled = true;
                        UpdateResult::Dirty
                    }
                    _ => UpdateResult::Clean,
                },
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
        let events = tokio_stream::iter((0..3).map(|_| {
            Ok(Event::Resize {
                width: 16,
                height: 1,
            })
        }));

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |_state, _frame| {
                    views.set(views.get() + 1);
                    element(Text::raw("frame"))
                },
                async |_state, signal| match signal {
                    Signal::Tick if views.get() >= 2 => UpdateResult::Exit,
                    _ => UpdateResult::Clean,
                },
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
        let events = tokio_stream::iter([key(KeyCode::Esc)]);

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |state, _frame| element(Text::raw(*state)),
                async |_state, _signal| UpdateResult::Exit,
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
        let events = tokio_stream::iter(Vec::<Result<Event, Infallible>>::new());

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut ticks,
                events,
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
        use crate::screen::ScreenMode;
        use ratatui_core::layout::Position;

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
        let events = tokio_stream::iter([key(KeyCode::Char('a')), key(KeyCode::Char('q'))]);
        let mut state = ();

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |_state, _frame| {
                    element(Text::new(vec![
                        ratatui_core::text::Line::from("FOOTER"),
                        ratatui_core::text::Line::from("FOOTER"),
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
        use crate::screen::ScreenMode;
        use ratatui_core::layout::Position;

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
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut ticks,
                tokio_stream::iter(Vec::<Result<Event, Infallible>>::new()),
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
        let events = tokio_stream::iter([key(KeyCode::Char('a')), key(KeyCode::Esc)]);
        let mut state = ();

        runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |_state, _frame| element(Text::raw("frame")),
                async |_state, signal| match signal {
                    Signal::Event(Event::Key(k)) if k.code == KeyCode::Esc => UpdateResult::Exit,
                    _ => UpdateResult::Clean,
                },
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
        use ratatui_core::layout::Rect;
        use ratatui_crossterm::CrosstermBackend;

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(Vec::<u8>::new()),
            TerminalOptions {
                viewport: ratatui_core::terminal::Viewport::Fixed(Rect::new(0, 0, 10, 1)),
            },
        )
        .expect("test terminal");
        let mut state = ();
        let events = tokio_stream::iter([Err::<Event, io::Error>(io::Error::other("boom"))]);

        let result = runner
            .run_with_events(
                &mut terminal,
                &Theme::default(),
                &mut state,
                events,
                |_state, _frame| element(Text::raw("x")),
                async |_state, _signal| UpdateResult::Dirty,
            )
            .await;

        let error = result.expect_err("stream error should propagate");
        assert_eq!(error.to_string(), "boom");
    }

    #[tokio::test]
    async fn terminal_events_and_external_messages_share_one_loop() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Message {
            Add(u64),
            Exit,
        }

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = terminal(24, 1);
        let mut count = 0u64;
        let events = tokio_stream::iter([key(KeyCode::Char('a'))]);
        let messages =
            tokio_stream::iter([Ok::<_, Infallible>(Message::Add(7)), Ok(Message::Exit)]);

        runner
            .run_with_events_and_messages(
                &mut terminal,
                &Theme::default(),
                &mut count,
                events,
                messages,
                |count, _| element(Text::raw(format!("count={count}"))),
                async |count, signal| match signal {
                    AsyncSignal::Event(Event::Key(_)) => {
                        *count += 1;
                        UpdateResult::Dirty
                    }
                    AsyncSignal::Message(Message::Add(value)) => {
                        *count += value;
                        UpdateResult::Dirty
                    }
                    AsyncSignal::Message(Message::Exit) => UpdateResult::Exit,
                    AsyncSignal::Tick | AsyncSignal::Event(_) => UpdateResult::Clean,
                },
            )
            .await
            .unwrap();

        assert_eq!(count, 8);
        assert!(buffer_text(&terminal).contains("count=8"));
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
            .run_with_events_and_messages(
                &mut terminal,
                &Theme::default(),
                &mut ticks,
                tokio_stream::empty::<Result<Event, Infallible>>(),
                tokio_stream::empty::<Result<(), Infallible>>(),
                |ticks, _| element(Text::raw(format!("ticks={ticks}"))),
                async |ticks, signal| match signal {
                    AsyncSignal::Tick => {
                        *ticks += 1;
                        if *ticks == 2 {
                            UpdateResult::Exit
                        } else {
                            UpdateResult::Dirty
                        }
                    }
                    _ => UpdateResult::Clean,
                },
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
        let messages = tokio_stream::iter([
            Ok::<_, Infallible>(Message::Clean),
            Ok(Message::Dirty),
            Ok(Message::Exit),
        ]);

        runner
            .run_with_events_and_messages(
                &mut terminal,
                &Theme::default(),
                &mut state,
                tokio_stream::empty::<Result<Event, Infallible>>(),
                messages,
                |state, _| {
                    views.set(views.get() + 1);
                    element(Text::raw(format!("state={state}")))
                },
                async |state, signal| match signal {
                    AsyncSignal::Message(Message::Clean) => {
                        *state = 1;
                        UpdateResult::Clean
                    }
                    AsyncSignal::Message(Message::Dirty) => {
                        *state = 2;
                        UpdateResult::Dirty
                    }
                    AsyncSignal::Message(Message::Exit) => {
                        *state = 3;
                        UpdateResult::Exit
                    }
                    _ => UpdateResult::Clean,
                },
            )
            .await
            .unwrap();

        assert_eq!(views.get(), 2, "initial frame plus the dirty message");
        assert!(buffer_text(&terminal).contains("state=2"));
        assert_eq!(state, 3, "exit mutates state but does not repaint");
    }

    #[tokio::test]
    async fn message_stream_error_propagates() {
        use ratatui_core::layout::Rect;

        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            ..RunnerConfig::default()
        });
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(Vec::<u8>::new()),
            TerminalOptions {
                viewport: ratatui_core::terminal::Viewport::Fixed(Rect::new(0, 0, 10, 1)),
            },
        )
        .expect("test terminal");
        let mut state = ();
        let messages = tokio_stream::iter([Err::<(), io::Error>(io::Error::other("message boom"))]);

        let result = runner
            .run_with_events_and_messages(
                &mut terminal,
                &Theme::default(),
                &mut state,
                tokio_stream::empty::<Result<Event, io::Error>>(),
                messages,
                |_state, _| element(Text::raw("x")),
                async |_state, _signal| UpdateResult::Clean,
            )
            .await;

        assert_eq!(
            result.expect_err("message error").to_string(),
            "message boom"
        );
    }
}
