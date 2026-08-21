//! Stateful dialog presets assembled from [`Dialog`](super::Dialog) and the
//! ordinary input components.

use crate::geometry::Rect;
use crate::text::Line;

use crate::components::text::wrap_str;
use crate::event::{Event, InputOutcome, KeyCode};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View, element};
use crate::width::str_cols;

use super::{
    Dialog, Flex, MultiSelectState, SelectList, SelectNavigation, SelectState, TabSelect,
    TabSelectState, TextInput, TextInputState,
};

fn escape(event: &Event) -> bool {
    matches!(event, Event::Key(key) if key.plain() && key.code == KeyCode::Esc)
}

/// Host-owned safe-default state for a [`ConfirmDialog`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfirmDialogState {
    selection: TabSelectState,
}

impl ConfirmDialogState {
    /// A state with the safe/cancel option selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the affirmative option is currently selected.
    pub fn confirmed(&self) -> bool {
        self.selection.selected() == 1
    }

    /// Handle Left/Right/Tab selection, Enter confirmation, and Esc
    /// cancellation.
    pub fn handle(&mut self, event: &Event) -> InputOutcome {
        if escape(event) {
            InputOutcome::Cancelled
        } else {
            self.selection.handle(event, 2)
        }
    }
}

/// Confirmation dialog with a message and safe-default two-button selector.
///
/// The dialog preset demo cycles through confirm, single-choice,
/// multiple-choice, and text-input variants.
///
/// ![dialog presets demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/dialog_presets.gif)
///
/// Run it with `cargo run --example demo -- dialog_presets`.
///
/// ```
/// use tuika::prelude::*;
///
/// let state = ConfirmDialogState::new();
/// let dialog = ConfirmDialog::new("Apply changes?", "Update three files?", &state)
///     .confirm_label("Apply")
///     .into_dialog();
/// let scene = Scene::new(element(Text::raw("workspace"))).dialog(dialog);
/// assert_eq!(scene.overlay_count(), 1);
/// ```
pub struct ConfirmDialog {
    title: String,
    message: String,
    cancel_label: String,
    confirm_label: String,
    selected: usize,
    width: u16,
    height: u16,
    dim: bool,
    owner: Option<String>,
}

impl ConfirmDialog {
    /// Create a confirmation dialog whose initial highlighted action mirrors
    /// `state` (`Cancel` by default).
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        state: &ConfirmDialogState,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            cancel_label: "Cancel".into(),
            confirm_label: "Confirm".into(),
            selected: state.selection.selected(),
            width: 52,
            height: 7,
            dim: true,
            owner: None,
        }
    }

    /// Replace the safe action label.
    pub fn cancel_label(mut self, label: impl Into<String>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Replace the affirmative action label.
    pub fn confirm_label(mut self, label: impl Into<String>) -> Self {
        self.confirm_label = label.into();
        self
    }

    /// Set absolute outer dialog size.
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Whether to dim the rendered scene behind the dialog (default true).
    pub fn dim_backdrop(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Claim exclusive input ownership while this dialog is topmost.
    pub fn focus_owner(mut self, id: impl Into<String>) -> Self {
        self.owner = Some(id.into());
        self
    }

    /// Convert this preset into the general [`Dialog`] builder.
    pub fn into_dialog(self) -> Dialog {
        let state = TabSelectState::with_selected(self.selected);
        let actions = TabSelect::new(
            vec![
                Line::from(self.cancel_label),
                Line::from(self.confirm_label),
            ],
            &state,
        );
        configured_dialog(
            Dialog::new(self.title, element(DialogCopy::new(self.message)))
                .actions(element(actions))
                .size(self.width, self.height),
            self.dim,
            self.owner,
        )
    }
}

impl From<ConfirmDialog> for Dialog {
    fn from(value: ConfirmDialog) -> Self {
        value.into_dialog()
    }
}

/// Host-owned cursor for a [`ChoiceDialog`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ChoiceDialogState {
    selection: SelectState,
}

