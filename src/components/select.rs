//! Selectable list (Pi's `SelectList`).
//!
//! [`SelectState`] persists an optional highlighted index and handles
//! up/down/wrap navigation; [`SelectList`] renders the options, marking the
//! current one with a theme-default or instance selection style and a caret.
//! Enter is surfaced as [`InputOutcome::Submitted`] so the caller decides what
//! "confirm" means.

use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use ratatui_core::text::Line;

use crate::event::{Event, InputOutcome, KeyCode, MouseButton, MouseKind};
use crate::geometry::Size;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

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
/// use ratatui_core::text::Line;
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
    selected: Option<usize>,
    /// Max visible rows; `None` shows the whole list.
    viewport: Option<u16>,
    scrollbar: bool,
    selection_style: Option<Style>,
}

impl SelectList {
    /// A list of `items` with the row from `state` highlighted.
    pub fn new(items: Vec<Line<'static>>, state: &SelectState) -> Self {
        Self {
            items,
            selected: state.selected(),
            viewport: None,
            scrollbar: true,
            selection_style: None,
        }
    }

    /// Cap the visible rows to `rows`, windowing a longer list around the
    /// selection so the highlighted row stays on screen.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = Some(rows.max(1));
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

    /// The `(start, visible_rows)` window: the whole list unless a `viewport`
    /// smaller than the list is set, in which case a slice centered on the
    /// selection and clamped to the ends.
    fn window(&self) -> (usize, usize) {
        let total = self.items.len();
        match self.viewport {
            Some(v) if total > v as usize => {
                let v = (v as usize).max(1);
                let start = self
                    .selected
                    .unwrap_or(0)
                    .saturating_sub(v / 2)
                    .min(total - v);
                (start, v)
            }
            _ => (0, total),
        }
    }
}

impl View for SelectList {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let width = self
            .items
            .iter()
            .map(super::text::line_width)
            .max()
            .unwrap_or(0)
            .saturating_add(2); // caret + space
        let (_, rows) = self.window();
        Size::new(width.min(available.width), rows as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let (start, rows) = self.window();
        let overflow = self.items.len() > rows;
        // Reserve the last column for the scrollbar when the list overflows.
        let row_width = if overflow && self.scrollbar {
            area.width.saturating_sub(1)
        } else {
            area.width
        };
        let row_right = area.x.saturating_add(row_width);
        let selection_style = self
            .selection_style
            .unwrap_or_else(|| ctx.theme.selection_style());
        for i in 0..rows {
            let idx = start + i;
            let Some(item) = self.items.get(idx) else {
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
            self.draw_scrollbar(area, start, rows, surface, ctx);
        }
    }
}

impl SelectList {
    /// A right-edge scrollbar whose thumb tracks the window position, mirroring
    /// [`Scroll`](super::Scroll)'s scrollbar.
    fn draw_scrollbar(
        &self,
        area: Rect,
        start: usize,
        rows: usize,
        surface: &mut Surface,
        ctx: &RenderCtx,
    ) {
        let total = self.items.len();
        let track_x = area.right() - 1;
        let track_h = rows as u16;
        let max_start = total.saturating_sub(rows).max(1) as u32;
        let thumb_h = (((rows * rows) / total).max(1) as u16).min(track_h);
        let travel = track_h.saturating_sub(thumb_h);
        let thumb_y = area.y + ((start as u32 * travel as u32) / max_start) as u16;
        let track_style = Style::default().fg(ctx.theme.dim);
        let thumb_style = Style::default().fg(ctx.theme.muted);
        for row in 0..track_h {
            let y = area.y + row;
            let within = y >= thumb_y && y < thumb_y.saturating_add(thumb_h);
            let (glyph, style) = if within {
                ('█', thumb_style)
            } else {
                ('│', track_style)
            };
            surface.set(track_x, y, glyph, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, InputOutcome, Key, KeyCode};
    use crate::style::Theme;
    use crate::tests::support::{buffer, rainbow_theme, row};
    use crate::view::{RenderCtx, View};
    use crate::{Size, Surface};
    use ratatui_core::text::Line;

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
        let style = Style::default().fg(ratatui_core::style::Color::Blue);
        let list = SelectList::new(vec![Line::from("a")], &state).selection_style(style);
        let buf = crate::testing::render(&list, 5, 1, &Theme::default());
        assert_eq!(buf[(0, 0)].fg, ratatui_core::style::Color::Blue);
        assert!(
            !buf[(0, 0)]
                .modifier
                .contains(ratatui_core::style::Modifier::BOLD)
        );
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
        assert_eq!(buf[(0, 0)].bg, ratatui_core::style::Color::Reset);
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
