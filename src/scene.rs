//! Owned base-tree and overlay composition.

use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};

use crate::focus::FocusRegistry;
use crate::geometry::Size;
use crate::overlay::OverlaySpec;
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

    /// Resolve this overlay's current rectangle.
    pub fn resolve(&self, area: Rect) -> Rect {
        self.spec.resolve(area)
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
        if let Some(owner) = self.overlays.last().and_then(SceneOverlay::owner) {
            focus.set_owner(owner);
        } else {
            focus.clear_owner();
        }
    }
}

impl View for Scene {
    fn measure(&self, available: Size) -> Size {
        self.root.measure(available)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.root
            .render(area, &mut surface.child(area), &ctx.with_focus(false));

        let top = self.overlays.len().saturating_sub(1);
        for (index, overlay) in self.overlays.iter().enumerate() {
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
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::components::Text;
    use crate::testing::{grid, render};
    use crate::{Theme, element};

    struct FocusProbe(Rc<RefCell<Vec<bool>>>);

    impl View for FocusProbe {
        fn measure(&self, available: Size) -> Size {
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
    fn scene_handles_zero_size_and_overlay_order() {
        let scene = Scene::new(element(Text::raw("base")))
            .layer(element(Text::raw("one")), OverlaySpec::centered(100, 100))
            .layer(element(Text::raw("two")), OverlaySpec::centered(100, 100));
        assert_eq!(grid(&render(&scene, 4, 1, &Theme::default())), "two ");
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
    fn dim_backdrop_respects_the_parent_surface_clip() {
        use ratatui_core::buffer::{Buffer, Cell};

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
}
