//! `tuika` — the application framework for Rust terminal UIs, built over
//! [`ratatui`](https://docs.rs/ratatui).
//!
//! Where ratatui owns the cell buffer and the widgets drawn into it, `tuika`
//! owns the layer an *application* needs above that, so a TUI starts as a
//! described screen rather than a render loop.
//!
//! `tuika` adds the pieces ratatui leaves to you — a flexbox-style layout
//! solver, anchored overlays, focus/input-ownership, a host for either screen
//! mode (alternate screen or a split footer over live terminal scrollback),
//! and a set of components (text, boxes, scroll, select, spinner, progress) —
//! while letting ratatui keep ownership of the cell buffer and its diff against
//! the terminal. It builds against `ratatui-core` directly rather than the
//! `ratatui` umbrella — it renders none of ratatui's own widgets — so
//! `ratatui-widgets` and `ratatui-macros` stay out of its dependency tree, and
//! it writes to the terminal through its own crossterm backend rather than
//! `ratatui-crossterm`. It otherwise depends only on `crossterm`,
//! `unicode-segmentation`, `unicode-width`, and `pulldown-cmark`.
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
//!   anchor over the base tree; the **host** ([`host`]) owns the screen
//!   ([`screen::ScreenMode`] — the alternate screen, or a split footer over live
//!   terminal scrollback), translates crossterm input, and composites the frame.
//! - **Scene composition** is owned with [`Scene`], or borrows a host-state view
//!   for one frame with [`ScopedScene`]; both use the same ordered overlay and
//!   focus semantics.
//! - **Input routing** ([`routing`]) delivers an event to whichever surface owns
//!   input this frame, every event kind through one registration.
//!
//! # Finding things
//!
//! The crate root re-exports the **framework**: the view model, layout,
//! events, styling, and the host boundary — the types you compose *with*. The
//! widgets themselves live in [`components`]. Owned [`Element`] trees use
//! [`Scene`]; [`ScopedElement`] trees may borrow application state at any depth
//! and use [`ScopedScene`] without cloning it. Everything that talks to the
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
//! [`components`]. No registration step; containers accept owned or
//! frame-borrowed views. One-off custom regions can use [`view_fn`] to provide
//! the same measurement and rendering contract inline.
//!
//! Existing ratatui widgets should normally be wrapped in
//! [`RatatuiView`](interop::RatatuiView), which preserves Tuika clipping without
//! exposing the frame buffer. [`TerminalSession`] and [`runner::Runner`] are
//! optional host-side lifecycle helpers; with `feature = "async"`,
//! [`runner::AsyncRunner`] is the same loop for hosts that already
//! run on Tokio.

#![warn(missing_docs)]
// On docs.rs (nightly, `--cfg docsrs`) annotate feature-gated items with the
// feature that enables them. A no-op on stable builds. `doc_auto_cfg` was
// merged into `doc_cfg` in 1.92 and removed as a name, so gating on the old
// one is a hard rustdoc error on current nightly — which is what silently
// left 0.4.0 undocumented on docs.rs.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod anim;
pub mod buffer;
mod clock;
pub mod components;
pub mod dock;
pub mod event;
#[macro_use]
mod macros;
pub mod focus;
pub mod framebuffer;
pub mod geometry;
pub mod highlight;
pub mod host;
#[cfg(feature = "ratatui")]
#[cfg_attr(docsrs, doc(cfg(feature = "ratatui")))]
pub mod interop;
pub mod keymap;
pub mod layout;
pub mod live;
pub mod mouse;
mod output;
pub mod overlay;
pub mod prelude;
pub mod probe;
pub mod routing;
pub mod runner;
pub mod scene;
pub mod screen;
pub mod style;
pub mod surface;
pub mod term;
pub mod testing;
pub mod text;
pub mod themes;
pub mod view;
pub mod width;

/// The cell-grid vocabulary, in one place for custom [`View`] implementations.
///
/// These are tuika's own types; nothing here is a re-export of another crate,
/// so a host writing a `View` takes no rendering dependency beyond tuika
/// itself. Each also has a canonical home module ([`geometry`], [`style`],
/// [`text`], [`buffer`]) — this module is the convenience path, and what
/// [`prelude`] pulls in.
pub mod ui {
    pub use crate::buffer::{Buffer, Cell, CellDiffOption};
    pub use crate::geometry::{Position, Rect};
    pub use crate::style::{Color, Modifier, Style};
    pub use crate::text::{Alignment, Line, Span};
}

// The framework spine: the types a host composes with on essentially every
// frame. Widgets are not here on purpose — they live in `components`, and
// `prelude` is the one-line import that brings both. Anything reachable only
// through its module (`themes::by_name`, `probe::RectProbe`, `term::clipboard`)
// is deliberately not flattened: a shallow path is worth something only if the
// name earns it.
pub use clock::{Clock, SystemClock};
pub use dock::{DockEdge, DockLayout, DockPlacement, DockSpec, DockState};
pub use event::{Event, EventFlow, InputOutcome, Key, KeyCode, Mouse, MouseButton, MouseKind};
pub use geometry::{Padding, Size};
pub use host::{
    MouseCapture, TerminalSession, TerminalSessionConfig, paint, paint_scene, paint_with_context,
    paint_with_sheet, translate_event,
};
pub use layout::{
    Align, AlignContent, Dimension, Direction, FlexItemStyle, FlexLine, FlexWrap, Item, Justify,
    LayoutResult, LayoutStyle, solve, solve_layout,
};
pub use output::{OneShotOptions, render_once, write_once};
pub use overlay::{Overlay, OverlaySpec, TargetAlign, TargetPlacement, TargetSide};
pub use routing::{Delivery, InputTarget, RouteStage, Router};
#[cfg(feature = "async")]
#[allow(deprecated)]
pub use runner::AsyncSignal;
pub use runner::{
    Application, EventSource, FrameSource, Runner, RunnerAction, RunnerConfig, RunnerCore, Signal,
    UpdateResult, from_fn, scripted_events,
};
#[cfg(feature = "async")]
pub use runner::{AsyncApplication, AsyncFrameSource, AsyncRunner, async_from_fn, no_messages};
pub use scene::{Backdrop, Scene, SceneOverlay, ScopedScene};
pub use screen::{ScreenMode, Scrollback};
pub use style::{SemanticRole, StyleResolver, StyleRole, StyleSheet, Theme};
pub use surface::Surface;
pub use view::{
    AvailableSpace, Element, MeasureRequest, RenderCtx, ScopedElement, View, element, view_fn,
};

#[cfg(test)]
mod tests;
