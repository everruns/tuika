//! Responsive lifecycle and geometry for a single auxiliary panel.
//!
//! [`DockState`] keeps the host-owned visibility/focus state; [`DockSpec`]
//! resolves that state to a wide dock, a narrow focused drawer, or a hidden
//! passive panel. The panel's content, focus id, input routing, and chrome stay
//! with the host.

use crate::geometry::Rect;

/// Screen edge that owns a dock or drawer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DockEdge {
    /// Place the panel at the left edge.
    Left,
    /// Place the panel at the right edge.
    #[default]
    Right,
}

/// Responsive geometry policy for one dockable panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockSpec {
    edge: DockEdge,
    breakpoint: u16,
    panel_width: u16,
}

impl DockSpec {
    /// Put a panel on the right, docking at `breakpoint` columns and using at
    /// most `panel_width` columns.
    pub const fn right(breakpoint: u16, panel_width: u16) -> Self {
        Self {
            edge: DockEdge::Right,
            breakpoint,
            panel_width,
        }
    }

    /// Put a panel on the left, docking at `breakpoint` columns and using at
    /// most `panel_width` columns.
    pub const fn left(breakpoint: u16, panel_width: u16) -> Self {
        Self {
            edge: DockEdge::Left,
            breakpoint,
            panel_width,
        }
    }

    /// The screen edge that owns the panel.
    pub const fn edge(self) -> DockEdge {
        self.edge
    }

    /// Width at which a visible panel becomes docked instead of a drawer.
    pub const fn breakpoint(self) -> u16 {
        self.breakpoint
    }

    /// Preferred panel width. Resolution clamps it to at least one column and
    /// at most the available frame width.
    pub const fn panel_width(self) -> u16 {
        self.panel_width
    }
}

/// How the panel is placed in the current frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockPlacement {
    /// The panel is not painted. This includes a passive panel below its dock
    /// breakpoint; its state remains visible so it can reappear after resize.
    Hidden,
    /// The panel consumes space beside the main content.
    Docked,
    /// The focused panel overlays the main content without shrinking it.
    Drawer,
}

/// Resolved rectangles for a responsive dock frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockLayout {
    /// Area left for the host's main content.
    pub main: Rect,
    /// Panel area when docked or drawn; `None` when hidden.
    pub panel: Option<Rect>,
    /// Placement selected for this frame.
    pub placement: DockPlacement,
}

/// Host-owned visibility and focus lifecycle for one dockable panel.
///
/// This is intentionally not a panel manager: it does not own views, focus
/// identifiers, key bindings, or application state. A host can pair it with a
/// [`FocusRegistry`](crate::focus::FocusRegistry) and any panel content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DockState {
    visible: bool,
    focused: bool,
}

impl DockState {
    /// A hidden, unfocused panel.
    pub const fn new() -> Self {
        Self {
            visible: false,
            focused: false,
        }
    }

    /// Whether the panel is logically open. A passive panel may still resolve
    /// to [`DockPlacement::Hidden`] below its breakpoint.
    pub const fn is_visible(self) -> bool {
        self.visible
    }

    /// Whether the panel should own input focus.
    pub const fn is_focused(self) -> bool {
        self.focused
    }

    /// Open the panel without taking focus from the main content.
    pub fn show_passive(&mut self) {
        self.visible = true;
        self.focused = false;
    }

    /// Open and focus the panel.
    pub fn focus(&mut self) {
        self.visible = true;
        self.focused = true;
    }

    /// Keep the panel open but return focus to the main content.
    pub fn blur(&mut self) {
        self.focused = false;
    }

    /// Close the panel and release its focus.
    pub fn hide(&mut self) {
        self.visible = false;
        self.focused = false;
    }

    /// Focus a hidden or passive panel; close a focused panel.
    ///
    /// This supports the common single-shortcut lifecycle: background work can
    /// open passively, the first shortcut focuses it, and the next closes it.
    pub fn toggle(&mut self) {
        if self.visible && self.focused {
            self.hide();
        } else {
            self.focus();
        }
    }

