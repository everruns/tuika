//! Modal dialog composition from existing Tuika primitives.

use crate::geometry::Padding;
use crate::overlay::{Extent, OverlaySpec};
use crate::scene::{Backdrop, SceneOverlay};
use crate::style::BorderStyle;
use crate::view::{Element, element};

use super::{Boxed, Flex, KeyHints};

/// Builder for an owned modal dialog layer.
///
/// The content and optional actions are ordinary [`Element`]s, so forms,
/// markdown, selectors, and custom views compose without dialog-specific
/// adapters. Persistent control state remains host-owned.
///
/// ![dialog demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/primitives.gif)
pub struct Dialog {
    title: String,
    content: Element,
    actions: Option<Element>,
    spec: OverlaySpec,
    border: BorderStyle,
    padding: Padding,
    clear: bool,
    dim: bool,
    owner: Option<String>,
}

impl Dialog {
    /// Create a centered dialog with a raised-surface background.
    pub fn new(title: impl Into<String>, content: Element) -> Self {
        Self {
            title: title.into(),
            content,
            actions: None,
            spec: OverlaySpec::centered(60, 60).min_size(24, 5).margin(1),
            border: BorderStyle::Rounded,
            padding: Padding::symmetric(1, 0),
            clear: true,
            dim: false,
            owner: None,
        }
    }

    /// Set an arbitrary actions/footer view.
    pub fn actions(mut self, actions: Element) -> Self {
        self.actions = Some(actions);
        self
    }

    /// Set the footer to compact `(key, action)` hints.
    pub fn key_hints<K, L>(self, hints: impl IntoIterator<Item = (K, L)>) -> Self
    where
        K: Into<String>,
        L: Into<String>,
    {
        self.actions(element(KeyHints::new(hints)))
    }

    /// Replace the complete overlay placement specification.
    pub fn placement(mut self, spec: OverlaySpec) -> Self {
        self.spec = spec;
        self
    }

    /// Set absolute dialog width and height.
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.spec.width = Extent::Cells(width);
        self.spec.height = Extent::Cells(height);
        self
    }

    /// Set minimum resolved dialog size.
    pub fn min_size(mut self, width: u16, height: u16) -> Self {
        self.spec = self.spec.min_size(width, height);
        self
    }

    /// Set maximum resolved dialog size.
    pub fn max_size(mut self, width: u16, height: u16) -> Self {
        self.spec = self.spec.max_size(width, height);
        self
    }

    /// Set the dialog border glyph style.
    pub fn border(mut self, border: BorderStyle) -> Self {
        self.border = border;
        self
    }

    /// Set padding between dialog chrome and content.
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Dim all already-rendered layers behind the dialog.
    pub fn dim_backdrop(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Clear the overlay rectangle before painting (default true).
    pub fn clear(mut self, clear: bool) -> Self {
        self.clear = clear;
        self
    }

    /// Claim exclusive input ownership for this id while the dialog is topmost.
    pub fn focus_owner(mut self, id: impl Into<String>) -> Self {
        self.owner = Some(id.into());
        self
    }

    /// Finish the composition as an owned scene overlay.
    pub fn into_overlay(self) -> SceneOverlay {
        let mut body = Flex::column().grow(1, self.content);
        if let Some(actions) = self.actions {
            body = body.fixed(1, actions);
        }
        let panel = Boxed::new(element(body))
            .title(format!(" {} ", self.title))
            .border(self.border)
            .padding(self.padding);
        let mut overlay = SceneOverlay::new(element(panel), self.spec)
            .clear(self.clear)
            .backdrop(if self.dim {
                Backdrop::Dim
            } else {
                Backdrop::None
            });
        if let Some(owner) = self.owner {
            overlay = overlay.focus_owner(owner);
        }
        overlay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::scene::Scene;
    use crate::testing::{grid, render};
    use crate::{Theme, element};

    #[test]
    fn dialog_composes_title_content_and_hints() {
        let scene = Scene::new(element(Text::raw("base"))).dialog(
            Dialog::new("Confirm", element(Text::raw("Delete?")))
                .size(18, 5)
                .key_hints([("enter", "yes"), ("esc", "no")]),
        );
        let rendered = grid(&render(&scene, 20, 7, &Theme::default()));
        assert!(rendered.contains(" Confirm "));
        assert!(rendered.contains("Delete?"));
        assert!(rendered.contains("enter"));
    }
}
