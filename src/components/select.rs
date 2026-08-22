//! Selectable list (Pi's `SelectList`).
//!
//! [`SelectState`] persists an optional highlighted index and handles
//! up/down/wrap navigation; [`SelectList`] renders the options, marking the
//! current one with a theme-default or instance selection style and a caret.
//! Enter is surfaced as [`InputOutcome::Submitted`] so the caller decides what
//! "confirm" means.

use crate::geometry::Rect;
use crate::style::Style;
use crate::text::Line;

use crate::event::{Event, InputOutcome, KeyCode, MouseButton, MouseKind};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

use super::{Scrollbar, SelectionAnchor, VirtualWindow};

/// Optional navigation bindings for [`SelectState`].
///
/// [`Default`] preserves the original arrow/Enter/Escape behavior. Use
/// [`common`](Self::common) for the aliases commonly expected by terminal
/// pickers, then disable individual groups as needed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectNavigation {
    /// Enable `j` and `k` as Down and Up.
    pub vim: bool,
    /// Enable Ctrl+N and Ctrl+P as Down and Up.
    pub ctrl_n_p: bool,
    /// Enable Tab and Shift+Tab as Down and Up.
    pub tab: bool,
    /// Enable `1` through `9` as direct activation shortcuts.
    pub numeric: bool,
}

impl SelectNavigation {
    /// Enable all common terminal-picker aliases.
    pub const fn common() -> Self {
        Self {
            vim: true,
            ctrl_n_p: true,
            tab: true,
            numeric: true,
        }
    }
}

/// Persisted optional selection index for one list.
#[derive(Clone, Copy, Debug)]
pub struct SelectState {
    selected: Option<usize>,
}

/// Host-owned selection plus a persistent visible-window start.
///
/// Unlike [`SelectList::viewport`], which derives a selection-centered window
/// independently on every frame, this state moves the window only when the
/// selected row crosses an edge. Resolve a [`VirtualWindow`] once with
/// [`resolve`](Self::resolve), pass that exact value to
/// [`SelectList::visible_window`] (or [`Table::visible_window`](super::Table::visible_window)),
/// and reuse it for [`handle_mouse`](Self::handle_mouse). This makes click hit
/// testing refer to the same rows that were painted.
#[derive(Clone, Copy, Debug)]
pub struct SelectViewportState {
    selection: SelectState,
    offset: usize,
    follow_selection: bool,
}

impl Default for SelectViewportState {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectViewportState {
    /// Create state with the first row selected and the window at the top.
    pub const fn new() -> Self {
        Self {
            selection: SelectState { selected: Some(0) },
            offset: 0,
            follow_selection: true,
        }
    }

    /// Create state with no selected row and the window at the top.
    pub const fn unselected() -> Self {
        Self {
            selection: SelectState { selected: None },
            offset: 0,
            follow_selection: true,
        }
    }

    /// Selection state used by [`SelectList`] and [`Table`](super::Table).
    pub const fn selection(&self) -> &SelectState {
        &self.selection
    }

    /// Currently selected absolute row.
    pub fn selected(&self) -> Option<usize> {
        self.selection.selected()
    }

    /// Select or clear an absolute row. The next [`resolve`](Self::resolve)
    /// minimally scrolls it into view.
    pub fn select(&mut self, index: Option<usize>) {
        self.selection.select(index);
        self.follow_selection = true;
    }

    /// First visible row retained across frames.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Set the first visible row directly. Resolution clamps it to the current
    /// collection and viewport.
    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
        self.follow_selection = false;
    }

    /// Reconcile collection size, viewport size, selection, and top row.
    ///
    /// The returned value is the exact token to use for both rendering and
    /// subsequent mouse hit testing. A selected row already inside the window
    /// never changes its start.
    pub fn resolve(&mut self, len: usize, visible: usize) -> VirtualWindow {
        self.selection.clamp(len);
        // The persistent offset is exactly `keeping`'s `start`: the stateless
        // policy and this one are the same math, differing only in whether the
        // caller remembers where the window was.
        let anchor = self
            .follow_selection
            .then(|| self.selection.selected())
            .flatten();
        let window = VirtualWindow::keeping(len, visible, self.offset, anchor);
        self.offset = window.start();
        window
    }

