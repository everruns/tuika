//! The common surface in one import.
//!
//! ```
//! use tuika::prelude::*;
//! ```
//!
//! This is the framework spine re-exported from the crate root, plus every
//! component and the handful of per-module types an application reaches for
//! constantly: focus, keymap, animation, live values, the highlighter boundary, and
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
//! Both ownership forms of frame composition are included: [`Element`] owns a
//! boxed view, [`ScopedElement`] may borrow data anywhere in a base subtree,
//! and [`ScopedScene`] composes such a tree with owned overlays for one paint.

pub use crate::anim::{Easing, Repeat, Timeline, Transition};
pub use crate::components::*;
pub use crate::focus::FocusRegistry;
pub use crate::highlight::{CodeHighlighter, Highlighter, PlainHighlighter};
#[cfg(feature = "ratatui")]
pub use crate::interop::RatatuiView;
pub use crate::keymap::{Binding, Chord, Dispatch, Hint, KeySequence, Keymap, Layer};
pub use crate::live::{Live, LiveView, RedrawHandle};
pub use crate::style::{
    BorderStyle, CodeTheme, Gradient, Role, StyleBundle, StyleResolver, StyleRole, lerp_color,
};
pub use crate::ui::{Color, Line, Modifier, Rect, Span, Style};
// The `view!` macro plus the module it is named for; the macro expands to
// `$crate::…` paths, so a glob-importing host needs neither in scope by name.
pub use crate::view;
pub use crate::{
    Align, AlignContent, Application, AvailableSpace, Backdrop, Clock, Delivery, Dimension,
    Direction, DockEdge, DockLayout, DockPlacement, DockSpec, DockState, Element, Event, EventFlow,
    EventSource, FlexItemStyle, FlexWrap, FrameSource, InputOutcome, InputTarget, Justify, Key,
    KeyCode, LayoutStyle, MeasureRequest, Mouse, MouseButton, MouseCapture, MouseKind,
    OneShotOptions, Overlay, OverlaySpec, Padding, RenderCtx, RouteStage, Router, Runner,
    RunnerAction, RunnerConfig, RunnerCore, Scene, SceneOverlay, ScopedElement, ScopedScene,
    ScreenMode, Scrollback, SemanticRole, Signal, Size, StyleSheet, Surface, SystemClock,
    TargetAlign, TargetPlacement, TargetSide, TerminalSession, TerminalSessionConfig, Theme,
    UpdateResult, View, element, from_fn, paint, paint_scene, paint_with_context, paint_with_sheet,
    render_once, translate_event, view_fn, write_once,
};

#[cfg(feature = "async")]
pub use crate::{AsyncApplication, AsyncFrameSource, AsyncRunner, async_from_fn, no_messages};
