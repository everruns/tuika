//! [`Toasts`] — a transient notification stack with frame-driven expiry.
//!
//! tuika owns no clock, so toasts expire by *ticking*: each notification carries
//! a remaining lifetime in frames, [`Toasts::tick`] decrements them once per
//! frame (or per whatever step the host drives), and a toast is dropped when its
//! lifetime hits zero. The host keeps a [`Toasts`] in its persisted state, calls
//! [`push`](Toasts::push) to enqueue, `tick` each frame, and renders a
//! [`ToastList`] over its UI — usually in an [`OverlaySpec`](crate::OverlaySpec)
//! corner. This is the notification analog of OpenTUI's toast system, without a
//! scheduler.

use crate::geometry::Rect;
use crate::style::{Color, Modifier};

use crate::geometry::Size;
use crate::style::StyleRole;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

/// Severity of a toast, which selects its semantic style role and leading glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLevel {
    /// Neutral information.
    Info,
    /// A successful action.
    Success,
    /// A caution.
    Warning,
    /// A failure.
    Error,
}

impl ToastLevel {
    /// The conventional accent color for this level (independent of theme hue,
    /// like [`DiffStyle`](crate::components::DiffStyle)).
    pub fn color(self) -> Color {
        match self {
            ToastLevel::Info => Color::Rgb(90, 150, 210),
            ToastLevel::Success => Color::Rgb(120, 190, 120),
            ToastLevel::Warning => Color::Rgb(220, 180, 90),
            ToastLevel::Error => Color::Rgb(220, 110, 110),
        }
    }

    /// A short leading glyph for this level.
    pub fn glyph(self) -> char {
        match self {
            ToastLevel::Info => 'ℹ',
            ToastLevel::Success => '✔',
            ToastLevel::Warning => '⚠',
            ToastLevel::Error => '✖',
        }
    }

    fn style_role(self) -> StyleRole {
        match self {
            ToastLevel::Info => StyleRole::TOAST_INFO,
            ToastLevel::Success => StyleRole::TOAST_SUCCESS,
            ToastLevel::Warning => StyleRole::TOAST_WARNING,
            ToastLevel::Error => StyleRole::TOAST_ERROR,
        }
    }
}

/// One live notification.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Toast {
    message: String,
    level: ToastLevel,
    /// Frames of life remaining; `None` means it never expires on its own.
    remaining: Option<u64>,
}

/// Default frames a toast lives before expiring (about 4s at 30fps).
pub const DEFAULT_TTL: u64 = 120;

/// A host-owned stack of active toasts (newest first).
///
/// ```
/// use tuika::prelude::*;
/// let mut toasts = Toasts::new(4);
/// toasts.push_for(ToastLevel::Success, "Saved", Some(2));
/// assert_eq!(toasts.len(), 1);
/// toasts.tick(); // 2 → 1
/// assert_eq!(toasts.tick(), 1); // 1 → 0, dropped
/// assert!(toasts.is_empty());
/// ```
#[derive(Clone, Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    max: usize,
}

impl Toasts {
    /// An empty stack keeping at most `max` toasts (older ones drop when the
    /// stack overflows). `max` of 0 is treated as 1.
    pub fn new(max: usize) -> Self {
        Self {
            items: Vec::new(),
            max: max.max(1),
        }
    }

    /// Push a notification that expires after [`DEFAULT_TTL`] frames.
    pub fn push(&mut self, level: ToastLevel, message: impl Into<String>) {
        self.push_for(level, message, Some(DEFAULT_TTL));
    }

    /// Push a notification with an explicit lifetime: `Some(frames)` to expire
    /// after that many [`tick`](Self::tick)s, or `None` to persist until
    /// [`dismiss_oldest`](Self::dismiss_oldest) / [`clear`](Self::clear).
    pub fn push_for(&mut self, level: ToastLevel, message: impl Into<String>, ttl: Option<u64>) {
        self.items.insert(
            0,
            Toast {
                message: message.into(),
                level,
                remaining: ttl,
            },
        );
        // Keep only the newest `max`.
        self.items.truncate(self.max.max(1));
    }

    /// Advance every toast by one frame, dropping any whose lifetime reached
    /// zero. Call once per rendered frame. Returns how many expired this tick.
    pub fn tick(&mut self) -> usize {
        let before = self.items.len();
        for t in &mut self.items {
            if let Some(r) = t.remaining {
                t.remaining = Some(r.saturating_sub(1));
            }
        }
        self.items
            .retain(|t| t.remaining.map(|r| r > 0).unwrap_or(true));
        before - self.items.len()
    }

    /// Remove the oldest toast (bottom of the stack), if any.
    pub fn dismiss_oldest(&mut self) {
        self.items.pop();
    }