impl ChoiceDialogState {
    /// A state with the first option selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently highlighted option.
    pub fn selected(&self) -> Option<usize> {
        self.selection.selected()
    }

    /// Handle list navigation, Enter submission, and Esc cancellation.
    pub fn handle(&mut self, event: &Event, option_count: usize) -> InputOutcome {
        self.selection
            .handle_with(event, option_count, SelectNavigation::common())
    }
}

/// Single-choice dialog with optional explanatory copy above the list.
///
/// ![dialog presets demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/dialog_presets.gif)
///
/// Run it with `cargo run --example demo -- dialog_presets`.
///
/// ```
/// use tuika::prelude::*;
///
/// let state = ChoiceDialogState::new();
/// let dialog = ChoiceDialog::new(
///     "Choose model",
///     "Select one model.",
///     vec!["Fast".into(), "Deep".into()],
///     &state,
/// );
/// let scene = Scene::new(element(Text::raw("workspace"))).dialog(dialog.into_dialog());
/// assert_eq!(scene.overlay_count(), 1);
/// ```
pub struct ChoiceDialog {
    title: String,
    message: String,
    options: Vec<String>,
    selected: Option<usize>,
    viewport: u16,
    width: u16,
    height: u16,
    dim: bool,
    owner: Option<String>,
}

impl ChoiceDialog {
    /// Create a choice dialog from owned option labels.
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        options: Vec<String>,
        state: &ChoiceDialogState,
    ) -> Self {
        let viewport = 8u16;
        let height = (options.len().min(usize::from(viewport)) as u16).saturating_add(6);
        Self {
            title: title.into(),
            message: message.into(),
            options,
            selected: state.selection.selected(),
            viewport,
            width: 56,
            height,
            dim: true,
            owner: None,
        }
    }

    /// Cap visible option rows.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = rows.max(1);
        self
    }

    /// Set absolute outer dialog size.
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Whether to dim the rendered scene behind the dialog (default true).
    pub fn dim_backdrop(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Claim exclusive input ownership while this dialog is topmost.
    pub fn focus_owner(mut self, id: impl Into<String>) -> Self {
        self.owner = Some(id.into());
        self
    }

    /// Convert this preset into the general [`Dialog`] builder.
    pub fn into_dialog(self) -> Dialog {
        let mut state = SelectState::unselected();
        state.select(self.selected);
        let rows = self.options.into_iter().map(Line::from).collect();
        let content = Flex::column()
            .auto(element(DialogCopy::new(self.message)))
            .grow(
                1,
                element(SelectList::new(rows, &state).viewport(self.viewport)),
            );
        configured_dialog(
            Dialog::new(self.title, element(content))
                .size(self.width, self.height)
                .key_hints([("enter", "choose"), ("esc", "cancel")]),
            self.dim,
            self.owner,
        )
    }
}

impl From<ChoiceDialog> for Dialog {
    fn from(value: ChoiceDialog) -> Self {
        value.into_dialog()
    }
}

/// Host-owned cursor and checked set for a [`MultiChoiceDialog`].
#[derive(Clone, Debug, Default)]
pub struct MultiChoiceDialogState {
    selection: MultiSelectState,
}

impl MultiChoiceDialogState {
    /// A state with the first row highlighted and nothing checked.
    pub fn new() -> Self {
        Self::default()
    }

    /// Highlighted row.
    pub fn cursor(&self) -> Option<usize> {
        self.selection.cursor().selected()
    }

    /// Whether `index` is checked.
    pub fn contains(&self, index: usize) -> bool {
        self.selection.contains(index)
    }

    /// Checked indices in ascending order.
    pub fn selected(&self) -> impl Iterator<Item = usize> + '_ {
        self.selection.selected()
    }

    /// Navigate with common aliases, toggle with Space, submit with Enter, and
    /// cancel with Esc.
    pub fn handle(&mut self, event: &Event, option_count: usize) -> InputOutcome {
        if let Event::Key(key) = event
            && key.plain()
            && key.code == KeyCode::Enter
        {
            return InputOutcome::Submitted;
        }
        self.selection
            .handle(event, option_count, SelectNavigation::common())
    }
}

