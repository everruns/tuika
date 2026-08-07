//! Borrowed, keyed, virtualized tables.
//!
//! [`KeyedTable`] renders rows borrowed from a host collection and identifies
//! them with application-provided keys. Persistent selection therefore follows
//! a record through reordering, insertion, and filtering instead of attaching
//! identity to its current index. The keys are application data, not retained
//! view identity: the table is still rebuilt each frame and owns no lifecycle.
//! Keys must be unique within the authoritative collection; duplicate keys are
//! indistinguishable to selection state.

use std::collections::BTreeSet;

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use ratatui_core::text::Line;

use crate::event::{Event, InputOutcome, KeyCode, MouseButton, MouseKind};
use crate::geometry::Size;
use crate::layout::{Dimension, Item, LayoutStyle, solve};
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

use super::select::SelectNavigation;
use super::text::line_width;
use super::{Scrollbar, VirtualWindow};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CellAlign {
    #[default]
    Left,
    Right,
}

/// One column in a [`KeyedTable`].
///
/// The cell callback returns a borrowed, styled [`Line`]. It is invoked only
/// for rows in the resolved viewport, so a large collection does not allocate
/// or clone cells outside the visible window.
pub struct KeyedColumn<'a, R> {
    header: Line<'a>,
    width: Dimension,
    align: CellAlign,
    hide_below: Option<u16>,
    optional: bool,
    cell: Box<dyn for<'r> Fn(&'r R) -> Line<'r> + 'a>,
}

impl<'a, R> KeyedColumn<'a, R> {
    /// A column with an explicit width policy and borrowed cell callback.
    pub fn new<F>(header: impl Into<Line<'a>>, width: Dimension, cell: F) -> Self
    where
        F: for<'r> Fn(&'r R) -> Line<'r> + 'a,
    {
        Self {
            header: header.into(),
            width,
            align: CellAlign::Left,
            hide_below: None,
            optional: false,
            cell: Box::new(cell),
        }
    }

    /// A column sized from its header and visible cells.
    pub fn auto<F>(header: impl Into<Line<'a>>, cell: F) -> Self
    where
        F: for<'r> Fn(&'r R) -> Line<'r> + 'a,
    {
        Self::new(header, Dimension::Auto, cell)
    }

    /// A column with a fixed cell width.
    pub fn fixed<F>(header: impl Into<Line<'a>>, cells: u16, cell: F) -> Self
    where
        F: for<'r> Fn(&'r R) -> Line<'r> + 'a,
    {
        Self::new(header, Dimension::Fixed(cells), cell)
    }

    /// A column sharing leftover width by `weight`.
    pub fn flex<F>(header: impl Into<Line<'a>>, weight: u16, cell: F) -> Self
    where
        F: for<'r> Fn(&'r R) -> Line<'r> + 'a,
    {
        Self::new(header, Dimension::Flex(weight), cell)
    }

    /// Align cells against the trailing edge of the column.
    pub fn right(mut self) -> Self {
        self.align = CellAlign::Right;
        self
    }

    /// Hide this column while the table is narrower than `width`.
    pub fn hide_below(mut self, width: u16) -> Self {
        self.hide_below = Some(width);
        self
    }

    /// Mark the column as optional. Optional columns are removed from the end
    /// when their declared minimum widths do not fit after required columns.
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Host-owned single-selection and viewport state keyed by application identity.
#[derive(Clone, Debug)]
pub struct KeyedSelectState<K> {
    selected: Option<K>,
    offset: usize,
    scroll_margin: usize,
    follow_selection: bool,
}

impl<K> Default for KeyedSelectState<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> KeyedSelectState<K> {
    /// Create an unselected state at the top of the collection.
    pub const fn new() -> Self {
        Self {
            selected: None,
            offset: 0,
            scroll_margin: 0,
            follow_selection: true,
        }
    }

    /// Create state selecting `key`.
    pub const fn with_selected(key: K) -> Self {
        Self {
            selected: Some(key),
            offset: 0,
            scroll_margin: 0,
            follow_selection: true,
        }
    }

    /// The stable key currently selected, including while filtered out.
    pub fn selected(&self) -> Option<&K> {
        self.selected.as_ref()
    }

    /// Select or clear an application key directly.
    pub fn select(&mut self, key: Option<K>) {
        self.selected = key;
        self.follow_selection = true;
    }

    /// Top row of a manually scrolled viewport.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Set the top row explicitly, detaching the viewport from the selection.
    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
        self.follow_selection = false;
    }

    /// Keep this many context rows around keyboard and mouse selection when possible.
    pub fn set_scroll_margin(&mut self, rows: usize) {
        self.scroll_margin = rows;
    }

