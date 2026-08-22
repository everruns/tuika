//! Compact, responsive key/action hints and complete keymap help.

use crate::geometry::Rect;
use crate::keymap::{Hint, Keymap};
use crate::style::StyleRole;
use crate::width::str_cols;
use crate::{RenderCtx, Size, Surface, View};

/// A key/action hint with a responsive fitting priority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyHint {
    /// Display-ready key sequence.
    pub key: String,
    /// Human-readable action label.
    pub label: String,
    /// Higher values are retained first when horizontal space is tight.
    pub priority: i32,
}

impl KeyHint {
    /// Create a hint at the default priority (`0`).
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            priority: 0,
        }
    }

    /// Assign a fitting priority.
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    fn width(&self) -> usize {
        usize::from(str_cols(&self.key)) + usize::from(str_cols(&self.label)) + 3
    }
}

/// A one-line sequence of styled key and action labels.
///
/// ![responsive key hints demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/key_hints.png)
///
/// A narrow area never paints half a hint. The highest-priority complete hints
/// that fit are retained, while their original declaration order is preserved.
pub struct KeyHints {
    hints: Vec<KeyHint>,
}

impl KeyHints {
    /// Build hints from `(key, action)` pairs at equal priority.
    pub fn new<K, L>(hints: impl IntoIterator<Item = (K, L)>) -> Self
    where
        K: Into<String>,
        L: Into<String>,
    {
        Self::prioritized(
            hints
                .into_iter()
                .map(|(key, label)| KeyHint::new(key, label)),
        )
    }

    /// Build hints with explicit priorities.
    pub fn prioritized(hints: impl IntoIterator<Item = KeyHint>) -> Self {
        Self {
            hints: hints.into_iter().collect(),
        }
    }

    /// Build footer hints from active, labeled keymap bindings.
    ///
    /// Layer priority doubles as fitting priority, so modal or contextual
    /// actions survive before global actions on narrow screens.
    pub fn from_keymap<C: Clone>(keymap: &Keymap<C>) -> Self {
        Self::prioritized(
            keymap.hints().into_iter().filter_map(|hint| {
                Some(KeyHint::new(hint.keys, hint.label?).priority(hint.priority))
            }),
        )
    }

    fn fitted(&self, width: usize) -> Vec<usize> {
        let mut candidates: Vec<usize> = (0..self.hints.len()).collect();
        candidates.sort_by(|&a, &b| {
            self.hints[b]
                .priority
                .cmp(&self.hints[a].priority)
                .then_with(|| a.cmp(&b))
        });
        let mut used = 0usize;
        let mut selected = Vec::new();
        for index in candidates {
            let extra = self.hints[index].width() + usize::from(!selected.is_empty()) * 2;
            if used.saturating_add(extra) <= width {
                used += extra;
                selected.push(index);
            }
        }
        selected.sort_unstable();
        selected
    }
}

impl View for KeyHints {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let width = self
            .hints
            .iter()
            .map(KeyHint::width)
            .sum::<usize>()
            .saturating_add(self.hints.len().saturating_sub(1) * 2)
            .min(available.width as usize) as u16;
        Size::new(width, u16::from(available.height > 0))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let mut x = area.x;
        for (position, index) in self.fitted(usize::from(area.width)).into_iter().enumerate() {
            let hint = &self.hints[index];
            if position > 0 {
                x = surface.set_string(
                    x,
                    area.y,
                    "  ",
                    ctx.style(StyleRole::KEY_HINT_LABEL).to_style(),
                );
            }
            x = surface.set_string(
                x,
                area.y,
                &format!(" {} ", hint.key),
                ctx.style(StyleRole::KEY_HINT_KEY).to_style(),
            );
            let label_style = ctx.style(StyleRole::KEY_HINT_LABEL).to_style();
            x = surface.set_string(x, area.y, " ", label_style);
            x = surface.set_string(x, area.y, &hint.label, label_style);
        }
    }
}

/// Complete, vertically scrollable help generated from active keymap bindings.
///
/// ![keymap help demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/keymap_help.png)
pub struct KeymapHelp {
    hints: Vec<HelpHint>,
    offset: usize,
}

struct HelpHint {
    keys: String,
    label: String,
}

impl KeymapHelp {
    /// Build help rows from active, labeled bindings in a keymap.
    pub fn from_keymap<C: Clone>(keymap: &Keymap<C>) -> Self {
        Self::from_hints(keymap.hints())
    }

    /// Build help rows from discovered bindings.
    pub fn from_hints<C>(hints: impl IntoIterator<Item = Hint<C>>) -> Self {
        Self {
            hints: hints
                .into_iter()
                .filter_map(|hint| {
                    Some(HelpHint {
                        keys: hint.keys,
                        label: hint.label?,
                    })
                })
                .collect(),
            offset: 0,
        }
    }

