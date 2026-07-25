//! Responsive labeled-form composition.

use ratatui_core::layout::Rect;

use crate::event::{Event, EventFlow, KeyCode};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, View};
use crate::width::str_cols;

/// Result of routing a form-level navigation event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormOutcome {
    /// Focus moved, with the event's consumed status.
    FocusChanged(EventFlow),
    /// The host should submit the form.
    Submitted,
    /// The host should cancel the form.
    Cancelled,
}

/// Host-owned focus state for a form.
#[derive(Clone, Copy, Debug, Default)]
pub struct FormState {
    focused: usize,
}

impl FormState {
    /// Create state focused on the first field.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current focused field index.
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Set focus explicitly; a later [`clamp`](Self::clamp) bounds it.
    pub fn focus(&mut self, index: usize) {
        self.focused = index;
    }

    /// Keep focus within `field_count`.
    pub fn clamp(&mut self, field_count: usize) {
        self.focused = if field_count == 0 {
            0
        } else {
            self.focused.min(field_count - 1)
        };
    }

    /// Handle Tab/BackTab traversal, Enter submission, and Esc cancellation.
    ///
    /// Give the focused control first refusal when Enter or Esc has
    /// control-specific meaning, then pass ignored events here.
    pub fn handle(&mut self, event: &Event, field_count: usize) -> FormOutcome {
        let Event::Key(key) = event else {
            return FormOutcome::FocusChanged(EventFlow::Ignored);
        };
        match key.code {
            KeyCode::Tab if key.plain() && field_count > 0 => {
                self.focused = (self.focused + 1) % field_count;
                FormOutcome::FocusChanged(EventFlow::Consumed)
            }
            KeyCode::BackTab if field_count > 0 => {
                self.focused = (self.focused + field_count - 1) % field_count;
                FormOutcome::FocusChanged(EventFlow::Consumed)
            }
            KeyCode::Enter if key.plain() => FormOutcome::Submitted,
            KeyCode::Esc if key.plain() => FormOutcome::Cancelled,
            _ => FormOutcome::FocusChanged(EventFlow::Ignored),
        }
    }
}

/// One labeled control in a [`Form`].
pub struct FormField {
    label: String,
    control: Element,
    help: Option<String>,
    error: Option<String>,
}

impl FormField {
    /// Create a labeled field around any control view.
    pub fn new(label: impl Into<String>, control: Element) -> Self {
        Self {
            label: label.into(),
            control,
            help: None,
            error: None,
        }
    }

    /// Add de-emphasized help text below the control.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Add a semantic-danger validation row below the control.
    pub fn error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    fn message_rows(&self) -> u16 {
        u16::from(self.help.is_some()) + u16::from(self.error.is_some())
    }
}

/// Responsive composition of labeled control views.
///
/// ![form demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/primitives.gif)
pub struct Form {
    fields: Vec<FormField>,
    focused: usize,
    stack_below: u16,
    gap: u16,
    actions: Option<Element>,
}

impl Form {
    /// Build a form and mirror the host-owned focused index from `state`.
    pub fn new(fields: Vec<FormField>, state: &FormState) -> Self {
        Self {
            fields,
            focused: state.focused(),
            stack_below: 40,
            gap: 1,
            actions: None,
        }
    }

    /// Stack labels above controls below this terminal width.
    pub fn stack_below(mut self, width: u16) -> Self {
        self.stack_below = width;
        self
    }

    /// Set blank rows between fields.
    pub fn gap(mut self, rows: u16) -> Self {
        self.gap = rows;
        self
    }

    /// Add an actions row after all fields.
    pub fn actions(mut self, actions: Element) -> Self {
        self.actions = Some(actions);
        self
    }

    /// Number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the form has no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    fn label_width(&self, available: u16) -> u16 {
        self.fields
            .iter()
            .map(|field| str_cols(&field.label))
            .max()
            .unwrap_or(0)
            .min(available / 2)
    }

    fn stacked(&self, width: u16) -> bool {
        width < self.stack_below
    }
}