    /// Remove a selected key only when it is absent from the authoritative rows.
    ///
    /// Do not call this with a filtered slice: absence is intentionally
    /// preserved across filtering and streaming windows.
    pub fn retain_present<R, F>(&mut self, rows: &[R], key: F)
    where
        K: PartialEq,
        F: Fn(&R) -> &K,
    {
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !rows.iter().any(|row| key(row) == selected))
        {
            self.selected = None;
        }
    }

    /// Resolve the visible window for `rows`.
    pub fn window<R, F>(&self, rows: &[R], visible: usize, key: F) -> VirtualWindow
    where
        K: PartialEq,
        F: Fn(&R) -> &K,
    {
        let selected_index = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| key(row) == selected));
        self.window_at(rows.len(), visible, selected_index)
    }

    fn window_at(
        &self,
        total: usize,
        visible: usize,
        selected_index: Option<usize>,
    ) -> VirtualWindow {
        let len = visible.min(total);
        let mut start = self.offset.min(VirtualWindow::max_start_for(total, len));
        if self.follow_selection
            && let Some(index) = selected_index.filter(|&index| index < total)
        {
            let margin = self.scroll_margin.min(len.saturating_sub(1) / 2);
            let lower = start.saturating_add(margin);
            let upper = start.saturating_add(len.saturating_sub(margin));
            if index < lower {
                start = index.saturating_sub(margin);
            } else if index >= upper {
                start = index
                    .saturating_add(margin)
                    .saturating_add(1)
                    .saturating_sub(len);
            }
        }
        VirtualWindow::new(total, len, start)
    }
}

impl<K: Clone + PartialEq> KeyedSelectState<K> {
    /// Handle keyboard navigation and wheel scrolling over the visible rows.
    pub fn handle<R, F>(
        &mut self,
        event: &Event,
        rows: &[R],
        viewport_rows: usize,
        key: F,
    ) -> InputOutcome
    where
        F: Fn(&R) -> &K,
    {
        self.handle_with(event, rows, viewport_rows, key, SelectNavigation::default())
    }

    /// Handle input with the same optional aliases as [`SelectState`](super::SelectState).
    pub fn handle_with<R, F>(
        &mut self,
        event: &Event,
        rows: &[R],
        viewport_rows: usize,
        key: F,
        navigation: SelectNavigation,
    ) -> InputOutcome
    where
        F: Fn(&R) -> &K,
    {
        if let Event::Mouse(mouse) = event {
            let max = VirtualWindow::max_start_for(rows.len(), viewport_rows);
            let before = self.window(rows, viewport_rows, &key).start();
            self.offset = before;
            match mouse.kind {
                MouseKind::ScrollUp => self.offset = self.offset.saturating_sub(3),
                MouseKind::ScrollDown => self.offset = self.offset.saturating_add(3).min(max),
                _ => return InputOutcome::Ignored,
            }
            self.follow_selection = false;
            return if self.offset == before {
                InputOutcome::Consumed
            } else {
                InputOutcome::Changed
            };
        }
        let Event::Key(k) = event else {
            return InputOutcome::Ignored;
        };
        if rows.is_empty() {
            return match k.code {
                KeyCode::Esc if k.plain() => InputOutcome::Cancelled,
                _ => InputOutcome::Ignored,
            };
        }

        let current = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| key(row) == selected));
        let page = viewport_rows.saturating_sub(1).max(1);
        let command = if navigation.ctrl_n_p && k.ctrl && !k.alt && !k.shift {
            match k.code {
                KeyCode::Char('n') => Some(1isize),
                KeyCode::Char('p') => Some(-1),
                _ => None,
            }
        } else if k.plain() {
            match k.code {
                KeyCode::Down | KeyCode::Char('j') if k.code == KeyCode::Down || navigation.vim => {
                    Some(1)
                }
                KeyCode::Up | KeyCode::Char('k') if k.code == KeyCode::Up || navigation.vim => {
                    Some(-1)
                }
                KeyCode::Tab if navigation.tab => Some(1),
                KeyCode::BackTab if navigation.tab => Some(-1),
                KeyCode::PageDown => Some(page as isize),
                KeyCode::PageUp => Some(-(page as isize)),
                KeyCode::Home => Some(isize::MIN),
                KeyCode::End => Some(isize::MAX),
                KeyCode::Char(digit @ '1'..='9') if navigation.numeric => {
                    let index = digit as usize - '1' as usize;
                    if let Some(row) = rows.get(index) {
                        self.selected = Some(key(row).clone());
                        self.follow_selection = true;
                        return InputOutcome::Submitted;
                    }
                    return InputOutcome::Ignored;
                }
                KeyCode::Enter if current.is_some() => return InputOutcome::Submitted,
                KeyCode::Esc => return InputOutcome::Cancelled,
                _ => None,
            }
        } else {
            None
        };
        let Some(delta) = command else {
            return InputOutcome::Ignored;
        };
        let next = match delta {
            isize::MIN => 0,
            isize::MAX => rows.len() - 1,
            d if d < 0 => current
                .unwrap_or(rows.len())
                .saturating_sub(d.unsigned_abs()),
            d => current
                .map_or(0, |index| index.saturating_add(d as usize))
                .min(rows.len() - 1),
        };
        let before = self.selected.as_ref();
        let changed = before != Some(key(&rows[next]));
        self.selected = Some(key(&rows[next]).clone());
        self.follow_selection = true;
        if changed {
            InputOutcome::Changed
        } else {
            InputOutcome::Consumed
        }
    }

    /// Select the clicked row in an already resolved viewport.
    pub fn handle_mouse<R, F>(
        &mut self,
        event: &Event,
        rows: &[R],
        bounds: Rect,
        window: VirtualWindow,
        key: F,
    ) -> InputOutcome
    where
        F: Fn(&R) -> &K,
    {
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
        let index = window.start() + usize::from(mouse.row - bounds.y);
        let Some(row) = rows.get(index).filter(|_| index < window.end()) else {
            return InputOutcome::Ignored;
        };
        self.selected = Some(key(row).clone());
        self.follow_selection = true;
        InputOutcome::Submitted
    }
}