/// Multiple-choice dialog with checkbox rows.
///
/// ![dialog presets demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/dialog_presets.gif)
///
/// Run it with `cargo run --example demo -- dialog_presets`.
///
/// ```
/// use tuika::prelude::*;
///
/// let state = MultiChoiceDialogState::new();
/// let dialog = MultiChoiceDialog::new(
///     "Include context",
///     "Select any sources.",
///     vec!["Diagnostics".into(), "Git diff".into()],
///     &state,
/// );
/// let scene = Scene::new(element(Text::raw("workspace"))).dialog(dialog.into_dialog());
/// assert_eq!(scene.overlay_count(), 1);
/// ```
pub struct MultiChoiceDialog {
    title: String,
    message: String,
    options: Vec<String>,
    cursor: Option<usize>,
    checked: Vec<usize>,
    viewport: u16,
    width: u16,
    height: u16,
    dim: bool,
    owner: Option<String>,
}

impl MultiChoiceDialog {
    /// Create a multiple-choice dialog from owned option labels.
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        options: Vec<String>,
        state: &MultiChoiceDialogState,
    ) -> Self {
        let viewport = 8u16;
        let height = (options.len().min(usize::from(viewport)) as u16).saturating_add(6);
        Self {
            title: title.into(),
            message: message.into(),
            options,
            cursor: state.cursor(),
            checked: state.selected().collect(),
            viewport,
            width: 56,
            height,
            dim: true,
            owner: None,
        }
    }

    /// Cap visible option rows.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = rows.max(1);
        self
    }

    /// Set absolute outer dialog size.
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Whether to dim the rendered scene behind the dialog (default true).
    pub fn dim_backdrop(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Claim exclusive input ownership while this dialog is topmost.
    pub fn focus_owner(mut self, id: impl Into<String>) -> Self {
        self.owner = Some(id.into());
        self
    }

    /// Convert this preset into the general [`Dialog`] builder.
    pub fn into_dialog(self) -> Dialog {
        let mut cursor = SelectState::unselected();
        cursor.select(self.cursor);
        let rows = self
            .options
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                let marker = if self.checked.contains(&index) {
                    "[×]"
                } else {
                    "[ ]"
                };
                Line::from(format!("{marker} {label}"))
            })
            .collect();
        let content = Flex::column()
            .auto(element(DialogCopy::new(self.message)))
            .grow(
                1,
                element(SelectList::new(rows, &cursor).viewport(self.viewport)),
            );
        configured_dialog(
            Dialog::new(self.title, element(content))
                .size(self.width, self.height)
                .key_hints([("space", "toggle"), ("enter", "submit"), ("esc", "cancel")]),
            self.dim,
            self.owner,
        )
    }
}

impl From<MultiChoiceDialog> for Dialog {
    fn from(value: MultiChoiceDialog) -> Self {
        value.into_dialog()
    }
}

/// Host-owned editor state for an [`InputDialog`].
#[derive(Clone, Debug, Default)]
pub struct InputDialogState {
    input: TextInputState,
}

impl InputDialogState {
    /// An empty input state that submits on Enter and inserts a newline on
    /// Shift+Enter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the editor with `text`, cursor at the end.
    pub fn from_text(text: &str) -> Self {
        Self {
            input: TextInputState::from_text(text),
        }
    }

    /// Borrow the underlying editor state for cursor placement or advanced
    /// editing configuration.
    pub fn input(&self) -> &TextInputState {
        &self.input
    }

    /// Mutably borrow the underlying editor state.
    pub fn input_mut(&mut self) -> &mut TextInputState {
        &mut self.input
    }

    /// Current text.
    pub fn text(&self) -> String {
        self.input.text()
    }

    /// Handle editing, Enter submission, and Esc cancellation.
    pub fn handle(&mut self, event: &Event) -> InputOutcome {
        if escape(event) {
            InputOutcome::Cancelled
        } else {
            self.input.handle(event)
        }
    }
}

