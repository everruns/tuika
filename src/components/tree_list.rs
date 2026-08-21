//! Generic, host-data tree navigation with stable node identity.

use std::collections::{BTreeMap, BTreeSet};

use crate::geometry::Rect;
use crate::style::Style;
use crate::text::Line;

use crate::event::{Event, InputOutcome, KeyCode, MouseButton, MouseKind};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

use super::text::line_width;
use super::{Scrollbar, VirtualWindow};

/// One host-provided node in depth-first display order.
///
/// The host owns stable ids, labels, and the domain-specific traversal that
/// produces these rows. Tuika uses `parent`, `depth`, and `expandable` only for
/// generic visibility, navigation, and branch presentation.
#[derive(Clone, Debug)]
pub struct TreeRow<'a, K> {
    /// Stable application identity for this node. Ids must be unique within
    /// the supplied row collection.
    pub id: K,
    /// Stable id of the parent, or `None` for a root.
    pub parent: Option<K>,
    /// Display depth, where roots are zero.
    pub depth: usize,
    /// Host-provided display label.
    pub label: Line<'a>,
    /// Whether the node can be expanded.
    pub expandable: bool,
}

impl<'a, K> TreeRow<'a, K> {
    /// Create a tree row. Rows should be supplied in depth-first display order.
    pub fn new(
        id: K,
        parent: Option<K>,
        depth: usize,
        label: impl Into<Line<'a>>,
        expandable: bool,
    ) -> Self {
        Self {
            id,
            parent,
            depth,
            label: label.into(),
            expandable,
        }
    }

    /// Create a root row.
    pub fn root(id: K, label: impl Into<Line<'a>>, expandable: bool) -> Self {
        Self::new(id, None, 0, label, expandable)
    }
}

/// Host-owned expansion, stable selection, and persistent viewport state.
///
/// Call [`resolve`](Self::resolve) after refreshing rows or changing the
/// viewport height. Pass its exact [`VirtualWindow`] to both
/// [`TreeList::visible_window`] and [`handle_mouse`](Self::handle_mouse).
/// Selection survives reorder by id; a selected node hidden or removed by a
/// refresh falls back through its remembered ancestry to the nearest visible
/// ancestor.
#[derive(Clone, Debug)]
pub struct TreeState<K> {
    selected: Option<K>,
    expanded: BTreeSet<K>,
    offset: usize,
    follow_selection: bool,
    selected_path: Vec<K>,
}

impl<K> Default for TreeState<K> {
    fn default() -> Self {
        Self {
            selected: None,
            expanded: BTreeSet::new(),
            offset: 0,
            follow_selection: true,
            selected_path: Vec::new(),
        }
    }
}

impl<K> TreeState<K> {
    /// Create an unselected, fully collapsed tree at the top.
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently selected stable node id.
    pub fn selected(&self) -> Option<&K> {
        self.selected.as_ref()
    }

    /// First visible-row position retained across frames.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Set the first visible-row position directly.
    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
        self.follow_selection = false;
    }
}

impl<K: Clone + Ord> TreeState<K> {
    /// Create state selecting `id`.
    pub fn with_selected(id: K) -> Self {
        Self {
            selected: Some(id.clone()),
            selected_path: vec![id],
            ..Self::new()
        }
    }

    /// Select or clear a stable node id.
    pub fn select(&mut self, id: Option<K>) {
        self.selected_path = id.iter().cloned().collect();
        self.selected = id;
        self.follow_selection = true;
    }

    /// Whether `id` is expanded.
    pub fn is_expanded(&self, id: &K) -> bool {
        self.expanded.contains(id)
    }

    /// Expand `id`, returning whether state changed.
    pub fn expand(&mut self, id: K) -> bool {
        self.expanded.insert(id)
    }

    /// Collapse `id`, returning whether state changed.
    pub fn collapse(&mut self, id: &K) -> bool {
        self.expanded.remove(id)
    }

    /// Toggle expansion for `id`, returning its new expanded state.
    pub fn toggle(&mut self, id: K) -> bool {
        if self.expanded.remove(&id) {
            false
        } else {
            self.expanded.insert(id);
            true
        }
    }

    /// Iterate expanded node ids in stable key order.
    pub fn expanded(&self) -> impl Iterator<Item = &K> {
        self.expanded.iter()
    }