/// Keyed cursor plus a stable set of checked row keys.
#[derive(Clone, Debug, Default)]
pub struct KeyedMultiSelectState<K> {
    cursor: KeyedSelectState<K>,
    selected: BTreeSet<K>,
}

impl<K> KeyedMultiSelectState<K> {
    /// Create an empty multi-selection.
    pub fn new() -> Self {
        Self {
            cursor: KeyedSelectState::new(),
            selected: BTreeSet::new(),
        }
    }

    /// Cursor and viewport state.
    pub fn cursor(&self) -> &KeyedSelectState<K> {
        &self.cursor
    }

    /// Mutable cursor and viewport state.
    pub fn cursor_mut(&mut self) -> &mut KeyedSelectState<K> {
        &mut self.cursor
    }

    /// Whether a stable key is checked.
    pub fn contains(&self, key: &K) -> bool
    where
        K: Ord,
    {
        self.selected.contains(key)
    }

    /// Checked keys in key order.
    pub fn selected(&self) -> impl Iterator<Item = &K> {
        self.selected.iter()
    }

    /// Clear every checked key.
    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// Check `key`, returning whether the set changed.
    pub fn select(&mut self, key: K) -> bool
    where
        K: Ord,
    {
        self.selected.insert(key)
    }

    /// Uncheck `key`, returning whether the set changed.
    pub fn deselect(&mut self, key: &K) -> bool
    where
        K: Ord,
    {
        self.selected.remove(key)
    }

    /// Remove checked/cursor keys absent from authoritative rows.
    pub fn retain_present<R, F>(&mut self, rows: &[R], key: F)
    where
        K: Ord,
        F: Fn(&R) -> &K + Copy,
    {
        self.cursor.retain_present(rows, key);
        self.selected
            .retain(|selected| rows.iter().any(|row| key(row) == selected));
    }
}

impl<K: Clone + Ord> KeyedMultiSelectState<K> {
    /// Navigate and toggle with Enter or Space.
    pub fn handle<R, F>(
        &mut self,
        event: &Event,
        rows: &[R],
        viewport_rows: usize,
        key: F,
        navigation: SelectNavigation,
    ) -> InputOutcome
    where
        F: Fn(&R) -> &K + Copy,
    {
        if let Event::Key(k) = event
            && k.plain()
            && k.code == KeyCode::Char(' ')
        {
            let visible = self
                .cursor
                .selected()
                .is_some_and(|selected| rows.iter().any(|row| key(row) == selected));
            return if visible {
                self.toggle_cursor()
            } else {
                InputOutcome::Ignored
            };
        }
        match self
            .cursor
            .handle_with(event, rows, viewport_rows, key, navigation)
        {
            InputOutcome::Submitted => self.toggle_cursor(),
            outcome => outcome,
        }
    }

    /// Hit-test and toggle a row in an already resolved viewport.
    pub fn handle_mouse<R, F>(
        &mut self,
        event: &Event,
        rows: &[R],
        bounds: Rect,
        window: VirtualWindow,
        key: F,
    ) -> InputOutcome
    where
        F: Fn(&R) -> &K,
    {
        match self.cursor.handle_mouse(event, rows, bounds, window, key) {
            InputOutcome::Submitted => self.toggle_cursor(),
            outcome => outcome,
        }
    }

    fn toggle_cursor(&mut self) -> InputOutcome {
        let Some(key) = self.cursor.selected().cloned() else {
            return InputOutcome::Ignored;
        };
        self.toggle(key);
        InputOutcome::Changed
    }

    fn toggle(&mut self, key: K) {
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
    }
}

enum Selection<'a, K> {
    Single(&'a KeyedSelectState<K>),
    Multi(&'a KeyedMultiSelectState<K>),
}

impl<K: PartialEq> Selection<'_, K> {
    fn cursor(&self) -> &KeyedSelectState<K> {
        match self {
            Self::Single(state) => state,
            Self::Multi(state) => state.cursor(),
        }
    }

    fn checked(&self, key: &K) -> bool {
        match self {
            Self::Single(_) => false,
            Self::Multi(state) => state.selected().any(|selected| selected == key),
        }
    }

    fn is_multi(&self) -> bool {
        matches!(self, Self::Multi(_))
    }
}