    /// Resolve the panel for `area` under `spec`.
    ///
    /// Wide visible panels dock regardless of focus. Below the breakpoint, a
    /// passive panel stays logically open but is not painted; focusing it turns
    /// it into an overlay drawer, so input can never be captured by an invisible
    /// panel.
    pub fn resolve(self, area: Rect, spec: DockSpec) -> DockLayout {
        if !self.visible || area.width == 0 || area.height == 0 {
            return hidden(area);
        }
        let panel_width = spec.panel_width.clamp(1, area.width);
        if area.width >= spec.breakpoint {
            let (main, panel) = split(area, panel_width, spec.edge);
            return DockLayout {
                main,
                panel: Some(panel),
                placement: DockPlacement::Docked,
            };
        }
        if !self.focused {
            return hidden(area);
        }
        let panel = match spec.edge {
            DockEdge::Left => Rect {
                width: panel_width,
                ..area
            },
            DockEdge::Right => Rect {
                x: area.right().saturating_sub(panel_width),
                width: panel_width,
                ..area
            },
        };
        DockLayout {
            main: area,
            panel: Some(panel),
            placement: DockPlacement::Drawer,
        }
    }
}

fn hidden(area: Rect) -> DockLayout {
    DockLayout {
        main: area,
        panel: None,
        placement: DockPlacement::Hidden,
    }
}

fn split(area: Rect, panel_width: u16, edge: DockEdge) -> (Rect, Rect) {
    let main_width = area.width.saturating_sub(panel_width);
    match edge {
        DockEdge::Left => (
            Rect {
                x: area.x.saturating_add(panel_width),
                width: main_width,
                ..area
            },
            Rect {
                width: panel_width,
                ..area
            },
        ),
        DockEdge::Right => (
            Rect {
                width: main_width,
                ..area
            },
            Rect {
                x: area.x.saturating_add(main_width),
                width: panel_width,
                ..area
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    #[test]
    fn passive_dock_focuses_then_closes() {
        let mut state = DockState::default();
        state.show_passive();
        assert!(state.is_visible());
        assert!(!state.is_focused());

        state.toggle();
        assert!(state.is_visible());
        assert!(state.is_focused());

        state.toggle();
        assert!(!state.is_visible());
        assert!(!state.is_focused());
    }

    #[test]
    fn wide_docks_narrow_passive_hides_and_narrow_focus_draws() {
        let spec = DockSpec::right(90, 40);
        let wide = Rect::new(0, 0, 120, 24);
        let narrow = Rect::new(0, 0, 70, 24);
        let mut state = DockState::default();
        state.show_passive();

        let docked = state.resolve(wide, spec);
        assert_eq!(docked.placement, DockPlacement::Docked);
        assert_eq!(docked.main, Rect::new(0, 0, 80, 24));
        assert_eq!(docked.panel, Some(Rect::new(80, 0, 40, 24)));

        let hidden = state.resolve(narrow, spec);
        assert_eq!(hidden.placement, DockPlacement::Hidden);
        assert_eq!(hidden.main, narrow);
        assert_eq!(hidden.panel, None);

        state.focus();
        let drawer = state.resolve(narrow, spec);
        assert_eq!(drawer.placement, DockPlacement::Drawer);
        assert_eq!(drawer.main, narrow);
        assert_eq!(drawer.panel, Some(Rect::new(30, 0, 40, 24)));
    }

    #[test]
    fn left_drawer_clamps_to_tiny_viewports() {
        let spec = DockSpec::left(90, 40);
        let mut state = DockState::default();
        state.focus();

        let area = Rect::new(3, 5, 18, 8);
        let layout = state.resolve(area, spec);
        assert_eq!(layout.placement, DockPlacement::Drawer);
        assert_eq!(layout.main, area);
        assert_eq!(layout.panel, Some(area));
    }

    #[test]
    fn focused_panel_with_zero_preferred_width_stays_visible() {
        let mut state = DockState::default();
        state.focus();

        let area = Rect::new(0, 0, 20, 8);
        let layout = state.resolve(area, DockSpec::right(90, 0));
        assert_eq!(layout.placement, DockPlacement::Drawer);
        assert_eq!(layout.panel, Some(Rect::new(19, 0, 1, 8)));
    }
}