    /// Handle keyboard navigation and wheel scrolling, then reconcile the
    /// persistent window for the supplied dimensions.
    pub fn handle(&mut self, event: &Event, len: usize, viewport_rows: usize) -> InputOutcome {
        self.handle_with(event, len, viewport_rows, SelectNavigation::default())
    }

    /// Handle input with configurable selection aliases.
    pub fn handle_with(
        &mut self,
        event: &Event,
        len: usize,
        viewport_rows: usize,
        navigation: SelectNavigation,
    ) -> InputOutcome {
        let before = self.resolve(len, viewport_rows);
        if let Event::Mouse(mouse) = event {
            match mouse.kind {
                MouseKind::ScrollUp => self.offset = self.offset.saturating_sub(3),
                MouseKind::ScrollDown => {
                    self.offset = self.offset.saturating_add(3).min(before.max_start())
                }
                _ => return InputOutcome::Ignored,
            }
            self.follow_selection = false;
            return if self.offset == before.start() {
                InputOutcome::Consumed
            } else {
                InputOutcome::Changed
            };
        }
        let outcome = self.selection.handle_with(event, len, navigation);
        if matches!(outcome, InputOutcome::Changed | InputOutcome::Submitted) {
            self.follow_selection = true;
            let _ = self.resolve(len, viewport_rows);
        }
        outcome
    }

    /// Select a clicked row from the exact window used to render `bounds`.
    pub fn handle_mouse(
        &mut self,
        event: &Event,
        len: usize,
        bounds: Rect,
        window: VirtualWindow,
    ) -> InputOutcome {
        let outcome = self
            .selection
            .handle_mouse(event, len, bounds, window.start());
        if outcome == InputOutcome::Submitted {
            self.offset = window.start();
            self.follow_selection = true;
            let _ = self.resolve(len, window.len());
        }
        outcome
    }
}

impl Default for SelectState {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectState {
    /// A fresh state with the first row highlighted.
    pub fn new() -> Self {
        Self { selected: Some(0) }
    }

    /// A fresh state with no highlighted row.
    pub fn unselected() -> Self {
        Self { selected: None }
    }

    /// The currently highlighted index, or `None` when no row is selected.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Set or clear the highlighted index directly. Lets a host drive the
    /// selection from its own optional state.
    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index;
    }

    /// Keep the index in range as the list length changes.
    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = None;
        } else if self.selected.is_some_and(|selected| selected >= len) {
            self.selected = Some(len - 1);
        }
    }

    /// Move the highlight up one row, clamping at the top (no wrap). The
    /// non-wrapping stepping primitive; use it when a picker holds at the ends
    /// rather than wrapping the way [`handle`](Self::handle) does.
    pub fn move_up(&mut self) {
        if let Some(selected) = self.selected {
            self.selected = Some(selected.saturating_sub(1));
        }
    }

    /// Move the highlight down one row, clamping at the last of `len` rows (no
    /// wrap). The non-wrapping counterpart to [`move_up`](Self::move_up).
    pub fn move_down(&mut self, len: usize) {
        if len == 0 {
            self.selected = None;
        } else if let Some(selected) = self.selected {
            self.selected = Some((selected + 1).min(len - 1));
        }
    }

    /// Navigate with arrow keys (wrapping), confirm with Enter, cancel on Esc.
    pub fn handle(&mut self, event: &Event, len: usize) -> InputOutcome {
        self.handle_with(event, len, SelectNavigation::default())
    }

    /// Handle keyboard input with a configurable navigation policy.
    pub fn handle_with(
        &mut self,
        event: &Event,
        len: usize,
        navigation: SelectNavigation,
    ) -> InputOutcome {
        if len == 0 {
            return InputOutcome::Ignored;
        }
        let Event::Key(k) = event else {
            return InputOutcome::Ignored;
        };

        if navigation.ctrl_n_p && k.ctrl && !k.alt && !k.shift {
            return match k.code {
                KeyCode::Char('n') => self.step_down(len),
                KeyCode::Char('p') => self.step_up(len),
                _ => InputOutcome::Ignored,
            };
        }
        if !k.plain() {
            return InputOutcome::Ignored;
        }
        match k.code {
            KeyCode::Up => self.step_up(len),
            KeyCode::Down => self.step_down(len),
            KeyCode::Char('k') if navigation.vim => self.step_up(len),
            KeyCode::Char('j') if navigation.vim => self.step_down(len),
            KeyCode::Tab if navigation.tab => self.step_down(len),
            KeyCode::BackTab if navigation.tab => self.step_up(len),
            KeyCode::Char(digit @ '1'..='9') if navigation.numeric => {
                let index = digit as usize - '1' as usize;
                if index < len {
                    self.selected = Some(index);
                    InputOutcome::Submitted
                } else {
                    InputOutcome::Ignored
                }
            }
            KeyCode::Enter if self.selected.is_some() => InputOutcome::Submitted,
            KeyCode::Esc => InputOutcome::Cancelled,
            _ => InputOutcome::Ignored,
        }
    }

    /// Hit-test a plain left-button press against visible list rows.
    ///
    /// `bounds` is the rendered list body and `first_visible` is the item shown
    /// on its first row. Supplying the scroll offset explicitly keeps mouse
    /// selection correct for viewported lists.
    pub fn handle_mouse(
        &mut self,
        event: &Event,
        len: usize,
        bounds: Rect,
        first_visible: usize,
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
        let index = first_visible.saturating_add(usize::from(mouse.row - bounds.y));
        if index >= len {
            return InputOutcome::Ignored;
        }
        self.selected = Some(index);
        InputOutcome::Submitted
    }

    fn step_up(&mut self, len: usize) -> InputOutcome {
        let before = self.selected;
        self.selected = Some(match self.selected {
            Some(selected) if selected > 0 && selected < len => selected - 1,
            _ => len - 1,
        });
        if self.selected == before {
            InputOutcome::Consumed
        } else {
            InputOutcome::Changed
        }
    }

    fn step_down(&mut self, len: usize) -> InputOutcome {
        let before = self.selected;
        self.selected = Some(match self.selected {
            Some(selected) if selected < len - 1 => selected + 1,
            _ => 0,
        });
        if self.selected == before {
            InputOutcome::Consumed
        } else {
            InputOutcome::Changed
        }
    }
}

