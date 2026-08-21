//! Terminal capabilities that live outside the cell grid.
//!
//! Everything above this module works in cells: a view measures in cells, paints
//! through a [`Surface`](crate::surface::Surface), and ratatui diffs the result.
//! A handful of terminal features do not fit that model at all — a clipboard
//! write, a hyperlink, the window's own progress indicator, the mouse pointer's
//! shape. They are *out-of-band*: the host writes an escape sequence directly to
//! the stream, the terminal acts on it, and no cell changes.
//!
//! Grouping them here is the point. They share one shape, so they should share
//! one place:
//!
//! - a pure `encode` that builds the sequence and does no I/O, so it is
//!   unit-testable without a terminal, and
//! - a thin writer over `impl Write` for hosts that just want it sent.
//!
//! | Module | Sequence | What the terminal does |
//! | --- | --- | --- |
//! | [`clipboard`] | OSC 52 | puts text on the system clipboard |
//! | [`hyperlink`] | OSC 8 | makes a text run clickable |
//! | [`progress`] | OSC 9;4 | lights its own progress bar or taskbar |
//! | [`mod@pointer`] | OSC 22 | changes the mouse pointer shape |
//! | [`capabilities`] | DA1 | answers what it supports |
//! | [`palette`] | OSC 10 / 11 / 4 | reports the colors it was configured with |
//!
//! Terminals silently swallow an OSC they do not understand, so emitting one is
//! safe everywhere; [`capabilities`] exists for the cases where the host wants to
//! know before it commits to a richer rendering path (pixels versus text).
//!
//! The last two are a shape of their own: they *ask* rather than tell, so their
//! answer arrives on stdin among the user's keystrokes. That is why both are
//! host-initiated, run once at startup in raw mode, and share a single fenced
//! round-trip — see [`capabilities::Capabilities::query_with_palette`].

pub mod backend;
pub mod capabilities;
pub mod clipboard;
pub mod hyperlink;
pub mod image;
pub mod palette;
pub mod pointer;
pub mod progress;
pub mod terminal;
pub mod testbackend;
pub mod traits;