/// A borrowed, keyed table that renders only its visible rows.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::testing::{grid, render};
///
/// struct Row { id: u64, label: String }
/// fn key(row: &Row) -> &u64 { &row.id }
/// fn label(row: &Row) -> Line<'_> { Line::from(row.label.as_str()) }
///
/// let rows = vec![
///     Row { id: 7, label: "build".into() },
///     Row { id: 9, label: "test".into() },
/// ];
/// let state = KeyedSelectState::with_selected(9);
/// let table = KeyedTable::new(
///     vec![KeyedColumn::flex("task", 1, label)],
///     &rows,
///     key,
///     &state,
/// );
/// assert!(grid(&render(&table, 16, 3, &Theme::default())).contains("test"));
/// ```
///
/// ![keyed table demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/keyed_table.gif)
///
/// Run the dynamic example with `cargo run --example keyed_table`.
pub struct KeyedTable<'a, R, K, F> {
    columns: Vec<KeyedColumn<'a, R>>,
    rows: &'a [R],
    key: F,
    selection: Selection<'a, K>,
    selected_index: Option<Option<usize>>,
    viewport: Option<u16>,
    scrollbar: bool,
    show_header: bool,
    gap: u16,
    caret: char,
    checked: char,
    unchecked: char,
    header_style: Option<Style>,
    selection_style: Option<Style>,
    preserve_selection_fg: bool,
}

impl<'a, R, K: PartialEq, F: Fn(&R) -> &K> KeyedTable<'a, R, K, F> {
    /// Build a table over borrowed rows with stable single-selection state.
    /// `key` must return a unique application key for each authoritative row.
    pub fn new(
        columns: Vec<KeyedColumn<'a, R>>,
        rows: &'a [R],
        key: F,
        state: &'a KeyedSelectState<K>,
    ) -> Self {
        Self::build(columns, rows, key, Selection::Single(state))
    }

    /// Build a table with keyed cursor and checked-row state.
    /// `key` follows the same uniqueness requirement as [`new`](Self::new).
    pub fn multi(
        columns: Vec<KeyedColumn<'a, R>>,
        rows: &'a [R],
        key: F,
        state: &'a KeyedMultiSelectState<K>,
    ) -> Self {
        Self::build(columns, rows, key, Selection::Multi(state))
    }

    fn build(
        columns: Vec<KeyedColumn<'a, R>>,
        rows: &'a [R],
        key: F,
        selection: Selection<'a, K>,
    ) -> Self {
        Self {
            columns,
            rows,
            key,
            selection,
            selected_index: None,
            viewport: None,
            scrollbar: true,
            show_header: true,
            gap: 2,
            caret: '›',
            checked: '✓',
            unchecked: '·',
            header_style: None,
            selection_style: None,
            preserve_selection_fg: false,
        }
    }

    /// Cap visible data rows below the assigned height.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = Some(rows.max(1));
        self
    }

    /// Show the overflow scrollbar.
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    /// Show the header row.
    pub fn header(mut self, show: bool) -> Self {
        self.show_header = show;
        self
    }

    /// Set the gap between visible columns.
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Set the cursor glyph in the leading indicator gutter.
    pub fn caret(mut self, caret: char) -> Self {
        self.caret = caret;
        self
    }

    /// Set the checked and unchecked glyphs used by multi-selection tables.
    pub fn check_glyphs(mut self, checked: char, unchecked: char) -> Self {
        self.checked = checked;
        self.unchecked = unchecked;
        self
    }

    /// Override the header style.
    pub fn header_style(mut self, style: Style) -> Self {
        self.header_style = Some(style);
        self
    }

    /// Override the cursor row style.
    pub fn selection_style(mut self, style: Style) -> Self {
        self.selection_style = Some(style);
        self
    }

    /// Keep each cell's foreground on the cursor row while applying the
    /// selection background. By default the complete selection style wins.
    pub fn preserve_selection_fg(mut self, preserve: bool) -> Self {
        self.preserve_selection_fg = preserve;
        self
    }

    /// Supply the selected key's position in the current ordered `rows` slice.
    ///
    /// This optional virtualization hint avoids scanning the whole collection
    /// to anchor the viewport. Keys remain the only persistent identity; the
    /// index is ephemeral and must be recomputed after reorder or filtering.
    pub fn selected_index(mut self, index: Option<usize>) -> Self {
        self.selected_index = Some(index);
        self
    }

    fn header_rows(&self) -> u16 {
        u16::from(self.show_header)
    }

    fn gutter_width(&self) -> u16 {
        if self.selection.is_multi() { 4 } else { 2 }
    }

    fn window(&self, body_rows: u16) -> VirtualWindow {
        let visible = self.viewport.map_or(body_rows, |rows| rows.min(body_rows));
        match self.selected_index {
            Some(index) => {
                self.selection
                    .cursor()
                    .window_at(self.rows.len(), usize::from(visible), index)
            }
            None => self
                .selection
                .cursor()
                .window(self.rows, usize::from(visible), &self.key),
        }
    }

    fn eligible_columns(&self, table_width: u16, fit_width: u16) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.hide_below.is_none_or(|min| table_width >= min))
            .map(|(index, _)| index)
            .collect();
        loop {
            let minimum = indices
                .iter()
                .map(|&index| match self.columns[index].width {
                    Dimension::Fixed(width) => width,
                    Dimension::Percent(percent) => fit_width.saturating_mul(percent) / 100,
                    Dimension::Auto => line_width(&self.columns[index].header),
                    Dimension::Flex(_) => 0,
                })
                .fold(0u16, u16::saturating_add)
                .saturating_add(
                    self.gap
                        .saturating_mul(indices.len().saturating_sub(1) as u16),
                );
            if minimum <= fit_width {
                break;
            }
            let Some(position) = indices
                .iter()
                .rposition(|&index| self.columns[index].optional)
            else {
                break;
            };
            indices.remove(position);
        }
        indices
    }

    fn solve_columns(&self, area: Rect, indices: &[usize], window: VirtualWindow) -> Vec<Rect> {
        let items = indices
            .iter()
            .map(|&index| {
                let column = &self.columns[index];
                let cells = window
                    .range()
                    .filter_map(|row| self.rows.get(row))
                    .map(|row| line_width(&(column.cell)(row)))
                    .max()
                    .unwrap_or(0);
                Item::new(
                    column.width,
                    Size::new(line_width(&column.header).max(cells), 1),
                )
            })
            .collect::<Vec<_>>();
        solve(area, &LayoutStyle::row().gap(self.gap), &items)
    }

    fn draw_line(
        &self,
        line: &Line<'_>,
        rect: Rect,
        y: u16,
        align: CellAlign,
        patch: Option<Style>,
        surface: &mut Surface,
    ) {
        let width = line_width(line).min(rect.width);
        let mut x = match align {
            CellAlign::Left => rect.x,
            CellAlign::Right => rect.right().saturating_sub(width),
        };
        let mut cell_surface = surface.child(Rect::new(rect.x, y, rect.width, 1));
        for span in &line.spans {
            if x >= rect.right() {
                break;
            }
            let style = patch.map_or(line.style.patch(span.style), |style| {
                line.style.patch(span.style).patch(style)
            });
            x = cell_surface.set_string(x, y, span.content.as_ref(), style);
        }
    }
}