    /// Skip the first `offset` help rows.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}

impl View for KeymapHelp {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let key_width = self
            .hints
            .iter()
            .map(|hint| str_cols(&hint.keys))
            .max()
            .unwrap_or(0);
        let width = self
            .hints
            .iter()
            .map(|hint| key_width + 2 + str_cols(&hint.label))
            .max()
            .unwrap_or(0)
            .min(available.width);
        Size::new(
            width,
            self.hints
                .len()
                .saturating_sub(self.offset)
                .min(usize::from(available.height)) as u16,
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let key_width = self
            .hints
            .iter()
            .map(|hint| str_cols(&hint.keys))
            .max()
            .unwrap_or(0);
        for (row, hint) in self
            .hints
            .iter()
            .skip(self.offset)
            .take(usize::from(area.height))
            .enumerate()
        {
            let y = area.y.saturating_add(row as u16);
            surface.set_string(
                area.x,
                y,
                &hint.keys,
                ctx.style(StyleRole::KEY_HINT_KEY).to_style(),
            );
            surface.set_string(
                area.x.saturating_add(key_width).saturating_add(2),
                y,
                &hint.label,
                ctx.style(StyleRole::KEY_HINT_LABEL).to_style(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Layer;
    use crate::style::Color;
    use crate::testing::{grid, render, render_sizes};
    use crate::{Size, Theme};

    #[test]
    fn key_hints_measure_unicode_by_terminal_width() {
        let hints = KeyHints::new([("⌘", "開く")]);
        let theme = Theme::default();
        assert_eq!(
            hints.measure(Size::new(20, 1), &RenderCtx::new(&theme)),
            Size::new(8, 1)
        );
    }

    #[test]
    fn narrow_hints_keep_whole_high_priority_items() {
        let hints = KeyHints::prioritized([
            KeyHint::new("A", "alpha"),
            KeyHint::new("Q", "quit").priority(10),
            KeyHint::new("?", "help").priority(5),
        ]);
        assert_eq!(
            grid(&render(&hints, 18, 1, &Theme::default())),
            " Q  quit   ?  help"
        );
        assert_eq!(grid(&render(&hints, 8, 1, &Theme::default())), " Q  quit");
        assert_eq!(grid(&render(&hints, 3, 1, &Theme::default())), "   ");
    }

    #[test]
    fn key_hints_and_help_survive_degenerate_sizes() {
        let keymap = Keymap::new().layer(
            Layer::new("global")
                .bind_labeled("enter", "open", ())
                .bind_labeled("q", "quit", ()),
        );
        let hints = KeyHints::from_keymap(&keymap);
        let help = KeymapHelp::from_keymap(&keymap);
        let hint_sizes = (0..=24).flat_map(|width| (0..=3).map(move |height| (width, height)));
        let help_sizes = (0..=24).flat_map(|width| (0..=4).map(move |height| (width, height)));
        render_sizes(&hints, hint_sizes, &Theme::default());
        render_sizes(&help, help_sizes, &Theme::default());
    }

    #[test]
    fn footer_and_help_share_semantic_key_styles() {
        let theme = Theme::default();
        let sheet = crate::StyleSheet {
            key_hint_key: crate::style::StyleBundle::new()
                .fg(Color::Yellow)
                .bg(Color::Blue),
            key_hint_label: crate::style::StyleBundle::new().fg(Color::Green),
            ..crate::StyleSheet::from_theme(&theme)
        };
        let footer = KeyHints::new([("q", "quit")]);
        let rendered = crate::testing::render_with_sheet(&footer, 12, 1, &theme, sheet);
        assert_eq!(rendered[(1, 0)].fg, Color::Yellow);
        assert_eq!(rendered[(1, 0)].bg, Color::Blue);
        assert_eq!(rendered[(4, 0)].fg, Color::Green);

        let help = KeymapHelp::from_hints([Hint {
            keys: "Q".into(),
            label: Some("quit".into()),
            command: (),
            layer: "global".into(),
            priority: 0,
        }]);
        let rendered = crate::testing::render_with_sheet(&help, 12, 1, &theme, sheet);
        assert_eq!(rendered[(0, 0)].fg, Color::Yellow);
        assert_eq!(rendered[(3, 0)].fg, Color::Green);
    }

    #[derive(Clone)]
    enum Action {
        Open,
        Quit,
    }

    #[test]
    fn one_keymap_declaration_drives_footer_and_help() {
        let keymap = Keymap::new().layer(
            Layer::new("global")
                .priority(4)
                .bind_labeled("enter", "open", Action::Open)
                .bind_labeled("q", "quit", Action::Quit),
        );
        let footer = KeyHints::from_keymap(&keymap);
        assert_eq!(
            grid(&render(&footer, 30, 1, &Theme::default())),
            " Enter  open   Q  quit        "
        );
        let help = KeymapHelp::from_keymap(&keymap);
        assert_eq!(
            grid(&render(&help, 20, 2, &Theme::default())),
            "Enter  open         \nQ      quit         "
        );
    }
}
