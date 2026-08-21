//! Owned and borrowed base-tree composition with owned overlays.
//!
//! [`Scene`] owns both its base [`Element`] and every [`SceneOverlay`]. Use it
//! when the complete frame can be moved into Tuika's owned view tree.
//! [`ScopedScene`] instead borrows a base [`View`] for one frame while retaining
//! the same owned overlay stack. It is intended for application roots that read
//! large host-owned state—such as transcripts, logs, or tables—directly during
//! painting. The borrowed root does not need to clone that state or become a
//! boxed `'static` [`Element`].
//!
//! Overlay views remain owned [`Element`]s, so dialogs and scene focus behavior
//! stay independent of the root borrow. The base tree may contain borrowed
//! views at any depth: [`element`](crate::element) preserves the borrow as a
//! [`ScopedElement`](crate::ScopedElement), and composition containers accept
//! that scoped element without requiring a custom wrapper view.
//!
//! # Borrowed application data with an owned dialog
//!
//! ```
//! use tuika::ui::Rect;
//! use tuika::prelude::*;
//!
//! struct Dashboard<'data> {
//!     messages: &'data [String],
//! }
//!
//! impl View for Dashboard<'_> {
//!     fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
//!         Size::new(available.width, (self.messages.len() as u16).min(available.height))
//!     }
//!
//!     fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
//!         for (row, message) in self.messages.iter().take(area.height as usize).enumerate() {
//!             surface.set_string(
//!                 area.x,
//!                 area.y + row as u16,
//!                 message,
//!                 ctx.theme.text_style(),
//!             );
//!         }
//!     }
//! }
//!
//! let messages = vec!["building".to_owned(), "ready".to_owned()];
//! let dashboard = Dashboard { messages: &messages };
//! let scene = ScopedScene::new(&dashboard).dialog(
//!     Dialog::new("Confirm", element(Text::raw("Delete?"))).size(20, 5),
//! );
//!
//! let rendered = tuika::testing::render(&scene, 30, 8, &Theme::default());
//! assert!(tuika::testing::grid(&rendered).contains("Delete?"));
//! ```
//!
//! # Rebuilding each frame without cloning
//!
//! The scene cannot outlive the root it borrows. End the frame's scope before
//! mutating the host state, then build the next frame from a fresh borrow:
//!
//! ```
//! use tuika::ui::Rect;
//! use tuika::prelude::*;
//!
//! struct Count<'data>(&'data [String]);
//! impl View for Count<'_> {
//!     fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
//!         Size::new(available.width.min(8), available.height.min(1))
//!     }
//!     fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
//!         surface.set_string(
//!             area.x,
//!             area.y,
//!             &self.0.len().to_string(),
//!             ctx.theme.text_style(),
//!         );
//!     }
//! }
//!
//! let mut messages = vec!["first".to_owned()];
//! {
//!     let root = Count(&messages);
//!     let scene = ScopedScene::new(&root);
//!     let frame = tuika::testing::render(&scene, 8, 1, &Theme::default());
//!     assert_eq!(&tuika::testing::grid(&frame)[..1], "1");
//! }
//! messages.push("second".to_owned());
//! {
//!     let root = Count(&messages);
//!     let scene = ScopedScene::new(&root);
//!     let frame = tuika::testing::render(&scene, 8, 1, &Theme::default());
//!     assert_eq!(&tuika::testing::grid(&frame)[..1], "2");
//! }
//! ```

use crate::geometry::Rect;
use crate::style::{Modifier, Style};

use crate::focus::FocusRegistry;
use crate::geometry::Size;
use crate::overlay::{OverlaySpec, TargetPlacement};
use crate::probe::RectProbe;
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, View};

/// How an overlay treats the already-rendered layers behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    /// Leave the layers behind the overlay unchanged.
    #[default]
    None,
    /// Add the terminal's dim modifier to the layers behind the overlay.
    Dim,
}

