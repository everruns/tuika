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
use super::style::{StyleBundle, StyleResolver, StyleRole, StyleSheet, Theme};

/// The space available to a view on one axis during intrinsic measurement.
///
/// Most callers have a concrete cell extent and use [`Definite`](Self::Definite).
/// Layout containers with intrinsic track sizing can instead ask for the
/// smallest unwrapped contribution or the preferred unconstrained extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AvailableSpace {
    /// A concrete number of terminal cells.
    Definite(u16),
    /// The smallest extent the content can occupy without avoidable overflow.
    MinContent,
    /// The preferred extent when the axis is unconstrained.
    MaxContent,
}

impl AvailableSpace {
    const fn fallback(self) -> u16 {
        match self {
            Self::Definite(cells) => cells,
            Self::MinContent => 0,
            Self::MaxContent => u16::MAX,
        }
    }
}

/// A layout engine's complete measurement request for a view.
///
/// `known_*` means the layout algorithm has already resolved that axis. The
/// default [`View::measure_request`] implementation adapts this richer request
/// to the original [`View::measure`] method, so existing third-party views keep
/// working. Width-sensitive built-ins can override it when min/max-content or
/// one-known-axis measurement materially changes their intrinsic contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeasureRequest {
    /// Width already fixed by the parent layout pass.
    pub known_width: Option<u16>,
    /// Height already fixed by the parent layout pass.
    pub known_height: Option<u16>,
    /// Width constraint when the width is not already known.
    pub available_width: AvailableSpace,
    /// Height constraint when the height is not already known.
    pub available_height: AvailableSpace,
}

impl MeasureRequest {
    /// Measure within a concrete cell rectangle, with neither axis pre-resolved.
    pub const fn new(available: Size) -> Self {
        Self {
            known_width: None,
            known_height: None,
            available_width: AvailableSpace::Definite(available.width),
            available_height: AvailableSpace::Definite(available.height),
        }
    }

    /// Supply a width the parent has already resolved.
    pub const fn with_known_width(mut self, width: u16) -> Self {
        self.known_width = Some(width);
        self
    }

    /// Supply a height the parent has already resolved.
    pub const fn with_known_height(mut self, height: u16) -> Self {
        self.known_height = Some(height);
        self
    }

    /// Change the width sizing mode used when width is not known.
    pub const fn with_available_width(mut self, available: AvailableSpace) -> Self {
        self.available_width = available;
        self
    }

    /// Change the height sizing mode used when height is not known.
    pub const fn with_available_height(mut self, available: AvailableSpace) -> Self {
        self.available_height = available;
        self
    }

    /// The concrete fallback passed to legacy [`View::measure`] implementations.
    pub fn fallback_available(self) -> Size {
        Size::new(
            match self.known_width {
                Some(width) => width,
                None => self.available_width.fallback(),
            },
            match self.known_height {
                Some(height) => height,
                None => self.available_height.fallback(),
            },
        )
    }

    pub(crate) fn resolve(self, measured: Size) -> Size {
        Size::new(
            resolve_axis(self.known_width, self.available_width, measured.width),
            resolve_axis(self.known_height, self.available_height, measured.height),
        )
    }
}

fn resolve_axis(known: Option<u16>, available: AvailableSpace, measured: u16) -> u16 {
    known.unwrap_or(match available {
        AvailableSpace::Definite(limit) => measured.min(limit),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => measured,
    })
}
use super::surface::Surface;

/// Context threaded to every [`View::measure`] and [`View::render`] call.
pub struct RenderCtx<'a> {
    /// The active theme supplying colors for this frame.
    pub theme: &'a Theme,
    /// The active stylesheet mapping semantic roles to styles. Defaults to
    /// [`StyleSheet::from_theme`] (owned by value, so a plain
    /// [`RenderCtx::new`] needs no separate sheet); a host centralizes styling
    /// by installing its own with [`with_sheet`](Self::with_sheet) or
    /// [`paint_with_sheet`](crate::host::paint_with_sheet).
    pub sheet: StyleSheet,
    /// Optional host policy for built-in and application-defined semantic
    /// styles. Resolver results overlay the active stylesheet.
    style_resolver: Option<&'a dyn StyleResolver>,
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
            style_resolver: None,
            focused: false,
        }
    }

    /// Replace the stylesheet, keeping the theme and focus.
    pub fn with_sheet(mut self, sheet: StyleSheet) -> Self {
        self.sheet = sheet;
        self
    }

    /// Install a host resolver for built-in and application-defined roles.
    pub fn with_style_resolver(mut self, resolver: &'a dyn StyleResolver) -> Self {
        self.style_resolver = Some(resolver);
        self
    }

    /// Resolve a semantic style from the stylesheet and optional host policy.
    ///
    /// Unknown roles start empty, so companion crates and applications can
    /// define their own [`StyleRole`] constants without extending tuika.
    pub fn style(&self, role: StyleRole) -> StyleBundle {
        let base = self.sheet.resolve_style(role).unwrap_or_default();
        self.style_resolver
            .and_then(|resolver| resolver.resolve(role))
            .map_or(base, |override_bundle| base.overlay(override_bundle))
    }

    pub(crate) fn style_resolver_key(&self) -> Option<(usize, u64)> {
        self.style_resolver.map(|resolver| {
            let identity = std::ptr::from_ref(resolver).cast::<()>() as usize;
            (identity, resolver.revision())
        })
    }

    /// A child context with an explicit focus flag.
    pub fn with_focus(&self, focused: bool) -> RenderCtx<'a> {
        RenderCtx {
            theme: self.theme,
            sheet: self.sheet,
            style_resolver: self.style_resolver,
            focused,
        }
    }
}