/// Text-entry dialog with a bounded multiline editor.
///
/// ![dialog presets demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/dialog_presets.gif)
///
/// Run it with `cargo run --example demo -- dialog_presets`.
///
/// ```
/// use tuika::prelude::*;
///
/// let state = InputDialogState::from_text("Review parser changes");
/// let dialog = InputDialog::new("Rename task", "Enter a concise name.", &state);
/// let scene = Scene::new(element(Text::raw("workspace"))).dialog(dialog.into_dialog());
/// assert_eq!(scene.overlay_count(), 1);
/// ```
pub struct InputDialog {
    title: String,
    message: String,
    input: TextInputState,
    placeholder: String,
    input_rows: u16,
    width: u16,
    height: u16,
    dim: bool,
    owner: Option<String>,
}

impl InputDialog {
    /// Create a text-entry dialog snapshot from `state`.
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        state: &InputDialogState,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            input: state.input.clone(),
            placeholder: String::new(),
            input_rows: 3,
            width: 56,
            height: 10,
            dim: true,
            owner: None,
        }
    }

    /// Set placeholder text shown while the editor is empty.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set the editor viewport height.
    pub fn input_rows(mut self, rows: u16) -> Self {
        self.input_rows = rows.max(1);
        self
    }

    /// Set absolute outer dialog size.
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Whether to dim the rendered scene behind the dialog (default true).
    pub fn dim_backdrop(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Claim exclusive input ownership while this dialog is topmost.
    pub fn focus_owner(mut self, id: impl Into<String>) -> Self {
        self.owner = Some(id.into());
        self
    }

    /// Convert this preset into the general [`Dialog`] builder.
    pub fn into_dialog(self) -> Dialog {
        let input = ThemedInput {
            state: self.input,
            placeholder: self.placeholder,
        };
        let content = Flex::column()
            .auto(element(DialogCopy::new(self.message)))
            .fixed(self.input_rows, element(input));
        configured_dialog(
            Dialog::new(self.title, element(content))
                .size(self.width, self.height)
                .key_hints([
                    ("enter", "submit"),
                    ("shift+enter", "newline"),
                    ("esc", "cancel"),
                ]),
            self.dim,
            self.owner,
        )
    }
}

impl From<InputDialog> for Dialog {
    fn from(value: InputDialog) -> Self {
        value.into_dialog()
    }
}

fn configured_dialog(mut dialog: Dialog, dim: bool, owner: Option<String>) -> Dialog {
    dialog = dialog.dim_backdrop(dim);
    if let Some(owner) = owner {
        dialog = dialog.focus_owner(owner);
    }
    dialog
}

struct DialogCopy {
    text: String,
}

impl DialogCopy {
    fn new(text: String) -> Self {
        Self { text }
    }

    fn lines(&self, width: u16) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        self.text
            .split('\n')
            // `wrap_str` already yields one empty row for a blank line, so a
            // deliberate blank between paragraphs survives.
            .flat_map(|line| wrap_str(line, width).into_iter().map(|row| row.text))
            .collect()
    }
}

impl View for DialogCopy {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let lines = self.lines(available.width);
        Size::new(
            lines.iter().map(|line| str_cols(line)).max().unwrap_or(0),
            (lines.len().min(usize::from(available.height))) as u16,
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        for (row, line) in self.lines(area.width).into_iter().enumerate() {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            surface.set_string(area.x, y, &line, ctx.theme.text_style());
        }
    }
}

struct ThemedInput {
    state: TextInputState,
    placeholder: String,
}

impl ThemedInput {
    fn view(&self, ctx: &RenderCtx) -> TextInput {
        TextInput::new(&self.state)
            .style(ctx.theme.text_style())
            .placeholder(self.placeholder.clone(), ctx.theme.muted_style())
    }
}

impl View for ThemedInput {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.view(ctx).measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.view(ctx).render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Key;
    use crate::scene::Scene;
    use crate::testing::{grid, render};
    use crate::{Theme, components::Text};