/// An owned overlay whose placement is resolved against the scene at render time.
pub struct SceneOverlay {
    view: Element,
    spec: OverlaySpec,
    clear: bool,
    backdrop: Backdrop,
    focus_owner: Option<String>,
    target: Option<(RectProbe, TargetPlacement)>,
}

impl SceneOverlay {
    /// Create an overlay from an owned view and placement specification.
    pub fn new(view: Element, spec: OverlaySpec) -> Self {
        Self {
            view,
            spec,
            clear: true,
            backdrop: Backdrop::None,
            focus_owner: None,
            target: None,
        }
    }

    /// Clear the resolved overlay rectangle before painting (default true).
    pub fn clear(mut self, clear: bool) -> Self {
        self.clear = clear;
        self
    }

    /// Configure the backdrop rendered behind this layer.
    pub fn backdrop(mut self, backdrop: Backdrop) -> Self {
        self.backdrop = backdrop;
        self
    }

    /// Associate an input-owner id with this overlay.
    pub fn focus_owner(mut self, id: impl Into<String>) -> Self {
        self.focus_owner = Some(id.into());
        self
    }

    /// Place this overlay relative to the rect reported by `target`.
    ///
    /// A target in the scene root is available in the same render because the
    /// root is painted before overlays. Rebuild the scene without this overlay
    /// whenever the target view is absent; an unpainted probe reports
    /// [`Rect::ZERO`].
    pub fn target(mut self, target: &RectProbe, placement: TargetPlacement) -> Self {
        self.target = Some((target.clone(), placement));
        self
    }

    /// Resolve this overlay's current rectangle.
    pub fn resolve(&self, area: Rect) -> Rect {
        if let Some((target, placement)) = &self.target {
            self.spec.resolve_target(area, target.rect(), *placement)
        } else {
            self.spec.resolve(area)
        }
    }

    /// The input owner associated with this overlay, if any.
    pub fn owner(&self) -> Option<&str> {
        self.focus_owner.as_deref()
    }
}

/// An owned root view plus ordered overlays.
///
/// A scene is itself a [`View`], so a host can pass it directly to
/// [`paint`](crate::paint). Overlays are rendered in insertion order and only
/// the topmost one receives a focused render context.
///
/// Use [`ScopedScene`] when the root view borrows host-owned data that should
/// not be cloned into this scene's owned [`Element`].
///
/// ![owned primitives demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/primitives.gif)
pub struct Scene {
    root: Element,
    overlays: Vec<SceneOverlay>,
}

impl Scene {
    /// Create a scene with no overlays.
    pub fn new(root: Element) -> Self {
        Self {
            root,
            overlays: Vec::new(),
        }
    }

    /// Add an owned overlay.
    pub fn overlay(mut self, overlay: SceneOverlay) -> Self {
        self.overlays.push(overlay);
        self
    }

    /// Add an owned view with a placement specification.
    pub fn layer(self, view: Element, spec: OverlaySpec) -> Self {
        self.overlay(SceneOverlay::new(view, spec))
    }

    /// Add a dialog composition.
    pub fn dialog(self, dialog: crate::components::Dialog) -> Self {
        self.overlay(dialog.into_overlay())
    }

    /// The number of overlay layers.
    pub fn overlay_count(&self) -> usize {
        self.overlays.len()
    }

    /// Synchronize exclusive input ownership with the topmost overlay.
    ///
    /// Call this after constructing the frame's scene. A scene without an
    /// owner-bearing overlay releases overlay ownership.
    pub fn sync_focus(&self, focus: &mut FocusRegistry) {
        sync_focus(&self.overlays, focus);
    }
}

impl View for Scene {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.root.measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        render_scene(self.root.as_ref(), &self.overlays, area, surface, ctx);
    }
}