/// A drawable, measurable UI element.
///
/// `measure` reports the intrinsic content size the view would like given
/// `available` and the same active theme, stylesheet, and focus state that its
/// render will receive; the flex solver uses it for `Dimension::Auto` sizing.
/// `render` paints into the `area` the layout assigned, through the clipped
/// `surface`. A view may borrow application state; use it as the root of a
/// [`ScopedScene`](crate::ScopedScene) when it needs Tuika-owned overlays.
pub trait View {
    /// Report the intrinsic content size wanted given `available` and `ctx`.
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size;
    /// Answer a measurement request with optional known axes and intrinsic
    /// sizing modes.
    ///
    /// The compatibility implementation delegates to [`measure`](Self::measure).
    /// Override this only when the distinction between definite,
    /// minimum-content, and maximum-content measurement affects the result.
    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        request.resolve(self.measure(request.fallback_available(), ctx))
    }
    /// Paint the view into the assigned `area` through `surface`.
    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx);
}

/// A boxed view that may borrow data for `'view`.
///
/// Composition containers are generic over their child view type and default
/// to owned [`Element`]s. Use `ScopedElement<'_>` when a heterogeneous subtree
/// contains frame-borrowed views.
pub type ScopedElement<'view> = Box<dyn View + 'view>;

/// Boxed owned view used by retained values and cross-thread host seams.
pub type Element = ScopedElement<'static>;

impl View for Box<dyn View + '_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        (**self).measure(available, ctx)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        (**self).measure_request(request, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        (**self).render(area, surface, ctx)
    }
}

/// Box a view while preserving any data it borrows.
///
/// A `'static` view naturally produces an owned [`Element`]. A borrowed view
/// produces a [`ScopedElement`] whose lifetime cannot escape the current frame.
pub fn element<'view, V: View + 'view>(view: V) -> ScopedElement<'view> {
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
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
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
    use crate::style::{StyleBundle, StyleResolver, StyleRole};
    use crate::testing::{grid, render, render_with_context};
    use ratatui_core::style::Color;

    const METRIC: StyleRole = StyleRole::new("example.metric");

    struct AppStyles;

    impl StyleResolver for AppStyles {
        fn resolve(&self, role: StyleRole) -> Option<StyleBundle> {
            match role {
                METRIC => Some(StyleBundle::new().fg(Color::Green).bold()),
                StyleRole::KEY_HINT_KEY => Some(StyleBundle::new().fg(Color::Yellow)),
                _ => None,
            }
        }
    }

    #[test]
    fn draw_view_is_clipped_and_reports_intrinsic_size() {
        let view = DrawView::new(
            |area: Rect, surface: &mut Surface<'_>, ctx: &RenderCtx<'_>| {
                surface.set_string(area.x, area.y, "canvas", ctx.theme.text_style());
                surface.set(area.right(), area.bottom(), 'x', ctx.theme.text_style());
            },
        )
        .intrinsic_size(Size::new(20, 3));

        let theme = Theme::default();
        assert_eq!(
            view.measure(Size::new(4, 1), &RenderCtx::new(&theme)),
            Size::new(4, 1)
        );
        assert_eq!(grid(&render(&view, 4, 1, &Theme::default())), "canv");
    }

    #[test]
    fn context_resolves_application_roles_and_overlays_built_in_defaults() {
        let view = DrawView::new(
            |area: Rect, surface: &mut Surface<'_>, ctx: &RenderCtx<'_>| {
                surface.set(area.x, area.y, 'm', ctx.style(METRIC).to_style());
                surface.set(
                    area.x + 1,
                    area.y,
                    'k',
                    ctx.style(StyleRole::KEY_HINT_KEY).to_style(),
                );
            },
        );
        let theme = Theme::default();
        let styles = AppStyles;
        let ctx = RenderCtx::new(&theme).with_style_resolver(&styles);
        let buffer = render_with_context(&view, 2, 1, &ctx);

        assert_eq!(buffer[(0, 0)].fg, Color::Green);
        assert!(
            buffer[(0, 0)]
                .modifier
                .contains(ratatui_core::style::Modifier::BOLD)
        );
        assert_eq!(buffer[(1, 0)].fg, Color::Yellow);
        assert_eq!(
            buffer[(1, 0)].bg,
            theme.accent,
            "unset resolver bg inherits"
        );
    }
}