    /// Remove every toast.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Whether there are no active toasts.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of active toasts.
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// A vertical stack of toast rows derived from [`Toasts`], newest at the top.
///
/// Each toast is a single row: a level-colored bar and glyph, then the message
/// over the theme's surface fill. Place it in a corner overlay (e.g.
/// [`Anchor::BottomRight`](crate::overlay::Anchor)); it fills the width it is given and is
/// as tall as there are toasts.
pub struct ToastList<'a> {
    toasts: &'a Toasts,
}

impl<'a> ToastList<'a> {
    /// A view over `toasts` for this frame.
    pub fn new(toasts: &'a Toasts) -> Self {
        Self { toasts }
    }
}

impl View for ToastList<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let width = self
            .toasts
            .items
            .iter()
            .map(|t| crate::width::str_cols(&t.message).saturating_add(4))
            .max()
            .unwrap_or(0)
            .min(available.width);
        Size::new(
            width,
            (self.toasts.items.len() as u16).min(available.height),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let fill = ctx.style(StyleRole::TOAST).to_style();
        for (row, toast) in self.toasts.items.iter().enumerate() {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            // Surface fill across the whole row.
            for x in area.x..area.right() {
                surface.set(x, y, ' ', fill);
            }
            let accent = ctx.style(toast.level.style_role()).apply(fill);
            let mut x = area.x;
            x = surface.set_string(x, y, "▌", accent.add_modifier(Modifier::BOLD));
            x = surface.set_string(x, y, &format!("{} ", toast.level.glyph()), accent);
            surface.set_string(x, y, &toast.message, fill);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Theme;
    use crate::tests::support::row;

    #[test]
    fn newest_pushes_to_the_top_and_max_evicts_oldest() {
        let mut t = Toasts::new(2);
        t.push(ToastLevel::Info, "first");
        t.push(ToastLevel::Success, "second");
        t.push(ToastLevel::Error, "third");
        assert_eq!(t.len(), 2, "capped at max");
        // Newest first; "first" was evicted.
        assert_eq!(t.items[0].message, "third");
        assert_eq!(t.items[1].message, "second");
    }

    #[test]
    fn tick_expires_after_ttl() {
        let mut t = Toasts::new(4);
        t.push_for(ToastLevel::Warning, "brief", Some(2));
        t.push_for(ToastLevel::Info, "sticky", None);
        assert_eq!(t.tick(), 0); // 2 -> 1
        assert_eq!(t.tick(), 1); // 1 -> 0, dropped
        assert_eq!(t.len(), 1, "the sticky toast persists");
        // Sticky never expires on its own.
        for _ in 0..1000 {
            t.tick();
        }
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn dismiss_and_clear() {
        let mut t = Toasts::new(4);
        t.push(ToastLevel::Info, "a");
        t.push(ToastLevel::Info, "b");
        t.dismiss_oldest(); // removes "a"
        assert_eq!(t.len(), 1);
        assert_eq!(t.items[0].message, "b");
        t.clear();
        assert!(t.is_empty());
    }

    #[test]
    fn renders_stack_with_level_colors() {
        let theme = Theme::default();
        let mut t = Toasts::new(4);
        t.push(ToastLevel::Info, "saved");
        t.push(ToastLevel::Error, "boom");
        let buf = crate::testing::render(&ToastList::new(&t), 20, 2, &theme);
        // Newest ("boom") on top with the error bar color.
        assert!(row(&buf, 0).contains("boom"));
        assert!(row(&buf, 1).contains("saved"));
        assert_eq!(
            buf[(0, 0)].fg,
            theme.semantic_color(crate::SemanticRole::Danger)
        );
    }

    #[test]
    fn toast_roles_follow_the_stylesheet() {
        let theme = Theme::default();
        let mut toasts = Toasts::new(1);
        toasts.push(ToastLevel::Error, "boom");
        let sheet = crate::StyleSheet {
            toast: crate::style::StyleBundle::new()
                .fg(Color::Cyan)
                .bg(Color::Blue),
            toast_error: crate::style::StyleBundle::new().fg(Color::Yellow),
            ..crate::StyleSheet::from_theme(&theme)
        };
        let buffer =
            crate::testing::render_with_sheet(&ToastList::new(&toasts), 12, 1, &theme, sheet);
        assert_eq!(buffer[(0, 0)].fg, Color::Yellow);
        assert_eq!(buffer[(0, 0)].bg, Color::Blue);
        assert_eq!(buffer[(4, 0)].fg, Color::Cyan);
    }

    #[test]
    fn empty_and_tiny_areas_do_not_panic() {
        let theme = Theme::default();
        let empty = Toasts::new(4);
        assert_eq!(
            ToastList::new(&empty)
                .measure(Size::new(40, 10), &RenderCtx::new(&theme))
                .height,
            0
        );
        let mut t = Toasts::new(4);
        t.push(ToastLevel::Info, "a very long notification message");
        for (w, h) in [(0u16, 0u16), (1, 1), (3, 1)] {
            let _ = crate::testing::render(&ToastList::new(&t), w, h, &theme);
        }
    }
}
