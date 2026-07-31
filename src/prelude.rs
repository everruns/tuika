//! The common surface in one import.
//!
//! ```
//! use tuika::prelude::*;
//! ```
//!
//! This is the framework spine re-exported from the crate root, plus every
//! component and the handful of per-module types an application reaches for
//! constantly: focus, keymap, animation, live values, the highlighter seam, and
//! the ratatui interop wrapper.
//!
//! What is *not* here is as deliberate as what is. Terminal escapes
//! ([`term`](crate::term)), pixel canvases ([`framebuffer`](crate::framebuffer)),
//! rect probes ([`probe`](crate::probe)), width measurement
//! ([`width`](crate::width)), and the bundled palettes
//! ([`themes`](crate::themes)) stay behind their module path — they are used in
//! one or two places in a host, where an explicit path documents the call better
//! than a glob does.
//!
//! A glob import can collide with names from another crate — ratatui's own
//! `Text` and this crate's [`components::Text`](crate::components::Text) are the obvious
//! pair. When that happens, import the two you need by path instead; the glob is
//! a convenience, never a requirement.
//!
//! Both ownership forms of frame composition are included: [`Scene`] owns its
//! root, while [`ScopedScene`] borrows a concrete root that reads host-owned
//! data for the duration of one paint.

pub use crate::anim::{Easing, Repeat, Timeline};
pub use crate::components::*;
pub use crate::focus::FocusRegistry;
pub use crate::highlight::{CodeHighlighter, Highlighter, PlainHighlighter};
pub use crate::interop::RatatuiView;
pub use crate::keymap::{Binding, Chord, Dispatch, Hint, KeySequence, Keymap, Layer};
pub use crate::live::{Live, LiveView, RedrawHandle};
pub use crate::style::{BorderStyle, CodeTheme, Role, StyleBundle};
// The `view!` macro plus the module it is named for; the macro expands to
// `$crate::…` paths, so a glob-importing host needs neither in scope by name.
pub use crate::view;
pub use crate::{
    Align, Backdrop, Dimension, Direction, Element, Event, EventFlow, Justify, Key, KeyCode,
    LayoutStyle, Mouse, MouseButton, MouseKind, Overlay, OverlaySpec, Padding, RenderCtx, Runner,
    RunnerConfig, Scene, SceneOverlay, ScopedScene, ScreenMode, Scrollback, SemanticRole, Signal,
    Size, StyleSheet, Surface, TerminalSession, Theme, UpdateResult, View, element, paint,
    paint_scene, paint_with_sheet, translate_event,
};

#[cfg(feature = "async")]
pub use crate::AsyncRunner;