/// Cursor and checked-item state for a multiple-selection list.
#[derive(Clone, Debug, Default)]
pub struct MultiSelectState {
    cursor: SelectState,
    selected: std::collections::BTreeSet<usize>,
}

impl MultiSelectState {
    /// Create state with the first row highlighted and no checked items.
    pub fn new() -> Self {
        Self {
            cursor: SelectState::new(),
            selected: std::collections::BTreeSet::new(),
        }
    }

    /// Cursor state used to render a [`SelectList`].
    pub fn cursor(&self) -> &SelectState {
        &self.cursor
    }

    /// Mutable cursor state for direct host control.
    pub fn cursor_mut(&mut self) -> &mut SelectState {
        &mut self.cursor
    }

    /// Whether `index` is checked.
    pub fn contains(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    /// Checked indices in ascending order.
    pub fn selected(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected.iter().copied()
    }

    /// Clear every checked item.
    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// Navigate and toggle with Enter or Space.
    pub fn handle(
        &mut self,
        event: &Event,
        len: usize,
        navigation: SelectNavigation,
    ) -> InputOutcome {
        if let Event::Key(key) = event
            && key.plain()
            && key.code == KeyCode::Char(' ')
        {
            return self.toggle_cursor(len);
        }
        match self.cursor.handle_with(event, len, navigation) {
            InputOutcome::Submitted => {
                let Some(index) = self.cursor.selected() else {
                    return InputOutcome::Ignored;
                };
                self.toggle(index);
                InputOutcome::Changed
            }
            outcome => outcome,
        }
    }

    /// Hit-test and toggle a visible row on a plain left click.
    pub fn handle_mouse(
        &mut self,
        event: &Event,
        len: usize,
        bounds: Rect,
        first_visible: usize,
    ) -> InputOutcome {
        match self.cursor.handle_mouse(event, len, bounds, first_visible) {
            InputOutcome::Submitted => {
                let Some(index) = self.cursor.selected() else {
                    return InputOutcome::Ignored;
                };
                self.toggle(index);
                InputOutcome::Changed
            }
            outcome => outcome,
        }
    }

    fn toggle_cursor(&mut self, len: usize) -> InputOutcome {
        self.cursor.clamp(len);
        let Some(index) = self.cursor.selected() else {
            return InputOutcome::Ignored;
        };
        self.toggle(index);
        InputOutcome::Changed
    }

    fn toggle(&mut self, index: usize) {
        if !self.selected.remove(&index) {
            self.selected.insert(index);
        }
    }
}

/// Renders `items` with the selected row highlighted. A state whose
/// [`selected`](SelectState::selected) value is `None` draws no caret or band.
/// With a [`viewport`] set,
/// a list taller than the viewport is windowed around the selection and a
/// scrollbar is drawn — the primitive for long pickers (hundreds of models).
///
/// [`viewport`]: SelectList::viewport
///
/// # Example
///
/// ```
/// use tuika::ui::Line;
/// use tuika::prelude::*;
/// use tuika::testing::{grid, render};
///
/// // A fresh state highlights the first row; the caret `›` marks it.
/// let state = SelectState::new();
/// let items = vec![Line::from("one"), Line::from("two")];
/// let view = SelectList::new(items, &state);
///
/// let buffer = render(&view, 5, 2, &Theme::default());
/// assert_eq!(grid(&buffer), "› one\n  two");
/// ```
///
/// ![select demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/select.gif)
pub struct SelectList {
    items: Vec<Line<'static>>,
    /// Host-provided source window, or `None` when `items` is the whole list.
    source_window: Option<VirtualWindow>,
    selected: Option<usize>,
    /// Max visible rows; `None` shows the whole list.
    viewport: Option<u16>,
    visible_window: Option<VirtualWindow>,
    scrollbar: bool,
    selection_style: Option<Style>,
    selection_anchor: SelectionAnchor,
}

pub(crate) struct SelectRows<'items> {
    items: &'items [Line<'static>],
    source_window: Option<VirtualWindow>,
    selected: Option<usize>,
    viewport: Option<u16>,
    visible_window: Option<VirtualWindow>,
    scrollbar: bool,
    selection_style: Option<Style>,
    selection_anchor: SelectionAnchor,
}

