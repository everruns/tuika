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
//! Synchronous applications whose views borrow their own state implement
//! [`Application`] and run through [`Runner::run_app`]. The original
//! state/view/update closure API remains available for owned [`Element`] trees.
//!
//! A host with its own event loop needs neither: call [`crate::paint`] directly.

#[cfg(feature = "async")]
mod asynchronous;

#[cfg(feature = "async")]
pub use asynchronous::AsyncRunner;

use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event;
use ratatui_core::backend::Backend;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::terminal::{Terminal, TerminalOptions};
use ratatui_crossterm::CrosstermBackend;

use crate::live::RedrawHandle;
use crate::mouse::{SelectionState, paint_selection, selected_text};
use crate::screen::{ScreenMode, Scrollback, close_footer, pin_footer};
use crate::term::clipboard;
use crate::term::image::{ImageLayer, ImageSupport};
use crate::{
    Clock, Element, Event, RenderCtx, ScopedElement, SystemClock, TerminalSession, Theme, View,
    paint_with_context, translate_event,
};

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

/// A signal delivered to a runner update function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The configured tick interval elapsed.
    Tick,
    /// A translated terminal input event arrived. Resize events force a redraw
    /// after the update unless it exits, even when the update is clean.
    Event(Event),
}

impl Signal {
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

/// A data-driven synchronous terminal application.
///
/// The runner mutably borrows the application only while delivering a
/// [`Signal`], then immutably borrows it to build the next frame. Because the
/// returned tree is scoped to that immutable borrow, custom views can read
/// application data directly without cloning it into an owned [`Element`] or
/// sharing it through `Rc<RefCell<_>>`.
///
/// Rendering should be pure: persistent UI and domain state belongs on the
/// application and changes only in [`update`](Self::update).
pub trait Application {
    /// Update application state in response to a tick or terminal event. Return
    /// [`UpdateResult::Clean`] only when the signal was unhandled, or
    /// [`UpdateResult::Consumed`] when it was handled without a repaint.
    fn update(&mut self, signal: Signal) -> UpdateResult;

    /// Build the ephemeral view tree for one numbered frame.
    fn view(&self, frame: u64) -> ScopedElement<'_>;
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

    /// Run until `update` returns [`UpdateResult::Exit`].
    ///
    /// The runner paints once initially. It then delivers input and periodic
    /// [`Signal::Tick`] values to `update`, repainting only when `update`
    /// returns [`UpdateResult::Dirty`] or a [`RedrawHandle`] requests it.
    pub fn run<S, V, U>(
        &self,
        theme: &Theme,
        state: &mut S,
        mut view: V,
        update: U,
    ) -> io::Result<()>
    where
        V: FnMut(&S, u64) -> Element,
        U: FnMut(&mut S, Signal) -> UpdateResult,
    {
        let graphics = FrameGraphics::detected();
        self.run_with_backend_inner(
            theme,
            CrosstermBackend::new(io::stdout()),
            state,
            |state, frame, paint_root| {
                let root = view(state, frame);
                paint_root(root.as_ref());
            },
            update,
            Some(&graphics),
        )
    }

    /// Run a data-driven [`Application`] on the real terminal.
    ///
    /// This is the borrowed-view counterpart to [`run`](Self::run). It uses the
    /// same terminal lifecycle, scheduling, redraw, and split-footer behavior.
    pub fn run_app<A: Application>(&self, theme: &Theme, app: &mut A) -> io::Result<()> {
        let graphics = FrameGraphics::detected();
        self.run_with_backend_inner(
            theme,
            CrosstermBackend::new(io::stdout()),
            app,
            |app, frame, paint_root| {
                let root = app.view(frame);
                paint_root(root.as_ref());
            },
            Application::update,
            Some(&graphics),
        )
    }

    /// Run with a caller-provided backend, such as
    /// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend).
    pub fn run_with_backend<S, B, V, U>(
        &self,
        theme: &Theme,
        backend: B,
        state: &mut S,
        mut view: V,
        update: U,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error>,
        V: FnMut(&S, u64) -> Element,
        U: FnMut(&mut S, Signal) -> UpdateResult,
    {
        self.run_with_backend_inner(
            theme,
            backend,
            state,
            |state, frame, paint_root| {
                let root = view(state, frame);
                paint_root(root.as_ref());
            },
            update,
            None,
        )
    }

    /// Run a data-driven [`Application`] with a caller-provided backend.
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
        self.run_with_backend_inner(
            theme,
            backend,
            app,
            |app, frame, paint_root| {
                let root = app.view(frame);
                paint_root(root.as_ref());
            },
            Application::update,
            None,
        )
    }