    fn key(code: KeyCode) -> Event {
        Event::Key(Key::new(code))
    }

    #[test]
    fn confirm_defaults_safe_and_submits_selected_action() {
        let mut state = ConfirmDialogState::new();
        assert!(!state.confirmed());
        assert_eq!(state.handle(&key(KeyCode::Right)), InputOutcome::Changed);
        assert!(state.confirmed());
        assert_eq!(state.handle(&key(KeyCode::Enter)), InputOutcome::Submitted);
        assert_eq!(state.handle(&key(KeyCode::Esc)), InputOutcome::Cancelled);
    }

    #[test]
    fn choice_and_multi_choice_keep_values_in_host_state() {
        let mut choice = ChoiceDialogState::new();
        assert_eq!(choice.handle(&key(KeyCode::Down), 3), InputOutcome::Changed);
        assert_eq!(choice.selected(), Some(1));
        assert_eq!(
            choice.handle(&key(KeyCode::Enter), 3),
            InputOutcome::Submitted
        );

        let mut multi = MultiChoiceDialogState::new();
        assert_eq!(
            multi.handle(&key(KeyCode::Char(' ')), 3),
            InputOutcome::Changed
        );
        assert!(multi.contains(0));
        assert_eq!(
            multi.handle(&key(KeyCode::Enter), 3),
            InputOutcome::Submitted
        );
        assert_eq!(multi.selected().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn input_dialog_delegates_editing_and_keeps_text_on_submit() {
        let mut state = InputDialogState::new();
        assert_eq!(
            state.handle(&key(KeyCode::Char('h'))),
            InputOutcome::Changed
        );
        assert_eq!(state.handle(&key(KeyCode::Enter)), InputOutcome::Submitted);
        assert_eq!(state.text(), "h");
        assert_eq!(state.handle(&key(KeyCode::Esc)), InputOutcome::Cancelled);
    }

    #[test]
    fn every_preset_renders_through_scene_dialog() {
        let theme = Theme::default();
        let root = || element(Text::raw("base"));

        let confirm = Scene::new(root()).dialog(
            ConfirmDialog::new(
                "Delete",
                "Delete the selected file?",
                &ConfirmDialogState::new(),
            )
            .into_dialog(),
        );
        let rendered = grid(&render(&confirm, 64, 12, &theme));
        assert!(rendered.contains("Delete the selected file?"));
        assert!(rendered.contains("Cancel"));
        assert!(rendered.contains("Confirm"));

        let choice = Scene::new(root()).dialog(
            ChoiceDialog::new(
                "Model",
                "Choose a model",
                vec!["Fast".into(), "Deep".into()],
                &ChoiceDialogState::new(),
            )
            .into_dialog(),
        );
        assert!(grid(&render(&choice, 64, 14, &theme)).contains("Choose a model"));

        let mut multi_state = MultiChoiceDialogState::new();
        let _ = multi_state.handle(&key(KeyCode::Char(' ')), 2);
        let multi = Scene::new(root()).dialog(
            MultiChoiceDialog::new(
                "Tools",
                "Allow tools",
                vec!["Shell".into(), "Network".into()],
                &multi_state,
            )
            .into_dialog(),
        );
        assert!(grid(&render(&multi, 64, 14, &theme)).contains("[×] Shell"));

        let input = Scene::new(root()).dialog(
            InputDialog::new("Reason", "Explain why", &InputDialogState::new())
                .placeholder("Type a reason")
                .into_dialog(),
        );
        assert!(grid(&render(&input, 64, 14, &theme)).contains("Type a reason"));
    }

    #[test]
    fn presets_survive_degenerate_screens() {
        for width in 0..5 {
            for height in 0..5 {
                let scene = Scene::new(element(Text::raw("base"))).dialog(
                    ConfirmDialog::new("Confirm", "Continue?", &ConfirmDialogState::new())
                        .into_dialog(),
                );
                let _ = render(&scene, width, height, &Theme::default());
            }
        }
    }
}
