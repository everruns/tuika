//! What a screen reader hears from each built-in component: the role, label,
//! value, and state it records while painting (see [`crate::access`]).
//!
//! These live together rather than beside each component because each one
//! pins the same cross-module contract — a component's paint feeding the
//! reading pass — and reading them side by side keeps the spoken vocabulary
//! consistent.

use crate::Theme;
use crate::components::Column;
use crate::components::{
    Boxed, Dialog, Form, FormField, ProgressBar, SelectList, SelectState, Spinner, TabSelect,
    TabSelectState, Table, Tabs, TabsState, Text, TextInput, TextInputState, ToastLevel, ToastList,
    Toasts, TreeList, TreeRow, TreeState,
};
use crate::layout::Dimension;
use crate::scene::Scene;
use crate::testing::screen_reader;
use crate::text::Line;
use crate::view::{RenderCtx, View, element};

fn read(view: &dyn View, width: u16, height: u16) -> String {
    screen_reader(view, width, height, &Theme::default())
}

/// Read with the root focused, as a host paints the focused component.
fn read_focused(view: &dyn View, width: u16, height: u16) -> String {
    let theme = Theme::default();
    let frame = crate::access::AccessFrame::new();
    let ctx = RenderCtx::new(&theme).with_access(&frame).with_focus(true);
    let buffer = crate::testing::render_with_context(view, width, height, &ctx);
    let document = frame.document(&buffer);
    let focused = document.focused().map(crate::access::Node::describe);
    format!("{document}\nfocus: {}", focused.unwrap_or_default())
}

fn lines(items: &[&'static str]) -> Vec<Line<'static>> {
    items.iter().map(|item| Line::from(*item)).collect()
}

#[test]
fn select_list_reads_items_with_position_and_selection() {
    let mut state = SelectState::new();
    state.select(Some(1));
    let list = SelectList::new(lines(&["Haiku", "Sonnet", "Opus"]), &state);
    assert_eq!(
        read_focused(&list, 20, 3),
        "list\nHaiku, 1 of 3\nSonnet, 2 of 3, selected\nOpus, 3 of 3\n\
         focus: Sonnet, 2 of 3, selected"
    );
}

#[test]
fn tabs_and_tab_select_read_as_tabs() {
    let tabs = Tabs::new(lines(&["Chat", "Diff"]), &TabsState::with_selected(1));
    assert_eq!(
        read(&tabs, 20, 1),
        "tab list\nChat, tab, 1 of 2\nDiff, tab, 2 of 2, selected"
    );
    let segmented = TabSelect::new(lines(&["Off", "On"]), &TabSelectState::with_selected(0));
    assert_eq!(
        read(&segmented, 20, 1),
        "tab list\nOff, tab, 1 of 2, selected\nOn, tab, 2 of 2"
    );
}

#[test]
fn text_input_reads_its_value_and_placeholder() {
    let empty = TextInputState::new();
    let input = TextInput::new(&empty).placeholder("Ask anything", Default::default());
    assert_eq!(
        read_focused(&input, 20, 1),
        "Ask anything, text input, blank\nfocus: Ask anything, text input, blank"
    );
    let filled = TextInputState::from_text("hello\nworld");
    assert_eq!(
        read(&TextInput::new(&filled), 20, 2),
        "text input, hello world"
    );
}

#[test]
fn progress_bar_reads_its_percentage_or_busy() {
    let bar = ProgressBar::determinate(0.4).label("Build").percent(true);
    assert_eq!(read(&bar, 20, 1), "Build, progress bar, 40%");
    assert_eq!(
        read(&ProgressBar::indeterminate(3), 20, 1),
        "progress bar, busy"
    );
}

#[test]
fn spinners_are_silent() {
    assert_eq!(read(&Spinner::new(3), 4, 1), "");
}

#[test]
fn toasts_read_errors_as_alerts() {
    let mut toasts = Toasts::new(4);
    toasts.push(ToastLevel::Info, "Saved");
    toasts.push(ToastLevel::Error, "Disk full");
    assert_eq!(
        read(&ToastList::new(&toasts), 20, 2),
        "Disk full, alert\nSaved"
    );
}

#[test]
fn tree_list_reads_depth_and_expansion() {
    let rows = vec![
        TreeRow::new("root", None, 0, "src", true),
        TreeRow::new("lib", Some("root"), 1, "lib.rs", false),
    ];
    let mut state = TreeState::new();
    state.expand("root");
    state.select(Some("lib"));
    assert_eq!(
        read(&TreeList::new(&rows, &state), 20, 2),
        "tree\nsrc, level 1, 1 of 2, expanded\nlib.rs, level 2, 2 of 2, selected"
    );
}

#[test]
fn table_reads_rows_as_their_cells() {
    let mut state = SelectState::new();
    state.select(Some(0));
    let table = Table::new(
        vec![
            Column::new("name", Dimension::Fixed(6)),
            Column::new("size", Dimension::Fixed(6)),
        ],
        vec![lines(&["a.rs", "3 KB"]), lines(&["b.rs", "9 KB"])],
        &state,
    );
    assert_eq!(
        read(&table, 20, 3),
        "table\nname size\na.rs, 3 KB, row, 1 of 2, selected\nb.rs, 9 KB, row, 2 of 2"
    );
}

#[test]
fn titled_panels_and_form_fields_read_as_labelled_groups() {
    let panel = Boxed::new(element(Text::raw("body"))).title(" Notes ");
    assert_eq!(read(&panel, 12, 3), "Notes\nbody");

    let name = TextInputState::from_text("Ada");
    let form = Form::new(
        vec![FormField::new("Name", element(TextInput::new(&name))).error("taken")],
        &crate::components::FormState::new(),
    );
    assert_eq!(read(&form, 30, 4), "Name\ntext input, Ada\ntaken");
}

#[test]
fn a_dialog_is_read_alone_with_its_title_once() {
    let dialog = Dialog::new("Apply changes?", element(Text::raw("Three files.")))
        .size(24, 5)
        .dim_backdrop(true);
    let scene = Scene::new(element(Text::raw("behind"))).overlay(dialog.into_overlay());
    assert_eq!(read(&scene, 30, 8), "Apply changes?, dialog\nThree files.");
}