    fn run_with_backend_inner<S, B, V, U>(
        &self,
        theme: &Theme,
        backend: B,
        state: &mut S,
        mut view: V,
        mut update: U,
        graphics: Option<&FrameGraphics>,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error>,
        V: FnMut(&S, u64, &mut dyn FnMut(&dyn View)),
        U: FnMut(&mut S, Signal) -> UpdateResult,
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
        let mut core = RunnerCore::new();
        let mut selection = RunnerSelection::new(self.selects_text());
        let mut last_tick = self.clock.now();

        if split {
            pin_footer(&mut terminal)?;
        }
        if let RunnerAction::Render(frame) = core.next_action() {
            draw(
                &mut terminal,
                theme,
                &mut view,
                state,
                frame,
                graphics,
                &mut selection,
            )?;
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
                // Publishing scrolls the terminal and may clear the viewport,
                // so a committed block always makes the footer dirty.
                if self.scrollback.flush(&mut terminal, theme)? {
                    core.request_redraw();
                    schedule_redraw(&mut redraw_at, now);
                }
            } else {
                self.scrollback.clear();
            }

            if now.saturating_duration_since(last_tick) >= self.config.tick_rate {
                last_tick = now;
                let result = update(state, Signal::Tick);
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
                        pin_footer(&mut terminal)?;
                    }
                    draw(
                        &mut terminal,
                        theme,
                        &mut view,
                        state,
                        frame,
                        graphics,
                        &mut selection,
                    )?;
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
            if event::poll(timeout)?
                && let Some(event) = translate_event(event::read()?)
            {
                let signal = Signal::Event(event);
                let requires_redraw = signal.requires_redraw();
                let selection_event = match &signal {
                    Signal::Event(event) => Some(event.clone()),
                    Signal::Tick => None,
                };
                let result = update(state, signal);
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

        // Some terminal emulators do not answer the cursor-position query used
        // by `clear`. Session restoration must still succeed and a cosmetic
        // cleanup failure must not turn a completed run into an application
        // error.
        if split {
            let _ = close_footer(&mut terminal);
        } else {
            let _ = terminal.clear();
        }
        Ok(())
    }
}

/// Paint one numbered frame from immutable state.
fn draw<S, B, V>(
    terminal: &mut Terminal<B>,
    theme: &Theme,
    view: &mut V,
    state: &S,
    frame: u64,
    graphics: Option<&FrameGraphics>,
    selection: &mut RunnerSelection,
) -> io::Result<()>
where
    B: Backend<Error = io::Error>,
    V: FnMut(&S, u64, &mut dyn FnMut(&dyn View)),
{
    let mut copied = None;
    terminal.draw(|terminal_frame| {
        let area = terminal_frame.area();
        let ctx = graphics.map_or_else(|| RenderCtx::new(theme), |g| g.render_context(theme));
        view(state, frame, &mut |root| {
            paint_with_context(terminal_frame.buffer_mut(), area, &ctx, root, &[]);
        });
        copied = selection.finish_frame(terminal_frame.buffer_mut(), area, theme);
    })?;
    if let Some(graphics) = graphics {
        graphics.finish_frame()?;
    }
    if let Some(text) = copied {
        let _ = clipboard::write(&mut io::stdout(), &text)?;
    }
    Ok(())
}

struct RunnerSelection {
    enabled: bool,
    state: SelectionState,
    pending_copy: bool,
}

impl RunnerSelection {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            state: SelectionState::new(),
            pending_copy: false,
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

        let changed = self.state.handle_with_clock(mouse, clock);
        if changed
            && matches!(mouse.kind, crate::MouseKind::Up(crate::MouseButton::Left))
            && self.state.range().is_some()
        {
            self.pending_copy = true;
        }
        changed
    }

    fn finish_frame(&mut self, buffer: &mut Buffer, area: Rect, theme: &Theme) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if self.state.resolve(buffer, area) {
            self.pending_copy = true;
        }
        let Some(range) = self.state.range() else {
            self.pending_copy = false;
            return None;
        };
        let copied = self
            .pending_copy
            .then(|| selected_text(buffer, area, range));
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
            ratatui_core::layout::Rect::new(2, 3, 4, 5),
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
        let mut buffer = crate::testing::render(
            &crate::components::Text::raw("hello world"),
            11,
            1,
            &Theme::default(),
        );
        let area = buffer.area;
        let down = crate::Mouse::at(crate::MouseKind::Down(crate::MouseButton::Left), 0, 0);
        let drag = crate::Mouse::at(crate::MouseKind::Drag(crate::MouseButton::Left), 4, 0);
        let up = crate::Mouse::at(crate::MouseKind::Up(crate::MouseButton::Left), 4, 0);

        assert!(!selection.handle_event(&Event::Mouse(down), UpdateResult::Clean, &clock));
        assert!(selection.handle_event(&Event::Mouse(drag), UpdateResult::Clean, &clock));
        assert!(selection.handle_event(&Event::Mouse(up), UpdateResult::Clean, &clock));

        let copied = selection.finish_frame(&mut buffer, area, &Theme::default());
        assert_eq!(copied.as_deref(), Some("hello"));
        for column in 0..=4 {
            assert_eq!(buffer[(column, 0)].bg, Theme::default().selection_bg);
        }
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
        assert!(Runner::new(RunnerConfig::default()).selects_text());
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
        handle.write(|_width| crate::element(crate::components::Text::raw("queued")));
        assert!(
            !runner.scrollback().is_empty(),
            "every handle sees the same queue"
        );
    }
}
