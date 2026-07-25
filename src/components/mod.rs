//! Component library.
//!
//! Every component implements [`View`](crate::view::View). Layout
//! containers ([`Flex`], [`Boxed`], [`ItemScroll`]) nest children; leaves
//! ([`Text`], [`Paragraph`], [`SelectList`], [`Table`], [`Scroll`],
//! [`StatusBar`], [`Spacer`]) paint content. Interactive leaves pair with a persisted `*State`
//! (see [`ScrollState`], [`SelectState`] — a [`Table`] reuses `SelectState`)
//! held by the host. To add a component, drop
//! a new module here and implement `View`; nothing else needs to change.
//!
//! # Where things live
//!
//! Every component type is re-exported flat, so `components::Scroll` is the one
//! canonical path to a component — there is no `components::scroll::Scroll` to
//! choose between. A component's own module is public only when it owns
//! something the flat namespace cannot name well: a constant or a free function
//! whose meaning depends on which component it belongs to. Those stay in their
//! module ([`markdown::to_lines`], [`toast::DEFAULT_TTL`]) instead of being
//! flattened into a hand-prefixed alias.

pub mod ascii_font;
mod boxed;
pub(crate) mod code_block;
pub mod console;
mod constrained;
mod dialog;
pub mod diff;
mod flex;
mod focus_scope;
mod form;
mod image;
mod item_scroll;
mod key_hints;
mod loader;
pub mod markdown;
mod progress_bar;
pub mod qr;
mod responsive;
mod rule;
mod scroll;
mod select;
mod slider;
mod spacer;
mod spinner;
mod status_bar;
mod tab_select;
mod table;
mod tabs;
pub mod text;
mod textinput;
pub mod toast;
mod viewport;

pub use ascii_font::AsciiFont;
pub use boxed::Boxed;
pub use code_block::CodeBlock;
pub use console::{Console, ConsoleLog};
pub use constrained::Constrained;
pub use dialog::Dialog;
pub use diff::{Diff, DiffMode, DiffRow, DiffStyle, DiffTag};
pub use flex::Flex;
pub use focus_scope::FocusScope;
pub use form::{Form, FormField, FormOutcome, FormState};
pub use image::Image;
pub use item_scroll::ItemScroll;
pub use key_hints::KeyHints;
pub use loader::Loader;
pub use markdown::{FencedBlockRenderer, ImageResolver, Markdown, MarkdownImage, MarkdownState};
pub use progress_bar::ProgressBar;
pub use qr::{QrCode, QrEcc};
pub use responsive::Responsive;
pub use rule::Rule;
pub use scroll::{Scroll, ScrollState};
pub use select::{SelectList, SelectOutcome, SelectState};
pub use slider::{Slider, SliderState};
pub use spacer::Spacer;
pub use spinner::{Spinner, SpinnerStyle};
pub use status_bar::StatusBar;
pub use tab_select::{TabSelect, TabSelectOutcome, TabSelectState};
pub use table::{Column, Table};
pub use tabs::{Tabs, TabsState};
pub use text::{Paragraph, Text, Wrap};
pub use textinput::{
    TextInput, TextInputEvent, TextInputMode, TextInputState, TextSpan, Token, Trigger,
    TriggerAnchor,
};
pub use toast::{ToastLevel, ToastList, Toasts};
pub use viewport::Viewport;
