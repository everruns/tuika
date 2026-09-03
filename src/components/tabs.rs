//! Focus-independent tab navigation with host-owned selection state.

use crate::geometry::Rect;
use crate::style::{Modifier, Style};
use crate::text::Line;

use crate::{Event, InputOutcome, KeyCode, RenderCtx, Size, Surface, View};

use super::text::line_width;

#[derive(Clone, Debug, Default)]
/// Host-owned selected-tab state.
pub struct TabsState {
    selected: usize,
}

impl TabsState {
    /// A state with the first tab selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// A state with `index` selected.
    pub fn with_selected(index: usize) -> Self {
        Self { selected: index }
    }

    /// Current selected tab index.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Select an index, clamped to the available tab count.
    pub fn select(&mut self, index: usize, len: usize) {
        self.selected = index.min(len.saturating_sub(1));
    }

    /// Handle left/right and forward/backward tab navigation.
    pub fn handle(&mut self, event: &Event, len: usize) -> InputOutcome {
        if len == 0 {
            return InputOutcome::Ignored;
        }
        let Event::Key(key) = event else {
            return InputOutcome::Ignored;
        };
        if !key.plain() {
            return InputOutcome::Ignored;
        }
        let before = self.selected;
        match key.code {
            KeyCode::Left | KeyCode::BackTab => {
                self.selected = self.selected.min(len - 1);
                self.selected = self.selected.checked_sub(1).unwrap_or(len - 1);
            }
            KeyCode::Right | KeyCode::Tab => {
                self.selected = self.selected.min(len - 1);
                self.selected = (self.selected + 1) % len;
            }
            _ => return InputOutcome::Ignored,
        }
        if self.selected == before {
            InputOutcome::Consumed
        } else {
            InputOutcome::Changed
        }
    }
}

/// A one-line tab strip derived from [`TabsState`].
///
/// ![tabs demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/tabs.gif)
pub struct Tabs {
    labels: Vec<Line<'static>>,
    selected: usize,
}

impl Tabs {
    /// Create a tab strip, snapshotting the selected index for this frame.
    pub fn new(labels: Vec<Line<'static>>, state: &TabsState) -> Self {
        Self {
            labels,
            selected: state.selected(),
        }
    }
}

impl View for Tabs {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let labels = self
            .labels
            .iter()
            .map(line_width)
            .fold(0, u16::saturating_add);
        let gaps = (self.labels.len().saturating_sub(1) as u16).saturating_mul(2);
        Size::new(
            labels.saturating_add(gaps).min(available.width),
            u16::from(available.height > 0),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let mut x = area.x;
        for (index, label) in self.labels.iter().enumerate() {
            if index > 0 {
                x = surface.set_string(x, area.y, "  ", ctx.theme.muted_style());
            }
            let selected = index == self.selected;
            for span in &label.spans {
                let style = if selected {
                    span.style.patch(
                        ctx.theme
                            .accent_style()
                            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    )
                } else {
                    span.style.patch(Style::default().fg(ctx.theme.muted))
                };
                x = surface.set_string(x, area.y, span.content.as_ref(), style);
            }
            if x >= area.right() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, InputOutcome, Key, KeyCode};
    use crate::style::Theme;
    use crate::tests::support::row;
    use crate::text::Line;

    #[test]
    fn tabs_state_wraps_and_tabs_render_selection() {
        let mut state = TabsState::default();
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Left)), 3),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), 2);

        let tabs = Tabs::new(
            vec![Line::from("one"), Line::from("two"), Line::from("three")],
            &state,
        );
        let theme = Theme::default();
        let rendered = crate::testing::render(&tabs, 20, 1, &theme);
        assert!(row(&rendered, 0).contains("one  two  three"));
        assert!(
            rendered[(10, 0)]
                .modifier
                .contains(crate::style::Modifier::UNDERLINED)
        );
    }
}
