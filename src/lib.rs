//! `tuika` — a small composable terminal UI toolkit over
//! [`ratatui`](https://docs.rs/ratatui).
//!
//! `tuika` adds the pieces ratatui leaves to you — a flexbox-style layout
//! solver, anchored overlays, focus/input-ownership, an alternate-screen host,
//! and a set of components (text, boxes, scroll, select, spinner, progress) —
//! while letting ratatui keep ownership of the cell buffer and its diff against
//! the terminal. It builds against `ratatui-core` (and `ratatui-crossterm` for
//! the backend) directly rather than the `ratatui` umbrella — it renders none of
//! ratatui's own widgets — so `ratatui-widgets` and `ratatui-macros` stay out of
//! its dependency tree. It otherwise depends only on `crossterm`, `textwrap`,
//! `unicode-segmentation`, and `unicode-width`.
//!
//! It was extracted from the [yolop](https://github.com/everruns/yolop) coding
//! agent, whose full-screen renderer is built on it, but it knows nothing about
//! any host application.
//!
//! # Model
//!
//! - **Views** ([`view::View`]) are ephemeral, rebuilt from application state
//!   each frame; ratatui diffs the resulting cell buffer, so this is cheap.
//! - **State** that must persist across frames ([`components::ScrollState`],
//!   [`components::SelectState`], [`focus::FocusRegistry`]) lives in the host,
//!   in the `StatefulWidget` idiom.
//! - **Layout** is a flexbox subset ([`layout`]); **overlays** ([`overlay`])
//!   anchor over the base tree; the **host** ([`host`]) owns the alternate
//!   screen, translates crossterm input, and composites the frame.
//!
//! # Finding things
//!
//! The crate root re-exports the **framework**: the view model, layout,
//! events, styling, and the host seam — the types you compose *with*. The
//! widgets themselves live in [`components`], and everything that talks to the
//! terminal outside the cell grid (clipboard, hyperlinks, images, native
//! progress, capability detection) lives in [`term`].
//!
//! For application code that wants the common surface in one line, glob-import
//! [`prelude`]:
//!
//! ```
//! use tuika::prelude::*;
//!
//! let screen = element(Flex::column().fixed(1, element(Text::raw("hello"))));
//! # let _ = screen;
//! ```
//!
//! # Extending
//!
//! Add a component by implementing [`view::View`] in a new module under
//! [`components`]. No registration step; containers accept any boxed `View`.
//!
//! Existing ratatui widgets should normally be wrapped in
//! [`RatatuiView`](interop::RatatuiView), which preserves Tuika clipping without
//! exposing the frame buffer. [`TerminalSession`] and [`runner::Runner`] are
//! optional host-side lifecycle helpers; with `feature = "async"`,
//! [`runner::AsyncRunner`] is the same loop for hosts that already
//! run on Tokio.

#![warn(missing_docs)]
// On docs.rs (nightly, `--cfg docsrs`) annotate feature-gated items with the
// feature that enables them. A no-op on stable builds.
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod anim;
pub mod components;
pub mod event;
#[macro_use]
mod macros;
pub mod focus;
pub mod framebuffer;
pub mod geometry;
pub mod highlight;
pub mod host;
pub mod interop;
pub mod keymap;
pub mod layout;
pub mod live;
pub mod mouse;
pub mod overlay;
pub mod prelude;
pub mod probe;
pub mod runner;
pub mod style;
pub mod surface;
pub mod term;
pub mod testing;
pub mod themes;
pub mod view;
pub mod width;

// The framework spine: the types a host composes with on essentially every
// frame. Widgets are not here on purpose — they live in `components`, and
// `prelude` is the one-line import that brings both. Anything reachable only
// through its module (`themes::by_name`, `probe::RectProbe`, `term::clipboard`)
// is deliberately not flattened: a shallow path is worth something only if the
// name earns it.
pub use event::{Event, EventFlow, Key, KeyCode, Mouse, MouseButton, MouseKind};
pub use geometry::{Padding, Size};
pub use host::{TerminalSession, paint, paint_with_sheet, translate_event};
pub use layout::{Align, Dimension, Direction, Justify, LayoutStyle};
pub use overlay::{Overlay, OverlaySpec};
#[cfg(feature = "async")]
pub use runner::{AsyncRunner, Signal};
pub use runner::{Runner, RunnerConfig};
pub use style::{StyleSheet, Theme};
pub use surface::Surface;
pub use view::{Element, RenderCtx, View, element};

#[cfg(test)]
mod tests;