impl View for Form {
    fn measure(&self, available: Size) -> Size {
        let stacked = self.stacked(available.width);
        let label_width = self.label_width(available.width);
        let control_width = if stacked {
            available.width
        } else {
            available
                .width
                .saturating_sub(label_width.saturating_add(2))
        };
        let mut height = 0u16;
        for (index, field) in self.fields.iter().enumerate() {
            if index > 0 {
                height = height.saturating_add(self.gap);
            }
            let control = field
                .control
                .measure(Size::new(control_width, available.height));
            height = height
                .saturating_add(control.height.max(1))
                .saturating_add(field.message_rows())
                .saturating_add(u16::from(stacked));
        }
        if let Some(actions) = &self.actions {
            height = height.saturating_add(actions.measure(available).height.max(1));
        }
        Size::new(available.width, height.min(available.height))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let stacked = self.stacked(area.width);
        let label_width = self.label_width(area.width);
        let control_x = if stacked {
            area.x
        } else {
            area.x.saturating_add(label_width).saturating_add(2)
        };
        let control_width = area.right().saturating_sub(control_x);
        let mut y = area.y;

        for (index, field) in self.fields.iter().enumerate() {
            if index > 0 {
                y = y.saturating_add(self.gap);
            }
            if y >= area.bottom() {
                break;
            }
            surface.set_string(area.x, y, &field.label, ctx.theme.text_style());
            if stacked {
                y = y.saturating_add(1);
                if y >= area.bottom() {
                    break;
                }
            }
            let measured = field
                .control
                .measure(Size::new(control_width, area.bottom().saturating_sub(y)));
            let control_height = measured.height.max(1).min(area.bottom().saturating_sub(y));
            let control_area = Rect::new(control_x, y, control_width, control_height);
            field.control.render(
                control_area,
                &mut surface.child(control_area),
                &ctx.with_focus(index == self.focused),
            );
            y = y.saturating_add(control_height);
            if let Some(help) = &field.help
                && y < area.bottom()
            {
                surface.set_string(control_x, y, help, ctx.theme.muted_style());
                y += 1;
            }
            if let Some(error) = &field.error
                && y < area.bottom()
            {
                surface.set_string(control_x, y, error, ctx.theme.danger_style());
                y += 1;
            }
        }
        if let Some(actions) = &self.actions
            && y < area.bottom()
        {
            let actions_area = Rect::new(area.x, y, area.width, area.bottom().saturating_sub(y));
            actions.render(actions_area, &mut surface.child(actions_area), ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Boxed, Text};
    use crate::event::Key;
    use crate::testing::{grid, render};
    use crate::{Theme, element};

    #[test]
    fn form_traverses_submits_and_cancels() {
        let mut state = FormState::new();
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Tab)), 2),
            FormOutcome::FocusChanged(EventFlow::Consumed)
        );
        assert_eq!(state.focused(), 1);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter)), 2),
            FormOutcome::Submitted
        );
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Esc)), 2),
            FormOutcome::Cancelled
        );
    }

    #[test]
    fn form_stacks_on_narrow_areas_and_renders_validation() {
        let state = FormState::new();
        let form = Form::new(
            vec![
                FormField::new("Name", element(Boxed::new(element(Text::raw("Ada")))))
                    .help("shown publicly")
                    .error("already used"),
            ],
            &state,
        );
        let wide = grid(&render(&form, 50, 5, &Theme::default()));
        assert!(wide.lines().next().unwrap().contains("Name"));
        assert!(wide.lines().next().unwrap().contains('╭'));
        let narrow = grid(&render(&form, 12, 6, &Theme::default()));
        assert_eq!(narrow.lines().next().unwrap().trim(), "Name");
        assert!(narrow.contains("already used"));
    }

    #[test]
    fn form_tolerates_zero_sized_render() {
        let state = FormState::new();
        let form = Form::new(Vec::new(), &state);
        let _ = render(&form, 0, 0, &Theme::default());
    }
}
