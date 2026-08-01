//! Small full-screen run loops.
//!
//! Both runners tie the same host primitives together — [`TerminalSession`] for
//! the screen, [`translate_event`] for input, [`paint`] for the frame — so a
//! host that wants lifecycle, redraw scheduling, and event translation in one
//! place does not have to assemble them itself. Either [`ScreenMode`] works:
//! pick one in [`RunnerConfig::screen_mode`] and the loop reserves, keeps, and
//! releases a split footer for you.
//!
//! [`Runner`] is the synchronous loop and is always available. [`AsyncRunner`]
//! (`feature = "async"`) is the same loop for hosts already on Tokio; it lives
//! behind the feature so a sync-only host never pulls a runtime into its build.
//! They are one module because they are one concept — picking between them is a
//! question about the host's existing runtime, not about which part of tuika to
//! reach for.
//!
//! A host with its own event loop needs neither: call [`paint`] directly.

#[cfg(feature = "async")]
mod asynchronous;

#[cfg(feature = "async")]
pub use asynchronous::AsyncRunner;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event;
use ratatui_core::backend::Backend;
use ratatui_core::terminal::{Terminal, TerminalOptions};
use ratatui_crossterm::CrosstermBackend;

use crate::live::RedrawHandle;
use crate::screen::{ScreenMode, Scrollback, close_footer, pin_footer};
use crate::{Clock, Element, Event, SystemClock, TerminalSession, Theme, paint, translate_event};

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
    /// A translated terminal input event arrived.
    Event(Event),
}

/// What a runner should do after an update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateResult {
    /// Keep waiting without rebuilding or repainting the view.
    #[default]
    Clean,
    /// Rebuild and repaint the view from the updated state.
    Dirty,
    /// Stop the runner without painting another frame.
    Exit,
}

/// A synchronous Crossterm event and rendering loop.
pub struct Runner {
    config: RunnerConfig,
    clock: Arc<dyn Clock + Send + Sync>,
    redraw: RedrawHandle,
    scrollback: Scrollback,
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

    /// Run until `update` returns [`UpdateResult::Exit`].
    ///
    /// The runner paints once initially. It then delivers input and periodic
    /// [`Signal::Tick`] values to `update`, repainting only when `update`
    /// returns [`UpdateResult::Dirty`] or a [`RedrawHandle`] requests it.
    pub fn run<S, V, U>(&self, theme: &Theme, state: &mut S, view: V, update: U) -> io::Result<()>
    where
        V: FnMut(&S, u64) -> Element,
        U: FnMut(&mut S, Signal) -> UpdateResult,
    {
        self.run_with_backend(
            theme,
            CrosstermBackend::new(io::stdout()),
            state,
            view,
            update,
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
        mut update: U,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error>,
        V: FnMut(&S, u64) -> Element,
        U: FnMut(&mut S, Signal) -> UpdateResult,
    {
        let mode = self.config.screen_mode;
        let split = !mode.is_alternate();
        let _session = TerminalSession::enter_with(mode)?;
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: mode.viewport(),
            },
        )?;
        let mut frame = 0u64;
        let mut last_tick = self.clock.now();

        if split {
            pin_footer(&mut terminal)?;
        }
        draw(&mut terminal, theme, &mut view, state, &mut frame)?;

        'running: loop {
            let mut dirty = self.redraw.take();
            if split {
                // Publishing scrolls the terminal and may clear the viewport,
                // so a committed block always makes the footer dirty.
                dirty |= self.scrollback.flush(&mut terminal, theme)?;
            } else {
                self.scrollback.clear();
            }

            let now = self.clock.now();
            if now.saturating_duration_since(last_tick) >= self.config.tick_rate {
                last_tick = now;
                if apply_update(update(state, Signal::Tick), &mut dirty) {
                    break;
                }
            }

            if dirty {
                if split {
                    terminal.autoresize()?;
                    pin_footer(&mut terminal)?;
                }
                draw(&mut terminal, theme, &mut view, state, &mut frame)?;
            }

            let elapsed = self.clock.now().saturating_duration_since(last_tick);
            let timeout = self.config.tick_rate.saturating_sub(elapsed);
            if event::poll(timeout)?
                && let Some(event) = translate_event(event::read()?)
            {
                let mut event_dirty = false;
                if apply_update(update(state, Signal::Event(event)), &mut event_dirty) {
                    break 'running;
                }
                if event_dirty {
                    self.redraw.request();
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

fn apply_update(result: UpdateResult, dirty: &mut bool) -> bool {
    match result {
        UpdateResult::Clean => false,
        UpdateResult::Dirty => {
            *dirty = true;
            false
        }
        UpdateResult::Exit => true,
    }
}

/// Paint one frame from immutable state and advance the animation frame.
fn draw<S, B, V, Er>(
    terminal: &mut Terminal<B>,
    theme: &Theme,
    view: &mut V,
    state: &S,
    frame: &mut u64,
) -> Result<(), Er>
where
    B: Backend<Error = Er>,
    V: FnMut(&S, u64) -> Element,
{
    terminal.draw(|terminal_frame| {
        let area = terminal_frame.area();
        let root = view(state, *frame);
        paint(terminal_frame.buffer_mut(), area, theme, root.as_ref(), &[]);
    })?;
    *frame = frame.wrapping_add(1);
    Ok(())
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
    fn clean_updates_do_not_request_a_repaint() {
        let mut dirty = false;
        assert!(!apply_update(UpdateResult::Clean, &mut dirty));
        assert!(!dirty);
    }

    #[test]
    fn dirty_updates_request_a_repaint() {
        let mut dirty = false;
        assert!(!apply_update(UpdateResult::Dirty, &mut dirty));
        assert!(dirty);
    }

    #[test]
    fn exit_updates_stop_without_forcing_a_repaint() {
        let mut dirty = false;
        assert!(apply_update(UpdateResult::Exit, &mut dirty));
        assert!(!dirty);
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
    fn the_default_config_owns_the_alternate_screen() {
        assert_eq!(RunnerConfig::default().screen_mode, ScreenMode::Alternate);
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