impl<'items> SelectRows<'items> {
    pub(crate) fn borrowed(items: &'items [Line<'static>], state: &SelectState) -> Self {
        Self {
            items,
            source_window: None,
            selected: state.selected(),
            viewport: None,
            visible_window: None,
            scrollbar: true,
            selection_style: None,
            selection_anchor: SelectionAnchor::default(),
        }
    }

    pub(crate) fn windowed(
        items: &'items [Line<'static>],
        window: VirtualWindow,
        state: &SelectState,
    ) -> Self {
        Self {
            source_window: Some(window),
            ..Self::borrowed(items, state)
        }
    }

    pub(crate) fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    pub(crate) fn selection_style(mut self, style: Option<Style>) -> Self {
        self.selection_style = style;
        self
    }

    fn window(&self, available_rows: Option<u16>) -> VirtualWindow {
        let visible = self
            .viewport
            .map(usize::from)
            .unwrap_or(usize::MAX)
            .min(available_rows.map(usize::from).unwrap_or(usize::MAX));
        if let Some(window) = self.visible_window {
            return VirtualWindow::new(window.total(), window.len().min(visible), window.start());
        }
        match self.source_window {
            Some(source) => {
                let len = source.len().min(visible);
                if len == source.len() {
                    source
                } else {
                    let selected = self.selected.unwrap_or(source.start());
                    let centered = selected.saturating_sub(len / 2);
                    let max_start = source.end().saturating_sub(len);
                    VirtualWindow::new(
                        source.total(),
                        len,
                        centered.clamp(source.start(), max_start),
                    )
                }
            }
            None => self
                .selection_anchor
                .window(self.items.len(), visible, self.selected),
        }
    }
}