/// A frame-scoped borrowed root view plus owned, ordered overlays.
///
/// `'view` is the lifetime of the reference to `root`; the root's own type may
/// contain longer-lived references to host state. The scene must be painted
/// before that root reference expires. `V` remains concrete, so constructing a
/// scoped scene does not allocate or require a `dyn View` coercion.
///
/// Rendering and focus semantics are identical to [`Scene`]: the root renders
/// first and unfocused when overlays exist, overlays render in insertion order,
/// and only the topmost overlay receives a focused [`RenderCtx`].
///
/// See the [module-level examples](self) for borrowed data across successive
/// frames.
pub struct ScopedScene<'view, V: View + ?Sized> {
    root: &'view V,
    overlays: Vec<SceneOverlay>,
}

impl<'view, V: View + ?Sized> ScopedScene<'view, V> {
    /// Borrow `root` for this scene's lifetime and start with no overlays.
    pub fn new(root: &'view V) -> Self {
        Self {
            root,
            overlays: Vec::new(),
        }
    }

    /// Add an owned overlay.
    pub fn overlay(mut self, overlay: SceneOverlay) -> Self {
        self.overlays.push(overlay);
        self
    }

    /// Add an owned view with a placement specification.
    pub fn layer(self, view: Element, spec: OverlaySpec) -> Self {
        self.overlay(SceneOverlay::new(view, spec))
    }

    /// Add an owned dialog composition over the borrowed root.
    pub fn dialog(self, dialog: crate::components::Dialog) -> Self {
        self.overlay(dialog.into_overlay())
    }

    /// The number of overlay layers.
    pub fn overlay_count(&self) -> usize {
        self.overlays.len()
    }

    /// Synchronize exclusive input ownership with the topmost overlay.
    ///
    /// Call this after constructing the frame's scene. An ownerless top layer
    /// releases overlay ownership, even if a lower layer has an owner.
    ///
    /// ```
    /// use tuika::prelude::*;
    ///
    /// let root = Text::raw("application");
    /// let scene = ScopedScene::new(&root).dialog(
    ///     Dialog::new("Confirm", element(Text::raw("Continue?")))
    ///         .focus_owner("confirm-dialog"),
    /// );
    /// let mut focus = FocusRegistry::new();
    /// focus.register("application");
    /// scene.sync_focus(&mut focus);
    ///
    /// assert_eq!(focus.active(), Some("confirm-dialog"));
    /// ```
    pub fn sync_focus(&self, focus: &mut FocusRegistry) {
        sync_focus(&self.overlays, focus);
    }
}

impl<V: View + ?Sized> View for ScopedScene<'_, V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.root.measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        render_scene(self.root, &self.overlays, area, surface, ctx);
    }
}

fn sync_focus(overlays: &[SceneOverlay], focus: &mut FocusRegistry) {
    if let Some(owner) = overlays.last().and_then(SceneOverlay::owner) {
        focus.set_owner(owner);
    } else {
        focus.clear_owner();
    }
}