impl<R, K: PartialEq, F: Fn(&R) -> &K> View for KeyedTable<'_, R, K, F> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let body = available.height.saturating_sub(self.header_rows());
        let window = self.window(body);
        let scrollbar = u16::from(self.scrollbar && window.overflows());
        let fit_width = available
            .width
            .saturating_sub(self.gutter_width())
            .saturating_sub(scrollbar);
        let indices = self.eligible_columns(available.width, fit_width);
        let width = indices
            .iter()
            .map(|&index| {
                let column = &self.columns[index];
                window
                    .range()
                    .filter_map(|row| self.rows.get(row))
                    .map(|row| line_width(&(column.cell)(row)))
                    .max()
                    .unwrap_or(0)
                    .max(line_width(&column.header))
            })
            .fold(self.gutter_width(), u16::saturating_add)
            .saturating_add(
                self.gap
                    .saturating_mul(indices.len().saturating_sub(1) as u16),
            );
        Size::new(
            width.min(available.width),
            self.header_rows().saturating_add(window.len() as u16),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() || self.columns.is_empty() {
            return;
        }
        let body_rows = area.height.saturating_sub(self.header_rows());
        let window = self.window(body_rows);
        let scrollbar_w = u16::from(self.scrollbar && window.overflows());
        let gutter = self.gutter_width();
        let columns_area = Rect::new(
            area.x.saturating_add(gutter),
            area.y,
            area.width
                .saturating_sub(gutter)
                .saturating_sub(scrollbar_w),
            area.height,
        );
        let indices = self.eligible_columns(area.width, columns_area.width);
        let rects = self.solve_columns(columns_area, &indices, window);

        if self.show_header {
            let style = self
                .header_style
                .unwrap_or_else(|| ctx.theme.accent_style());
            for (&index, &rect) in indices.iter().zip(&rects) {
                self.draw_line(
                    &self.columns[index].header,
                    rect,
                    area.y,
                    self.columns[index].align,
                    Some(style),
                    surface,
                );
            }
        }

        let body_y = area.y.saturating_add(self.header_rows());
        for (screen_row, index) in window.range().enumerate() {
            let Some(row) = self.rows.get(index) else {
                break;
            };
            let y = body_y.saturating_add(screen_row as u16);
            if y >= area.bottom() {
                break;
            }
            let key = (self.key)(row);
            let focused = self.selection.cursor().selected() == Some(key);
            let row_style = focused.then(|| {
                self.selection_style
                    .unwrap_or_else(|| ctx.theme.selection_style())
            });
            let cell_style = row_style.map(|style| {
                if self.preserve_selection_fg {
                    Style { fg: None, ..style }
                } else {
                    style
                }
            });
            if let Some(style) = row_style {
                surface
                    .child(Rect::new(
                        area.x,
                        y,
                        area.width.saturating_sub(scrollbar_w),
                        1,
                    ))
                    .fill(style);
            }
            surface.set(
                area.x,
                y,
                if focused { self.caret } else { ' ' },
                row_style.unwrap_or_else(|| ctx.theme.muted_style()),
            );
            if self.selection.is_multi() {
                surface.set(
                    area.x.saturating_add(2),
                    y,
                    if self.selection.checked(key) {
                        self.checked
                    } else {
                        self.unchecked
                    },
                    row_style.unwrap_or_else(|| ctx.theme.muted_style()),
                );
            }
            for (&column_index, &rect) in indices.iter().zip(&rects) {
                let column = &self.columns[column_index];
                let cell = (column.cell)(row);
                self.draw_line(&cell, rect, y, column.align, cell_style, surface);
            }
        }

        if scrollbar_w == 1 {
            Scrollbar::vertical(window).render(
                Rect::new(
                    area.right() - 1,
                    body_y,
                    1,
                    body_rows.min(window.len().min(u16::MAX as usize) as u16),
                ),
                surface,
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ratatui_core::style::{Color, Style};

    use super::*;
    use crate::event::{Key, Mouse};
    use crate::style::Theme;
    use crate::testing::{grid, render, render_sizes};
    use crate::tests::support::rainbow_theme;

    struct Row {
        id: u64,
        name: String,
        count: usize,
    }

    fn key(row: &Row) -> &u64 {
        &row.id
    }

    fn name(row: &Row) -> Line<'_> {
        Line::styled(row.name.as_str(), Style::default().fg(Color::Green))
    }

    fn count(row: &Row) -> Line<'_> {
        Line::from(row.count.to_string())
    }

    static CELL_CALLS: AtomicUsize = AtomicUsize::new(0);
    static KEY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counted_name(row: &Row) -> Line<'_> {
        CELL_CALLS.fetch_add(1, Ordering::Relaxed);
        Line::from(row.name.as_str())
    }

    fn counted_key(row: &Row) -> &u64 {
        KEY_CALLS.fetch_add(1, Ordering::Relaxed);
        &row.id
    }

    fn rows(ids: &[u64]) -> Vec<Row> {
        ids.iter()
            .map(|&id| Row {
                id,
                name: format!("row-{id}"),
                count: id as usize,
            })
            .collect()
    }

    #[test]
    fn single_selection_uses_keys_across_collection_changes() {
        let mut state = KeyedSelectState::with_selected(2);
        let reordered = rows(&[3, 1, 2]);
        assert_eq!(state.window(&reordered, 2, key).range(), 1..3);
        assert_eq!(state.selected(), Some(&2));

        let inserted = rows(&[9, 3, 1, 2]);
        assert!(state.window(&inserted, 2, key).contains(3));
        assert_eq!(state.selected(), Some(&2));

        let filtered = rows(&[3, 1]);
        let _ = state.window(&filtered, 2, key);
        assert_eq!(state.selected(), Some(&2), "filtering preserves identity");

        state.retain_present(&filtered, key);
        assert_eq!(state.selected(), None, "authoritative deletion clears it");
    }

    #[test]
    fn multi_selection_survives_reorder_filter_and_prunes_deletions() {
        let all = rows(&[1, 2, 3]);
        let mut state = KeyedMultiSelectState::new();
        let down = Event::Key(Key::new(KeyCode::Down));
        assert_eq!(
            state.handle(&down, &all, 3, key, SelectNavigation::default()),
            InputOutcome::Changed
        );
        assert_eq!(
            state.handle(
                &Event::Key(Key::new(KeyCode::Char(' '))),
                &all,
                3,
                key,
                SelectNavigation::default(),
            ),
            InputOutcome::Changed
        );
        assert!(state.contains(&1));
        let filtered = rows(&[3, 2]);
        assert!(state.contains(&1));
        let reordered = rows(&[3, 1, 2]);
        assert!(state.contains(&1));
        state.retain_present(&filtered, key);
        assert!(!state.contains(&1));
        assert_eq!(reordered.len(), 3);
    }

    #[test]
    fn multi_selection_does_not_toggle_a_filtered_cursor() {
        let all = rows(&[1, 2, 3]);
        let filtered = rows(&[1, 3]);
        let mut state = KeyedMultiSelectState::new();
        state.cursor_mut().select(Some(2));
        let outcome = state.handle(
            &Event::Key(Key::new(KeyCode::Char(' '))),
            &filtered,
            2,
            key,
            SelectNavigation::default(),
        );
        assert_eq!(outcome, InputOutcome::Ignored);
        assert!(!state.contains(&2));
        assert_eq!(state.cursor().selected(), Some(&2));
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn window_honors_scroll_margin_and_manual_wheel_scrolling() {
        let rows = rows(&(0..20).collect::<Vec<_>>());
        let mut state = KeyedSelectState::with_selected(10);
        state.set_scroll_margin(2);
        assert_eq!(state.window(&rows, 6, key).range(), 7..13);
        let wheel = Event::Mouse(Mouse::at(MouseKind::ScrollDown, 0, 0));
        assert_eq!(state.handle(&wheel, &rows, 6, key), InputOutcome::Changed);
        assert_eq!(state.window(&rows, 6, key).start(), 10);
        let end = Event::Key(Key::new(KeyCode::End));
        let _ = state.handle(&end, &rows, 6, key);
        assert_eq!(state.window(&rows, 6, key).range(), 14..20);
    }

    #[test]
    fn mouse_maps_through_nonzero_window_and_ignores_header() {
        let rows = rows(&(0..10).collect::<Vec<_>>());
        let mut state = KeyedSelectState::new();
        state.set_offset(4);
        let window = state.window(&rows, 3, key);
        let body = Rect::new(2, 5, 20, 3);
        let click = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 3, 6));
        assert_eq!(
            state.handle_mouse(&click, &rows, body, window, key),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), Some(&5));
        let header = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 3, 4));
        assert_eq!(
            state.handle_mouse(&header, &rows, body, window, key),
            InputOutcome::Ignored
        );
    }

    #[test]
    fn responsive_columns_alignment_and_styles_render() {
        let rows = rows(&[1]);
        let state = KeyedSelectState::new();
        let narrow = KeyedTable::new(
            vec![
                KeyedColumn::fixed("name", 8, name),
                KeyedColumn::fixed("count", 5, count).right().hide_below(18),
                KeyedColumn::fixed("extra", 8, name).optional(),
            ],
            &rows,
            key,
            &state,
        )
        .gap(1);
        let buf = render(&narrow, 16, 2, &Theme::default());
        let text = grid(&buf);
        assert!(!text.contains("count"));
        assert!(!text.contains("extra"));
        assert_eq!(buf[(2, 1)].fg, Color::Green, "borrowed cell style kept");

        let wide = KeyedTable::new(
            vec![
                KeyedColumn::flex("name", 1, name),
                KeyedColumn::fixed("count", 5, count).right(),
            ],
            &rows,
            key,
            &state,
        )
        .gap(1);
        let text = grid(&render(&wide, 20, 2, &Theme::default()));
        assert!(text.lines().nth(1).unwrap().ends_with("    1"));
    }

    #[test]
    fn responsive_breakpoint_uses_total_table_width() {
        let rows = rows(&(0..20).collect::<Vec<_>>());
        let state = KeyedSelectState::new();
        let build = || {
            KeyedTable::new(
                vec![
                    KeyedColumn::fixed("x", 1, count).hide_below(10),
                    KeyedColumn::flex("name", 1, name),
                ],
                &rows,
                key,
                &state,
            )
            .gap(0)
        };
        assert!(grid(&render(&build(), 10, 3, &Theme::default())).contains('x'));
        assert!(!grid(&render(&build(), 9, 3, &Theme::default())).contains('x'));
    }

    #[test]
    fn capped_viewport_caps_the_scrollbar_track() {
        let rows = rows(&(0..20).collect::<Vec<_>>());
        let state = KeyedSelectState::new();
        let table = KeyedTable::new(vec![KeyedColumn::flex("name", 1, name)], &rows, key, &state)
            .viewport(3);
        let buf = render(&table, 12, 8, &Theme::default());
        assert!((1..=3).all(|y| matches!(buf[(11, y)].symbol(), "█" | "│")));
        assert_eq!(buf[(11, 4)].symbol(), " ");
    }

    #[test]
    fn multi_table_draws_cursor_and_check_indicators() {
        let rows = rows(&[1, 2]);
        let mut state = KeyedMultiSelectState::new();
        state.cursor_mut().select(Some(2));
        let _ = state.handle(
            &Event::Key(Key::new(KeyCode::Char(' '))),
            &rows,
            2,
            key,
            SelectNavigation::default(),
        );
        let table = KeyedTable::multi(vec![KeyedColumn::flex("name", 1, name)], &rows, key, &state)
            .check_glyphs('x', '.');
        let text = grid(&render(&table, 16, 3, &Theme::default()));
        assert!(text.lines().nth(2).unwrap().starts_with("› x "));
    }

    #[test]
    fn cells_clip_to_columns_and_right_alignment_uses_display_width() {
        let rows = vec![Row {
            id: 1,
            name: "界".into(),
            count: 7,
        }];
        let state = KeyedSelectState::new();
        let table = KeyedTable::new(
            vec![
                KeyedColumn::fixed("", 4, name).right(),
                KeyedColumn::fixed("n", 2, count),
            ],
            &rows,
            key,
            &state,
        )
        .gap(0)
        .header(false);
        let buf = render(&table, 8, 1, &Theme::default());
        assert_eq!(buf[(4, 0)].symbol(), "界", "wide cell is right aligned");
        assert_eq!(buf[(6, 0)].symbol(), "7", "next column remains intact");

        let rows = vec![Row {
            id: 1,
            name: "abcdefgh".into(),
            count: 7,
        }];
        let table = KeyedTable::new(
            vec![
                KeyedColumn::fixed("", 4, name),
                KeyedColumn::fixed("n", 2, count),
            ],
            &rows,
            key,
            &state,
        )
        .gap(0)
        .header(false);
        let text = grid(&render(&table, 8, 1, &Theme::default()));
        assert_eq!(text, "  abcd7 ");

        let rows = vec![Row {
            id: 1,
            name: "界".into(),
            count: 7,
        }];
        let table = KeyedTable::new(
            vec![
                KeyedColumn::fixed("", 1, name),
                KeyedColumn::fixed("", 1, count),
            ],
            &rows,
            key,
            &state,
        )
        .gap(0)
        .header(false);
        let buf = render(&table, 4, 1, &Theme::default());
        assert_eq!(buf[(2, 0)].symbol(), " ", "wide cell is dropped");
        assert_eq!(buf[(3, 0)].symbol(), "7", "adjacent column is intact");
    }

    #[test]
    fn theme_slots_style_header_selection_and_inactive_gutter() {
        let rows = rows(&[1, 2]);
        let state = KeyedSelectState::with_selected(1);
        let table = KeyedTable::new(vec![KeyedColumn::flex("name", 1, name)], &rows, key, &state);
        let theme = rainbow_theme();
        let buf = render(&table, 14, 3, &theme);
        assert_eq!(buf[(2, 0)].fg, theme.accent, "header uses accent");
        assert_eq!(buf[(2, 1)].fg, theme.selection_fg);
        assert_eq!(buf[(2, 1)].bg, theme.selection_bg);
        assert_eq!(buf[(0, 2)].fg, theme.muted, "inactive caret uses muted");

        let preserved =
            KeyedTable::new(vec![KeyedColumn::flex("name", 1, name)], &rows, key, &state)
                .preserve_selection_fg(true);
        let buf = render(&preserved, 14, 3, &theme);
        assert_eq!(buf[(2, 1)].fg, Color::Green);
        assert_eq!(buf[(2, 1)].bg, theme.selection_bg);
    }

    #[test]
    fn keyed_table_survives_degenerate_size_sweep() {
        let rows = rows(&(0..20).collect::<Vec<_>>());
        let state = KeyedSelectState::with_selected(10);
        let table = KeyedTable::new(
            vec![
                KeyedColumn::flex("name", 1, name),
                KeyedColumn::fixed("count", 5, count).right(),
            ],
            &rows,
            key,
            &state,
        );
        let sizes = (0..=8).flat_map(|width| (0..=5).map(move |height| (width, height)));
        let _ = render_sizes(&table, sizes, &Theme::default());
    }

    #[test]
    fn non_clone_rows_are_borrowed_and_only_visible_cells_render() {
        struct Borrowed<'a> {
            id: u64,
            label: &'a str,
        }
        fn borrowed_key<'r, 'data>(row: &'r Borrowed<'data>) -> &'r u64 {
            &row.id
        }
        fn borrowed_label<'r, 'data>(row: &'r Borrowed<'data>) -> Line<'r> {
            Line::from(row.label)
        }

        let labels = [
            String::from("zero"),
            String::from("one"),
            String::from("two"),
        ];
        let borrowed_rows = labels
            .iter()
            .enumerate()
            .map(|(id, label)| Borrowed {
                id: id as u64,
                label,
            })
            .collect::<Vec<_>>();
        let state = KeyedSelectState::new();
        let table = KeyedTable::new(
            vec![KeyedColumn::auto("label", borrowed_label)],
            &borrowed_rows,
            borrowed_key,
            &state,
        )
        .viewport(1);
        let text = grid(&render(&table, 12, 2, &Theme::default()));
        assert!(text.contains("zero"));

        CELL_CALLS.store(0, Ordering::Relaxed);
        let source = rows(&(0..100).collect::<Vec<_>>());
        let state = KeyedSelectState::new();
        let table = KeyedTable::new(
            vec![KeyedColumn::auto("name", counted_name)],
            &source,
            key,
            &state,
        )
        .viewport(3);
        let _ = render(&table, 12, 4, &Theme::default());
        assert_eq!(
            CELL_CALLS.load(Ordering::Relaxed),
            6,
            "three visible cells are measured for column layout and painted"
        );

        KEY_CALLS.store(0, Ordering::Relaxed);
        let state = KeyedSelectState::with_selected(99);
        let table = KeyedTable::new(
            vec![KeyedColumn::auto("name", counted_name)],
            &source,
            counted_key,
            &state,
        )
        .selected_index(Some(99))
        .viewport(3);
        let _ = render(&table, 12, 4, &Theme::default());
        assert_eq!(
            KEY_CALLS.load(Ordering::Relaxed),
            3,
            "a position hint limits key access to visible rows"
        );
    }

    #[test]
    fn single_selection_keys_need_equality_but_not_ordering() {
        #[derive(PartialEq)]
        struct Key(&'static str);
        struct Record {
            key: Key,
            label: &'static str,
        }
        fn record_key(record: &Record) -> &Key {
            &record.key
        }
        fn record_label(record: &Record) -> Line<'_> {
            Line::from(record.label)
        }

        let rows = [Record {
            key: Key("a"),
            label: "alpha",
        }];
        let state = KeyedSelectState::with_selected(Key("a"));
        let table = KeyedTable::new(
            vec![KeyedColumn::auto("label", record_label)],
            &rows,
            record_key,
            &state,
        );
        assert!(grid(&render(&table, 10, 2, &Theme::default())).contains("alpha"));
    }
}