impl View for SelectRows<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let width = self
            .items
            .iter()
            .map(super::text::line_width)
            .max()
            .unwrap_or(0)
            .saturating_add(2);
        let rows = self.window(None).len();
        Size::new(
            width.min(available.width),
            rows.min(u16::MAX as usize) as u16,
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let window = self.window(Some(area.height));
        let start = window.start();
        let rows = window.len();
        let overflow = window.overflows();
        let row_width = if overflow && self.scrollbar {
            area.width.saturating_sub(1)
        } else {
            area.width
        };
        let row_right = area.x.saturating_add(row_width);
        let selection_style = self
            .selection_style
            .unwrap_or_else(|| ctx.theme.selection_style());
        let items_start = self.source_window.map_or(0, VirtualWindow::start);
        for i in 0..rows {
            let idx = start + i;
            let Some(local) = idx.checked_sub(items_start) else {
                continue;
            };
            let Some(item) = self.items.get(local) else {
                break;
            };
            let y = area.y.saturating_add(i as u16);
            if y >= area.bottom() {
                break;
            }
            let selected = self.selected == Some(idx);
            if selected {
                let mut line = surface.child(Rect::new(area.x, y, row_width, 1));
                line.fill(selection_style);
            }
            let caret = if selected { '›' } else { ' ' };
            let caret_style = if selected {
                selection_style
            } else {
                ctx.theme.muted_style()
            };
            surface.set(area.x, y, caret, caret_style);
            let mut x = area.x.saturating_add(2);
            for span in &item.spans {
                if x >= row_right {
                    break;
                }
                let style = if selected {
                    item.style.patch(span.style).patch(selection_style)
                } else {
                    item.style.patch(span.style)
                };
                x = surface.set_string(x, y, span.content.as_ref(), style);
            }
        }
        if overflow && self.scrollbar && row_width < area.width {
            Scrollbar::vertical(window).render(
                Rect::new(
                    area.right() - 1,
                    area.y,
                    1,
                    area.height.min(rows.min(u16::MAX as usize) as u16),
                ),
                surface,
                ctx,
            );
        }
    }
}

impl SelectList {
    /// A list of `items` with the row from `state` highlighted.
    pub fn new(items: Vec<Line<'static>>, state: &SelectState) -> Self {
        Self {
            items,
            source_window: None,
            selected: state.selected(),
            viewport: None,
            visible_window: None,
            scrollbar: true,
            selection_style: None,
            selection_anchor: SelectionAnchor::default(),
        }
    }

    /// Build from only the items in `window` rather than the whole collection.
    ///
    /// `items` correspond to `window.range()` in order, while `window.total()`
    /// preserves absolute selection and scrollbar geometry. This keeps frame
    /// construction and rendering O(visible rows) for host-backed collections.
    pub fn windowed(items: Vec<Line<'static>>, window: VirtualWindow, state: &SelectState) -> Self {
        Self {
            items,
            source_window: Some(window),
            selected: state.selected(),
            viewport: None,
            visible_window: None,
            scrollbar: true,
            selection_style: None,
            selection_anchor: SelectionAnchor::default(),
        }
    }

    /// Cap the visible rows to `rows`, windowing a longer list around the
    /// selection so the highlighted row stays on screen.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = Some(rows.max(1));
        self
    }

    /// Paint the exact persistent `window` resolved by
    /// [`SelectViewportState::resolve`].
    ///
    /// Unlike [`windowed`](Self::windowed), `items` remains the complete
    /// collection. The window is clamped again only if layout assigns fewer
    /// rows than it contains.
    pub fn visible_window(mut self, window: VirtualWindow) -> Self {
        self.visible_window = Some(window);
        self
    }

    /// Show the overflow scrollbar (default true; only drawn when windowed).
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    /// Override the selected row's style. By default the theme's selection
    /// style is used.
    pub fn selection_style(mut self, style: Style) -> Self {
        self.selection_style = Some(style);
        self
    }

    /// Choose how the list windows itself around the selection when it resolves
    /// its own window — i.e. under [`viewport`](Self::viewport) or a smaller
    /// assigned height, not under [`visible_window`](Self::visible_window) or
    /// [`windowed`](Self::windowed).
    ///
    /// Defaults to [`SelectionAnchor::Center`].
    pub fn selection_anchor(mut self, anchor: SelectionAnchor) -> Self {
        self.selection_anchor = anchor;
        self
    }

    fn rows(&self) -> SelectRows<'_> {
        SelectRows {
            items: &self.items,
            source_window: self.source_window,
            selected: self.selected,
            viewport: self.viewport,
            visible_window: self.visible_window,
            scrollbar: self.scrollbar,
            selection_style: self.selection_style,
            selection_anchor: self.selection_anchor,
        }
    }
}