fn render_scene<V: View + ?Sized>(
    root: &V,
    overlays: &[SceneOverlay],
    area: Rect,
    surface: &mut Surface,
    ctx: &RenderCtx,
) {
    root.render(area, &mut surface.child(area), &ctx.with_focus(false));

    let top = overlays.len().saturating_sub(1);
    for (index, overlay) in overlays.iter().enumerate() {
        if overlay.backdrop == Backdrop::Dim {
            let dim_area = area.intersection(surface.area());
            let buffer = surface.buffer_mut();
            for y in dim_area.y..dim_area.bottom() {
                for x in dim_area.x..dim_area.right() {
                    buffer[(x, y)].set_style(Style::default().add_modifier(Modifier::DIM));
                }
            }
        }
        let overlay_area = overlay.resolve(area);
        let mut layer = surface.child(overlay_area);
        if overlay.clear {
            layer.fill(Style::default().bg(ctx.theme.surface));
        }
        overlay
            .view
            .render(overlay_area, &mut layer, &ctx.with_focus(index == top));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::components::Text;
    use crate::overlay::{Extent, TargetAlign, TargetPlacement};
    use crate::probe::RectProbe;
    use crate::testing::{grid, render};
    use crate::{Theme, element};

    struct FocusProbe(Rc<RefCell<Vec<bool>>>);

    impl View for FocusProbe {
        fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
            available
        }

        fn render(&self, _area: Rect, _surface: &mut Surface, ctx: &RenderCtx) {
            self.0.borrow_mut().push(ctx.focused);
        }
    }

    #[test]
    fn scene_resolves_layers_and_topmost_alone_is_focused() {
        let focus = Rc::new(RefCell::new(Vec::new()));
        let scene = Scene::new(element(Text::raw("base")))
            .overlay(SceneOverlay::new(
                element(FocusProbe(focus.clone())),
                OverlaySpec::centered(100, 100),
            ))
            .overlay(SceneOverlay::new(
                element(FocusProbe(focus.clone())),
                OverlaySpec::centered(50, 50),
            ));
        let _ = render(&scene, 8, 4, &Theme::default());
        assert_eq!(&*focus.borrow(), &[false, true]);
    }

    #[test]
    fn scoped_scene_matches_owned_composition() {
        let owned = Scene::new(element(Text::raw("base")))
            .overlay(
                SceneOverlay::new(element(Text::raw("lower")), OverlaySpec::centered(100, 100))
                    .clear(false)
                    .backdrop(Backdrop::Dim),
            )
            .layer(element(Text::raw("top")), OverlaySpec::centered(50, 100));
        let borrowed_root = Text::raw("base");
        let scoped = ScopedScene::new(&borrowed_root)
            .overlay(
                SceneOverlay::new(element(Text::raw("lower")), OverlaySpec::centered(100, 100))
                    .clear(false)
                    .backdrop(Backdrop::Dim),
            )
            .layer(element(Text::raw("top")), OverlaySpec::centered(50, 100));

        assert_eq!(
            render(&owned, 8, 2, &Theme::default()),
            render(&scoped, 8, 2, &Theme::default())
        );
        assert_eq!(scoped.overlay_count(), 2);
    }

    #[test]
    fn scoped_scene_orders_overlays_and_focuses_only_the_topmost() {
        let focus = Rc::new(RefCell::new(Vec::new()));
        let root = FocusProbe(focus.clone());
        let scene = ScopedScene::new(&root)
            .overlay(SceneOverlay::new(
                element(FocusProbe(focus.clone())),
                OverlaySpec::centered(100, 100),
            ))
            .overlay(SceneOverlay::new(
                element(FocusProbe(focus.clone())),
                OverlaySpec::centered(50, 50),
            ));

        let _ = render(&scene, 8, 4, &Theme::default());
        assert_eq!(&*focus.borrow(), &[false, false, true]);
    }

    #[test]
    fn scene_resolves_target_overlay_after_the_root_is_laid_out() {
        let target = RectProbe::new();
        let root = crate::components::Flex::column()
            .fixed(2, element(Text::raw("header")))
            .fixed(1, target.wrap(Text::raw("trigger")))
            .grow(1, element(Text::raw("body")));
        let spec = OverlaySpec {
            width: Extent::Cells(10),
            height: Extent::Cells(2),
            ..OverlaySpec::centered(0, 0)
        };
        let scene = ScopedScene::new(&root).overlay(
            SceneOverlay::new(element(Text::raw("popover")), spec)
                .target(&target, TargetPlacement::below().align(TargetAlign::Start)),
        );

        let rendered = render(&scene, 20, 8, &Theme::default());

        assert_eq!(target.rect(), Rect::new(0, 2, 20, 1));
        assert!(
            grid(&rendered)
                .lines()
                .nth(3)
                .is_some_and(|line| line.starts_with("popover"))
        );
    }

    #[test]
    fn scene_handles_zero_size_and_overlay_order() {
        let scene = Scene::new(element(Text::raw("base")))
            .layer(element(Text::raw("one")), OverlaySpec::centered(100, 100))
            .layer(element(Text::raw("two")), OverlaySpec::centered(100, 100));
        assert_eq!(grid(&render(&scene, 4, 1, &Theme::default())), "two ");
        let _ = render(&scene, 0, 0, &Theme::default());
    }

    #[test]
    fn scoped_scene_handles_zero_size() {
        let root = Text::raw("borrowed");
        let scene = ScopedScene::new(&root).layer(
            element(Text::raw("overlay")),
            OverlaySpec::centered(100, 100),
        );
        let _ = render(&scene, 0, 0, &Theme::default());
    }

    #[test]
    fn scene_synchronizes_topmost_input_owner() {
        let scene = Scene::new(element(Text::raw("base"))).overlay(
            SceneOverlay::new(element(Text::raw("dialog")), OverlaySpec::centered(50, 50))
                .focus_owner("dialog"),
        );
        let mut focus = FocusRegistry::new();
        focus.register("base");
        scene.sync_focus(&mut focus);
        assert_eq!(focus.active(), Some("dialog"));
    }

    #[test]
    fn ownerless_top_layer_releases_lower_layer_owner() {
        let scene = Scene::new(element(Text::raw("base")))
            .overlay(
                SceneOverlay::new(element(Text::raw("dialog")), OverlaySpec::centered(50, 50))
                    .focus_owner("lower"),
            )
            .layer(element(Text::raw("top")), OverlaySpec::centered(20, 20));
        let mut focus = FocusRegistry::new();
        focus.register("base");
        scene.sync_focus(&mut focus);
        assert_eq!(focus.active(), Some("base"));
    }

    #[test]
    fn scoped_scene_synchronizes_owner_and_releases_it_for_ownerless_top_layer() {
        let root = Text::raw("borrowed");
        let owned_top = ScopedScene::new(&root).overlay(
            SceneOverlay::new(element(Text::raw("dialog")), OverlaySpec::centered(50, 50))
                .focus_owner("dialog"),
        );
        let mut focus = FocusRegistry::new();
        focus.register("base");
        owned_top.sync_focus(&mut focus);
        assert_eq!(focus.active(), Some("dialog"));

        let ownerless_top =
            owned_top.layer(element(Text::raw("tooltip")), OverlaySpec::centered(20, 20));
        ownerless_top.sync_focus(&mut focus);
        assert_eq!(focus.active(), Some("base"));
    }

    #[test]
    fn dim_backdrop_respects_the_parent_surface_clip() {
        use crate::buffer::{Buffer, Cell};

        let mut buffer = Buffer::filled(Rect::new(0, 0, 6, 1), Cell::new("#"));
        let scene = Scene::new(element(Text::raw("base"))).overlay(
            SceneOverlay::new(element(Text::raw("modal")), OverlaySpec::centered(100, 100))
                .clear(false)
                .backdrop(Backdrop::Dim),
        );
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let mut surface = Surface::new(&mut buffer, Rect::new(2, 0, 2, 1));
        scene.render(Rect::new(0, 0, 6, 1), &mut surface, &ctx);

        assert!(!buffer[(1, 0)].modifier.contains(Modifier::DIM));
        assert!(buffer[(2, 0)].modifier.contains(Modifier::DIM));
        assert!(buffer[(3, 0)].modifier.contains(Modifier::DIM));
        assert!(!buffer[(4, 0)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn scoped_dim_backdrop_respects_the_parent_surface_clip() {
        use crate::buffer::{Buffer, Cell};

        let mut buffer = Buffer::filled(Rect::new(0, 0, 6, 1), Cell::new("#"));
        let root = Text::raw("base");
        let scene = ScopedScene::new(&root).overlay(
            SceneOverlay::new(element(Text::raw("modal")), OverlaySpec::centered(100, 100))
                .clear(false)
                .backdrop(Backdrop::Dim),
        );
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let mut surface = Surface::new(&mut buffer, Rect::new(2, 0, 2, 1));
        scene.render(Rect::new(0, 0, 6, 1), &mut surface, &ctx);

        assert!(!buffer[(1, 0)].modifier.contains(Modifier::DIM));
        assert!(buffer[(2, 0)].modifier.contains(Modifier::DIM));
        assert!(buffer[(3, 0)].modifier.contains(Modifier::DIM));
        assert!(!buffer[(4, 0)].modifier.contains(Modifier::DIM));
    }
}