    /// Reconcile selection and return the exact visible-row window.
    pub fn resolve(&mut self, rows: &[TreeRow<'_, K>], viewport_rows: usize) -> VirtualWindow {
        let visible = visible_indices(rows, &self.expanded);
        self.reconcile_selection(rows, &visible);
        let len = viewport_rows.min(visible.len());
        self.offset = self
            .offset
            .min(VirtualWindow::max_start_for(visible.len(), len));
        if self.follow_selection
            && len > 0
            && let Some(position) = self.selected.as_ref().and_then(|selected| {
                visible
                    .iter()
                    .position(|&index| rows[index].id == *selected)
            })
        {
            if position < self.offset {
                self.offset = position;
            } else if position >= self.offset.saturating_add(len) {
                self.offset = position.saturating_add(1).saturating_sub(len);
            }
        }
        VirtualWindow::new(visible.len(), len, self.offset)
    }

    /// Navigate visible nodes with Up/Down, collapse-or-parent with Left,
    /// expand with Right, toggle with Enter, and scroll with the mouse wheel.
    pub fn handle(
        &mut self,
        event: &Event,
        rows: &[TreeRow<'_, K>],
        viewport_rows: usize,
    ) -> InputOutcome {
        let window = self.resolve(rows, viewport_rows);
        let visible = visible_indices(rows, &self.expanded);
        if let Event::Mouse(mouse) = event {
            let before = self.offset;
            match mouse.kind {
                MouseKind::ScrollUp => self.offset = self.offset.saturating_sub(3),
                MouseKind::ScrollDown => {
                    self.offset = self.offset.saturating_add(3).min(window.max_start())
                }
                _ => return InputOutcome::Ignored,
            }
            self.follow_selection = false;
            return if self.offset == before {
                InputOutcome::Consumed
            } else {
                InputOutcome::Changed
            };
        }
        let Event::Key(key) = event else {
            return InputOutcome::Ignored;
        };
        if !key.plain() {
            return InputOutcome::Ignored;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            visible
                .iter()
                .position(|&index| rows[index].id == *selected)
        });
        let outcome = match key.code {
            KeyCode::Up => {
                self.select_visible(rows, &visible, current.map_or(0, |p| p.saturating_sub(1)))
            }
            KeyCode::Down => self.select_visible(
                rows,
                &visible,
                current.map_or(0, |p| {
                    p.saturating_add(1).min(visible.len().saturating_sub(1))
                }),
            ),
            KeyCode::Left => self.left(rows),
            KeyCode::Right => self.right(rows),
            KeyCode::Enter => self.toggle_selected(rows),
            _ => InputOutcome::Ignored,
        };
        if matches!(outcome, InputOutcome::Changed | InputOutcome::Submitted) {
            self.follow_selection = true;
            let _ = self.resolve(rows, viewport_rows);
        }
        outcome
    }

    /// Hit-test a plain left click against the exact rendered `window`.
    /// Clicking a disclosure marker selects and toggles the node; clicking the
    /// rest of a row selects it without changing expansion.
    pub fn handle_mouse(
        &mut self,
        event: &Event,
        rows: &[TreeRow<'_, K>],
        bounds: Rect,
        window: VirtualWindow,
    ) -> InputOutcome {
        let Event::Mouse(mouse) = event else {
            return InputOutcome::Ignored;
        };
        if !mouse.plain()
            || mouse.kind != MouseKind::Down(MouseButton::Left)
            || mouse.column < bounds.x
            || mouse.column >= bounds.right()
            || mouse.row < bounds.y
            || mouse.row >= bounds.bottom()
        {
            return InputOutcome::Ignored;
        }
        let visible = visible_indices(rows, &self.expanded);
        let position = window.start() + usize::from(mouse.row - bounds.y);
        if position >= window.end() || position >= visible.len() {
            return InputOutcome::Ignored;
        }
        let row = &rows[visible[position]];
        let changed_selection = self.selected.as_ref() != Some(&row.id);
        self.selected = Some(row.id.clone());
        self.follow_selection = true;
        self.update_selected_path(rows);
        self.offset = window.start();
        let marker_x = bounds
            .x
            .saturating_add((row.depth.saturating_mul(2)) as u16);
        let toggled = row.expandable && mouse.column <= marker_x.saturating_add(1);
        if toggled {
            self.toggle(row.id.clone());
        }
        changed_outcome(changed_selection || toggled)
    }

    fn select_visible(
        &mut self,
        rows: &[TreeRow<'_, K>],
        visible: &[usize],
        position: usize,
    ) -> InputOutcome {
        let Some(row) = visible.get(position).and_then(|&index| rows.get(index)) else {
            return InputOutcome::Consumed;
        };
        if self.selected.as_ref() == Some(&row.id) {
            return InputOutcome::Consumed;
        }
        self.selected = Some(row.id.clone());
        self.follow_selection = true;
        self.update_selected_path(rows);
        InputOutcome::Changed
    }

    fn left(&mut self, rows: &[TreeRow<'_, K>]) -> InputOutcome {
        let Some(index) = self.selected_index(rows) else {
            return InputOutcome::Consumed;
        };
        if rows[index].expandable && self.collapse(&rows[index].id) {
            return InputOutcome::Changed;
        }
        let Some(parent) = rows[index].parent.clone() else {
            return InputOutcome::Consumed;
        };
        if self.selected.as_ref() == Some(&parent) {
            return InputOutcome::Consumed;
        }
        self.selected = Some(parent);
        self.follow_selection = true;
        self.update_selected_path(rows);
        InputOutcome::Changed
    }

    fn right(&mut self, rows: &[TreeRow<'_, K>]) -> InputOutcome {
        let Some(index) = self.selected_index(rows) else {
            return InputOutcome::Consumed;
        };
        if !rows[index].expandable {
            return InputOutcome::Consumed;
        }
        changed_outcome(self.expand(rows[index].id.clone()))
    }

    fn toggle_selected(&mut self, rows: &[TreeRow<'_, K>]) -> InputOutcome {
        let Some(index) = self.selected_index(rows) else {
            return InputOutcome::Consumed;
        };
        if !rows[index].expandable {
            return InputOutcome::Consumed;
        }
        self.toggle(rows[index].id.clone());
        InputOutcome::Changed
    }

    fn selected_index(&self, rows: &[TreeRow<'_, K>]) -> Option<usize> {
        self.selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| row.id == *selected))
    }

    fn reconcile_selection(&mut self, rows: &[TreeRow<'_, K>], visible: &[usize]) {
        let visible_id = |id: &K| visible.iter().any(|&index| rows[index].id == *id);
        if self.selected.as_ref().is_some_and(visible_id) {
            self.update_selected_path(rows);
            return;
        }

        let mut candidates = Vec::new();
        if let Some(index) = self.selected_index(rows) {
            candidates.extend(path_from(rows, index));
        }
        candidates.extend(self.selected_path.iter().cloned());
        self.selected = candidates.into_iter().find(visible_id);
        self.update_selected_path(rows);
    }

    fn update_selected_path(&mut self, rows: &[TreeRow<'_, K>]) {
        self.selected_path = self
            .selected_index(rows)
            .map_or_else(Vec::new, |index| path_from(rows, index));
    }
}

fn changed_outcome(changed: bool) -> InputOutcome {
    if changed {
        InputOutcome::Changed
    } else {
        InputOutcome::Consumed
    }
}

fn path_from<K: Clone + Ord>(rows: &[TreeRow<'_, K>], mut index: usize) -> Vec<K> {
    let indices: BTreeMap<&K, usize> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (&row.id, index))
        .collect();
    let mut path = Vec::new();
    for _ in 0..=rows.len() {
        let row = &rows[index];
        path.push(row.id.clone());
        let Some(parent) = row.parent.as_ref() else {
            break;
        };
        let Some(parent_index) = indices.get(parent).copied() else {
            break;
        };
        index = parent_index;
    }
    path
}

fn visible_indices<K: Ord>(rows: &[TreeRow<'_, K>], expanded: &BTreeSet<K>) -> Vec<usize> {
    let mut visibility = BTreeMap::new();
    let mut visible = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let row_visible = row.parent.as_ref().is_none_or(|parent| {
            expanded.contains(parent) && visibility.get(parent).copied().unwrap_or(false)
        });
        visibility.insert(&row.id, row_visible);
        if row_visible {
            visible.push(index);
        }
    }
    visible
}

/// A selectable, expandable tree over host-provided [`TreeRow`]s.
///
/// Domain traversal and labels remain in the host. `TreeList` owns generic
/// branch drawing, expansion visibility, stable-id selection presentation, and
/// scrollbar geometry.
///
/// ![tree list demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/tree_list.gif)
///
/// ```
/// use tuika::prelude::*;
/// use tuika::testing::{grid, render};
///
/// let rows = vec![
///     TreeRow::root(1, "workspace", true),
///     TreeRow::new(2, Some(1), 1, "src", false),
/// ];
/// let mut state = TreeState::with_selected(1);
/// state.expand(1);
/// let window = state.resolve(&rows, 4);
/// let tree = TreeList::new(&rows, &state).visible_window(window);
/// assert!(grid(&render(&tree, 16, 4, &Theme::default())).contains("src"));
/// ```
pub struct TreeList<'a, K> {
    rows: &'a [TreeRow<'a, K>],
    state: &'a TreeState<K>,
    viewport: Option<u16>,
    visible_window: Option<VirtualWindow>,
    scrollbar: bool,
    selection_style: Option<Style>,
}

impl<'a, K> TreeList<'a, K> {
    /// Build a tree from host-provided depth-first rows and state.
    pub fn new(rows: &'a [TreeRow<'a, K>], state: &'a TreeState<K>) -> Self {
        Self {
            rows,
            state,
            viewport: None,
            visible_window: None,
            scrollbar: true,
            selection_style: None,
        }
    }

    /// Cap the visible row count.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = Some(rows.max(1));
        self
    }

    /// Paint the exact window returned by [`TreeState::resolve`].
    pub fn visible_window(mut self, window: VirtualWindow) -> Self {
        self.visible_window = Some(window);
        self
    }

    /// Show the overflow scrollbar (default true).
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    /// Override the selected row style.
    pub fn selection_style(mut self, style: Style) -> Self {
        self.selection_style = Some(style);
        self
    }
}

impl<K: Clone + Ord> TreeList<'_, K> {
    fn window(&self, available: u16, visible: &[usize]) -> VirtualWindow {
        let rows = self.viewport.map_or(available, |cap| cap.min(available));
        self.visible_window.map_or_else(
            || VirtualWindow::new(visible.len(), usize::from(rows), self.state.offset),
            |window| {
                VirtualWindow::new(
                    visible.len(),
                    window.len().min(usize::from(rows)),
                    window.start(),
                )
            },
        )
    }
}

impl<K: Clone + Ord> View for TreeList<'_, K> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let visible = visible_indices(self.rows, &self.state.expanded);
        let window = self.window(available.height, &visible);
        let width = visible
            .iter()
            .map(|&index| {
                self.rows[index]
                    .depth
                    .saturating_mul(2)
                    .saturating_add(2)
                    .saturating_add(usize::from(line_width(&self.rows[index].label)))
            })
            .max()
            .unwrap_or(0)
            .min(usize::from(available.width)) as u16;
        Size::new(width, window.len().min(u16::MAX as usize) as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let visible = visible_indices(self.rows, &self.state.expanded);
        let last_sibling: BTreeMap<Option<&K>, usize> = visible
            .iter()
            .enumerate()
            .map(|(position, &index)| (self.rows[index].parent.as_ref(), position))
            .collect();
        let window = self.window(area.height, &visible);
        let overflow = window.overflows();
        let row_width = area
            .width
            .saturating_sub(u16::from(overflow && self.scrollbar));
        let right = area.x.saturating_add(row_width);
        let selection_style = self
            .selection_style
            .unwrap_or_else(|| ctx.theme.selection_style());

        for (screen_row, position) in window.range().enumerate() {
            let Some(&source_index) = visible.get(position) else {
                break;
            };
            let row = &self.rows[source_index];
            let y = area.y.saturating_add(screen_row as u16);
            if y >= area.bottom() {
                break;
            }
            let selected = self.state.selected.as_ref() == Some(&row.id);
            if selected {
                surface
                    .child(Rect::new(area.x, y, row_width, 1))
                    .fill(selection_style);
            }
            let style = if selected {
                selection_style
            } else {
                ctx.theme.text_style()
            };
            let muted = if selected {
                selection_style
            } else {
                ctx.theme.muted_style()
            };
            let mut x = area.x;
            if row.depth > 0 {
                for _ in 1..row.depth {
                    x = surface.set_string(x, y, "│ ", muted);
                }
                let has_later_sibling = last_sibling
                    .get(&row.parent.as_ref())
                    .is_some_and(|&last| last > position);
                x = surface.set_string(
                    x,
                    y,
                    if has_later_sibling {
                        "├─"
                    } else {
                        "└─"
                    },
                    muted,
                );
            }
            x = surface.set_string(
                x,
                y,
                if row.expandable {
                    if self.state.is_expanded(&row.id) {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                },
                muted,
            );
            for span in &row.label.spans {
                if x >= right {
                    break;
                }
                let span_style = if selected {
                    row.label.style.patch(span.style).patch(style)
                } else {
                    row.label.style.patch(span.style)
                };
                x = surface.set_string(x, y, span.content.as_ref(), span_style);
            }
        }
        if overflow && self.scrollbar && row_width < area.width {
            Scrollbar::vertical(window).render(
                Rect::new(
                    area.right() - 1,
                    area.y,
                    1,
                    area.height.min(window.len() as u16),
                ),
                surface,
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Key, Mouse};
    use crate::style::Theme;

    fn rows() -> Vec<TreeRow<'static, u8>> {
        vec![
            TreeRow::root(1, "root", true),
            TreeRow::new(2, Some(1), 1, "src", true),
            TreeRow::new(3, Some(2), 2, "lib.rs", false),
            TreeRow::new(4, Some(1), 1, "tests", false),
            TreeRow::root(5, "other", false),
        ]
    }

    #[test]
    fn navigation_expansion_parent_and_stable_refresh() {
        let rows = rows();
        let mut state = TreeState::with_selected(1);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Right)), &rows, 3),
            InputOutcome::Changed
        );
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Down)), &rows, 3),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(&2));
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter)), &rows, 3),
            InputOutcome::Changed
        );
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Down)), &rows, 3),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(&3));
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Left)), &rows, 3),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(&2));

        let reordered = vec![
            rows[4].clone(),
            rows[0].clone(),
            rows[1].clone(),
            rows[2].clone(),
            rows[3].clone(),
        ];
        let _ = state.resolve(&reordered, 3);
        assert_eq!(state.selected(), Some(&2));
    }

    #[test]
    fn refresh_falls_back_to_nearest_visible_ancestor() {
        let rows = rows();
        let mut state = TreeState::with_selected(3);
        state.expand(1);
        state.expand(2);
        let _ = state.resolve(&rows, 4);
        state.collapse(&2);
        let _ = state.resolve(&rows, 4);
        assert_eq!(state.selected(), Some(&2));

        let without_child = vec![
            rows[0].clone(),
            rows[1].clone(),
            rows[3].clone(),
            rows[4].clone(),
        ];
        let _ = state.resolve(&without_child, 4);
        assert_eq!(state.selected(), Some(&2));
    }

    #[test]
    fn mouse_uses_exact_window_for_selection_and_disclosure_toggle() {
        let rows = rows();
        let mut state = TreeState::with_selected(1);
        state.expand(1);
        let window = state.resolve(&rows, 3);
        let click_label = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 6, 2));
        assert_eq!(
            state.handle_mouse(&click_label, &rows, Rect::new(0, 0, 12, 3), window),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(&4));
        assert_eq!(state.resolve(&rows, 3).start(), window.start());

        let click_marker = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 0, 0));
        assert_eq!(
            state.handle_mouse(&click_marker, &rows, Rect::new(0, 0, 12, 3), window),
            InputOutcome::Changed
        );
        assert!(!state.is_expanded(&1));
    }

    #[test]
    fn renders_branches_scrollbar_theme_and_tiny_sizes() {
        let rows = rows();
        let mut state = TreeState::with_selected(3);
        state.expand(1);
        state.expand(2);
        let window = state.resolve(&rows, 3);
        let tree = TreeList::new(&rows, &state).visible_window(window);
        let theme = Theme::default();
        let buffer = crate::testing::render(&tree, 14, 3, &theme);
        let text = crate::testing::grid(&buffer);
        assert!(text.contains("├─") && text.contains("lib.rs"), "{text}");
        assert_eq!(buffer[(0, 2)].bg, theme.selection_bg);
        assert!((0..3).any(|y| matches!(buffer[(13, y)].symbol(), "█" | "│")));

        for width in 0..=8 {
            for height in 0..=5 {
                let _ = crate::testing::render(&tree, width, height, &theme);
            }
        }
    }
}
