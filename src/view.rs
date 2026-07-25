//! The `View` trait — the extensibility seam for components.
//!
//! Design note (why not a React-style reconciler): `tuika` rebuilds the view
//! tree from application state every frame, which is cheap because ratatui
//! already diffs the resulting cell buffer against the terminal. So views are
//! ephemeral *render descriptions* (immediate-mode structure), while the
//! interactive state that must survive across frames — scroll offset, current
//! selection, editor cursor — lives in host-persisted `State` structs (the
//! ratatui `StatefulWidget` idiom). Adding a component means implementing
//! `View` for its render, and, if interactive, a small state struct with an
//! event handler. No trait-object tree to reconcile.

use ratatui_core::layout::Rect;

use super::geometry::Size;
use super::style::{StyleSheet, Theme};
use super::surface::Surface;

/// Context threaded to every `View::render`.
pub struct RenderCtx<'a> {
    /// The active theme supplying colors for this frame.
    pub theme: &'a Theme,
    /// The active stylesheet mapping semantic roles to styles. Defaults to
    /// [`StyleSheet::from_theme`] (owned by value, so a plain
    /// [`RenderCtx::new`] needs no separate sheet); a host centralizes styling
    /// by installing its own with [`with_sheet`](Self::with_sheet) or
    /// [`paint_with_sheet`](crate::host::paint_with_sheet).
    pub sheet: StyleSheet,
    /// Whether the focused component of the frame is this one. Containers pass
    /// this down unchanged; focus-aware leaves use it to highlight borders.
    pub focused: bool,
}

impl<'a> RenderCtx<'a> {
    /// A root context for `theme`, unfocused, with the theme's default stylesheet.
    pub fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            sheet: StyleSheet::from_theme(theme),
            focused: false,
        }
    }

    /// Replace the stylesheet, keeping the theme and focus.
    pub fn with_sheet(mut self, sheet: StyleSheet) -> Self {
        self.sheet = sheet;
        self
    }

    /// A child context with an explicit focus flag.
    pub fn with_focus(&self, focused: bool) -> RenderCtx<'a> {
        RenderCtx {
            theme: self.theme,
            sheet: self.sheet,
            focused,
        }
    }
}

/// A drawable, measurable UI element.
///
/// `measure` reports the intrinsic content size the view would like given
/// `available`; the flex solver uses it for `Dimension::Auto` sizing.
/// `render` paints into the `area` the layout assigned, through the clipped
/// `surface`. A view may borrow application state; use it as the root of a
/// [`ScopedScene`](crate::ScopedScene) when it needs Tuika-owned overlays.
pub trait View {
    /// Report the intrinsic content size wanted given `available`.
    fn measure(&self, available: Size) -> Size;
    /// Paint the view into the assigned `area` through `surface`.
    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx);
}

/// Boxed `'static` view, the element type stored by owned containers.
///
/// For a frame root that borrows host state, keep the concrete view on the
/// stack and compose it with [`ScopedScene`](crate::ScopedScene) instead of
/// cloning the state into an `Element`.
pub type Element = Box<dyn View>;

impl View for Element {
    fn measure(&self, available: Size) -> Size {
        (**self).measure(available)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        (**self).render(area, surface, ctx)
    }
}

/// Convenience to box any view into an [`Element`].
pub fn element<V: View + 'static>(view: V) -> Element {
    Box::new(view)
}

/// A closure-backed view for custom terminal-cell drawing.
///
/// The callback receives the assigned area, an already-clipped [`Surface`],
/// and the current [`RenderCtx`]. It is the small escape hatch for charts,
/// terminal grids, and incremental migrations that do not warrant a named
/// component.
///
/// ![custom drawing demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/primitives.gif)
pub struct DrawView<F> {
    intrinsic: Size,
    draw: F,
}

impl<F> DrawView<F> {
    /// Create a draw view that measures to all available space.
    pub fn new(draw: F) -> Self {
        Self {
            intrinsic: Size::new(u16::MAX, u16::MAX),
            draw,
        }
    }

    /// Set the intrinsic size reported by [`View::measure`].
    pub fn intrinsic_size(mut self, size: Size) -> Self {
        self.intrinsic = size;
        self
    }
}

impl<F> View for DrawView<F>
where
    F: Fn(Rect, &mut Surface<'_>, &RenderCtx<'_>),
{
    fn measure(&self, available: Size) -> Size {
        Size::new(
            self.intrinsic.width.min(available.width),
            self.intrinsic.height.min(available.height),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        (self.draw)(area, surface, ctx);
    }
}

/// Alias for [`DrawView`] emphasizing free-form canvas-style drawing.
pub type CanvasView<F> = DrawView<F>;

#[cfg(test)]
mod draw_view_tests {
    use super::*;
    use crate::Theme;
    use crate::testing::{grid, render};

    #[test]
    fn draw_view_is_clipped_and_reports_intrinsic_size() {
        let view = DrawView::new(
            |area: Rect, surface: &mut Surface<'_>, ctx: &RenderCtx<'_>| {
                surface.set_string(area.x, area.y, "canvas", ctx.theme.text_style());
                surface.set(area.right(), area.bottom(), 'x', ctx.theme.text_style());
            },
        )
        .intrinsic_size(Size::new(20, 3));

        assert_eq!(view.measure(Size::new(4, 1)), Size::new(4, 1));
        assert_eq!(grid(&render(&view, 4, 1, &Theme::default())), "canv");
    }
}
