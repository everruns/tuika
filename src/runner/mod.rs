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
pub use asynchronous::{AsyncRunner, Signal};

use std::io;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use crossterm::event;
use ratatui_core::backend::Backend;
use ratatui_core::terminal::{Terminal, TerminalOptions};
use ratatui_crossterm::CrosstermBackend;

use crate::live::RedrawHandle;
use crate::screen::{ScreenMode, Scrollback, close_footer, pin_footer};
use crate::{Element, Event, TerminalSession, Theme, paint, translate_event};

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

/// A synchronous Crossterm event and rendering loop.
pub struct Runner {
    config: RunnerConfig,
    redraw: RedrawHandle,
    scrollback: Scrollback,
}

impl Runner {
    /// Create a runner without touching the terminal.
    pub fn new(mut config: RunnerConfig) -> Self {
        // A zero interval would busy-spin even when the application has no
        // events or updates. Keep the public config ergonomic while enforcing
        // a safe scheduling floor at the boundary.
        config.tick_rate = config.tick_rate.max(Duration::from_millis(1));
        Self {
            config,
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

    /// Run until `on_event` returns [`ControlFlow::Break`].
    pub fn run(
        &self,
        theme: &Theme,
        build: impl FnMut(u64) -> Element,
        on_event: impl FnMut(Event) -> ControlFlow<()>,
    ) -> io::Result<()> {
        self.run_with_backend(theme, CrosstermBackend::new(io::stdout()), build, on_event)
    }

    /// Run with a caller-provided backend, such as
    /// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend).
    pub fn run_with_backend<B>(
        &self,
        theme: &Theme,
        backend: B,
        mut build: impl FnMut(u64) -> Element,
        mut on_event: impl FnMut(Event) -> ControlFlow<()>,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error>,
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
        let mut last_draw = Instant::now();
        self.redraw.request();

        loop {
            if split {
                // Publishing scrolls the terminal and (on the portable path)
                // clears the viewport, so a block that went out means the footer
                // owes a repaint on this iteration rather than at the next tick.
                if self.scrollback.flush(&mut terminal, theme)? {
                    self.redraw.request();
                }
            } else {
                self.scrollback.clear();
            }

            let due = last_draw.elapsed() >= self.config.tick_rate;
            let requested = self.redraw.take();
            if due || requested {
                if split {
                    // Resolve any resize first so the footer is pinned against
                    // the new screen height, not the previous one.
                    terminal.autoresize()?;
                    pin_footer(&mut terminal)?;
                }
                terminal.draw(|terminal_frame| {
                    let area = terminal_frame.area();
                    let root = build(frame);
                    paint(terminal_frame.buffer_mut(), area, theme, root.as_ref(), &[]);
                })?;
                frame = frame.wrapping_add(1);
                last_draw = Instant::now();
            }

            let timeout = self.config.tick_rate.saturating_sub(last_draw.elapsed());
            if event::poll(timeout)?
                && let Some(event) = translate_event(event::read()?)
                && on_event(event).is_break()
            {
                break;
            }
        }

        // Some terminal emulators do not answer the cursor-position query used
        // by `clear`. Session restoration must still succeed and a cosmetic
        // cleanup failure must not turn a completed run into an application
        // error. A split footer gives its rows back instead: the scrollback
        // above it is the user's, and only the reserved region is ours to
        // clear.
        if split {
            let _ = close_footer(&mut terminal);
        } else {
            let _ = terminal.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_tick_rate_is_clamped() {
        let runner = Runner::new(RunnerConfig {
            tick_rate: Duration::ZERO,
            ..RunnerConfig::default()
        });
        assert_eq!(runner.config.tick_rate, Duration::from_millis(1));
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