impl View for SelectList {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.rows().measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.rows().render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, InputOutcome, Key, KeyCode};
    use crate::style::Theme;
    use crate::tests::support::{buffer, rainbow_theme, row};
    use crate::text::Line;
    use crate::view::{RenderCtx, View};
    use crate::{Size, Surface};

    #[test]
    fn select_navigation_wraps_and_confirms() {
        let mut s = SelectState::new();
        let down = Event::Key(Key::new(KeyCode::Down));
        let up = Event::Key(Key::new(KeyCode::Up));
        assert_eq!(s.handle(&up, 3), InputOutcome::Changed);
        assert_eq!(s.selected(), Some(2)); // wrapped from 0 to last
        assert_eq!(s.handle(&down, 3), InputOutcome::Changed);
        assert_eq!(s.selected(), Some(0)); // wrapped back
        let enter = Event::Key(Key::new(KeyCode::Enter));
        assert_eq!(s.handle(&enter, 3), InputOutcome::Submitted);
        let esc = Event::Key(Key::new(KeyCode::Esc));
        assert_eq!(s.handle(&esc, 3), InputOutcome::Cancelled);
    }

    #[test]
    fn common_navigation_supports_vim_ctrl_tab_and_numbers() {
        let policy = SelectNavigation::common();
        let mut state = SelectState::new();
        assert_eq!(
            state.handle_with(&Event::Key(Key::new(KeyCode::Char('j'))), 4, policy),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(1));
        assert_eq!(
            state.handle_with(
                &Event::Key(Key {
                    code: KeyCode::Char('p'),
                    ctrl: true,
                    alt: false,
                    shift: false
                }),
                4,
                policy
            ),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(0));
        assert_eq!(
            state.handle_with(&Event::Key(Key::new(KeyCode::BackTab)), 4, policy),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(3));
        assert_eq!(
            state.handle_with(&Event::Key(Key::new(KeyCode::Char('2'))), 4, policy),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn default_navigation_does_not_claim_optional_aliases() {
        let mut state = SelectState::new();
        for code in [KeyCode::Char('j'), KeyCode::Tab, KeyCode::Char('1')] {
            assert_eq!(
                state.handle_with(&Event::Key(Key::new(code)), 3, SelectNavigation::default()),
                InputOutcome::Ignored
            );
        }
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn mouse_hit_testing_respects_bounds_and_scroll_offset() {
        let mut state = SelectState::new();
        let bounds = Rect::new(10, 5, 20, 3);
        let click = Event::Mouse(crate::event::Mouse::at(
            MouseKind::Down(MouseButton::Left),
            12,
            6,
        ));
        assert_eq!(
            state.handle_mouse(&click, 10, bounds, 4),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), Some(5));

        let outside = Event::Mouse(crate::event::Mouse::at(
            MouseKind::Down(MouseButton::Left),
            9,
            6,
        ));
        assert_eq!(
            state.handle_mouse(&outside, 10, bounds, 4),
            InputOutcome::Ignored
        );
    }

    #[test]
    fn multi_select_toggles_without_losing_cursor_navigation() {
        let mut state = MultiSelectState::new();
        let policy = SelectNavigation::common();
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Char(' '))), 3, policy),
            InputOutcome::Changed
        );
        assert!(state.contains(0));
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Char('j'))), 3, policy),
            InputOutcome::Changed
        );
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter)), 3, policy),
            InputOutcome::Changed
        );
        assert_eq!(state.selected().collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Char('1'))), 3, policy),
            InputOutcome::Changed
        );
        assert_eq!(state.selected().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn select_move_up_down_clamp_at_ends() {
        let mut s = SelectState::new();
        // Down steps forward, clamping at the last of `len` rows (no wrap).
        s.move_down(3);
        assert_eq!(s.selected(), Some(1));
        s.move_down(3);
        assert_eq!(s.selected(), Some(2));
        s.move_down(3);
        assert_eq!(s.selected(), Some(2)); // held at the bottom, not wrapped
        // Up steps back, clamping at the top.
        s.move_up();
        assert_eq!(s.selected(), Some(1));
        s.move_up();
        s.move_up();
        assert_eq!(s.selected(), Some(0)); // held at the top, not wrapped
        // Degenerate: an empty list stays at 0.
        s.move_down(0);
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn select_state_select_sets_index_directly() {
        // A host can drive the highlight from its own state.
        let mut s = SelectState::new();
        s.select(Some(2));
        assert_eq!(s.selected(), Some(2));
    }

    #[test]
    fn select_state_and_list_support_no_selection() {
        assert_eq!(SelectState::default().selected(), Some(0));
        let mut state = SelectState::unselected();
        assert_eq!(state.selected(), None);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter)), 2),
            InputOutcome::Ignored
        );

        let list = SelectList::new(vec![Line::from("a"), Line::from("b")], &state);
        let theme = Theme::default();
        let buf = crate::testing::render(&list, 5, 2, &theme);
        assert!((0..2).all(|y| buf[(0, y)].symbol() == " "));
        assert!((0..2).all(|y| buf[(0, y)].bg != theme.selection_bg));
    }

    #[test]
    fn select_list_accepts_an_instance_selection_style() {
        let state = SelectState::new();
        let style = Style::default().fg(crate::style::Color::Blue);
        let list = SelectList::new(vec![Line::from("a")], &state).selection_style(style);
        let buf = crate::testing::render(&list, 5, 1, &Theme::default());
        assert_eq!(buf[(0, 0)].fg, crate::style::Color::Blue);
        assert!(!buf[(0, 0)].modifier.contains(crate::style::Modifier::BOLD));
    }

    #[test]
    fn select_highlights_current_row() {
        let items = vec![Line::from("alpha"), Line::from("beta")];
        let mut state = SelectState::new();
        let _ = state.handle(&Event::Key(Key::new(KeyCode::Down)), 2); // select beta
        let list = SelectList::new(items, &state);
        let mut buf = buffer(10, 2);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        list.render(area, &mut surface, &ctx);
        assert!(row(&buf, 1).contains("beta"));
        // Selected row carries the selection background.
        assert_eq!(buf[(0, 1)].bg, theme.selection_bg);
        assert_eq!(buf[(0, 0)].bg, crate::style::Color::Reset);
    }

    #[test]
    fn select_viewport_windows_a_long_list_and_keeps_selection_visible() {
        // 20 items, viewport of 4: the selection must always be on screen.
        let items: Vec<Line> = (0..20).map(|i| Line::from(format!("item{i}"))).collect();
        let mut state = SelectState::new();
        state.select(Some(12));
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let list = SelectList::new(items.clone(), &state).viewport(4);
        // Windowed height is the viewport, not the full list.
        assert_eq!(list.measure(Size::new(20, 40), &ctx).height, 4);
        let rendered = crate::testing::render(&list, 20, 4, &theme);
        let text = crate::testing::grid(&rendered);
        assert!(
            text.contains("item12"),
            "selection should be visible:\n{text}"
        );
        assert!(
            !text.contains("item0\n") && !text.contains("item19"),
            "far items windowed out"
        );
        // A scrollbar occupies the last column on at least one row.
        let has_scrollbar = (0..4).any(|y| matches!(rendered[(19, y)].symbol(), "█" | "│"));
        assert!(
            has_scrollbar,
            "overflowing list should draw a scrollbar:\n{text}"
        );
    }

    #[test]
    fn select_list_edge_anchor_trails_the_selection_instead_of_centering() {
        let items: Vec<Line> = (0..20).map(|i| Line::from(format!("item{i}"))).collect();
        let mut state = SelectState::new();
        state.select(Some(7));
        let list = |anchor: SelectionAnchor| {
            SelectList::new(items.clone(), &state)
                .viewport(5)
                .selection_anchor(anchor)
                .rows()
                .window(Some(5))
                .range()
        };
        // Edge trails the selection by one row; Center recenters on it.
        assert_eq!(list(SelectionAnchor::Edge), 3..8);
        assert_eq!(list(SelectionAnchor::Center), 5..10);

        // The rendered rows follow the policy, not just the window token.
        let edge = SelectList::new(items, &state)
            .viewport(5)
            .selection_anchor(SelectionAnchor::Edge);
        let text = crate::testing::grid(&crate::testing::render(&edge, 12, 5, &Theme::default()));
        assert!(text.contains("item3"), "top of the edge window:\n{text}");
        assert!(text.contains("item7"), "selection stays visible:\n{text}");
        assert!(!text.contains("item9"), "below the edge window:\n{text}");
    }

    #[test]
    fn render_height_smaller_than_viewport_keeps_selection_visible() {
        let items: Vec<Line> = (0..20).map(|i| Line::from(format!("item{i}"))).collect();
        let mut state = SelectState::new();
        state.select(Some(15));
        let list = SelectList::new(items, &state).viewport(10);
        let text = crate::testing::grid(&crate::testing::render(&list, 12, 3, &Theme::default()));

        assert!(
            text.contains("item15"),
            "selection should be visible:\n{text}"
        );
    }

    #[test]
    fn select_viewport_shows_whole_list_when_it_fits() {
        let items: Vec<Line> = (0..3).map(|i| Line::from(format!("item{i}"))).collect();
        let state = SelectState::new();
        let list = SelectList::new(items, &state).viewport(8);
        let theme = Theme::default();
        // Fits within the viewport → no windowing, height is the item count.
        assert_eq!(
            list.measure(Size::new(20, 40), &RenderCtx::new(&theme))
                .height,
            3
        );
        let text = crate::testing::grid(&crate::testing::render(&list, 20, 3, &theme));
        assert!(text.contains("item0") && text.contains("item2"));
    }

    #[test]
    fn persistent_viewport_bottom_click_uses_rendered_window_without_recentering() {
        use crate::event::{Mouse, MouseButton, MouseKind};

        let mut state = SelectViewportState::new();
        state.select(Some(10));
        let window = state.resolve(20, 4);
        assert_eq!(window.range(), 7..11);

        let click = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 2, 3));
        assert_eq!(
            state.handle_mouse(&click, 20, Rect::new(0, 0, 12, 4), window),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), Some(10));
        assert_eq!(state.resolve(20, 4).range(), 7..11);
    }

    #[test]
    fn persistent_viewport_moves_only_across_each_keyboard_edge() {
        let mut state = SelectViewportState::new();
        let down = Event::Key(Key::new(KeyCode::Down));
        for _ in 0..3 {
            assert_eq!(state.handle(&down, 10, 4), InputOutcome::Changed);
        }
        assert_eq!(state.resolve(10, 4).range(), 0..4);
        assert_eq!(state.handle(&down, 10, 4), InputOutcome::Changed);
        assert_eq!(state.resolve(10, 4).range(), 1..5);

        let up = Event::Key(Key::new(KeyCode::Up));
        assert_eq!(state.handle(&up, 10, 4), InputOutcome::Changed);
        assert_eq!(state.resolve(10, 4).range(), 1..5);
        for _ in 0..3 {
            let _ = state.handle(&up, 10, 4);
        }
        assert_eq!(state.resolve(10, 4).range(), 0..4);
    }

    #[test]
    fn persistent_viewport_reconciles_resize_and_collection_shrink() {
        let mut state = SelectViewportState::new();
        state.select(Some(13));
        assert_eq!(state.resolve(20, 4).range(), 10..14);
        assert_eq!(state.resolve(20, 6).range(), 10..16);

        let shrunk = state.resolve(8, 6);
        assert_eq!(state.selected(), Some(7));
        assert_eq!(shrunk.range(), 2..8);
    }

    #[test]
    fn persistent_window_drives_rows_and_scrollbar_geometry() {
        let items = (0..10)
            .map(|index| Line::from(format!("row{index}")))
            .collect();
        let mut state = SelectViewportState::new();
        state.set_offset(3);
        state.select(Some(4));
        let window = state.resolve(10, 4);
        let list = SelectList::new(items, state.selection()).visible_window(window);
        let rendered = crate::testing::render(&list, 8, 4, &Theme::default());
        let text = crate::testing::grid(&rendered);
        assert!(text.contains("row3") && text.contains("row6"), "{text}");
        assert!(!text.contains("row2") && !text.contains("row7"), "{text}");
        assert_eq!(rendered[(7, 0)].symbol(), "│");
        assert_eq!(rendered[(7, 1)].symbol(), "█");
    }

    #[test]
    fn select_list_selection_uses_theme_slots() {
        let t = rainbow_theme();
        let mut state = SelectState::new();
        let _ = state.handle(&Event::Key(Key::new(KeyCode::Down)), 2); // select row 1
        let list = SelectList::new(vec![Line::from("a"), Line::from("b")], &state);
        let mut buf = buffer(10, 2);
        let area = buf.area;
        let ctx = RenderCtx::new(&t);
        let mut surface = Surface::new(&mut buf, area);
        list.render(area, &mut surface, &ctx);
        assert_eq!(buf[(0, 1)].bg, t.selection_bg, "selected row bg");
        assert_eq!(buf[(0, 1)].fg, t.selection_fg, "selected caret fg");
        assert_ne!(
            buf[(0, 0)].bg,
            t.selection_bg,
            "unselected row not highlighted"
        );
    }
}
