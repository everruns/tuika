//! Columned, selectable table — the multi-column peer of [`SelectList`].
//!
//! The defining widget of repo/branch/worktree browsers, process and container
//! lists, and file explorers: a header row, per-column width policies, a
//! full-row selection highlight, a caret gutter, and windowed scrolling. Column
//! widths are resolved by the same flexbox [`solve`](crate::layout::solve) the rest of
//! the toolkit uses — a column is [`Column::fixed`], [`Column::auto`] (sizes to
//! its widest cell), or [`Column::flex`] (shares leftover width by weight) — so
//! a table lays out consistently with every other container.
//!
//! Selection and navigation are the same [`SelectState`] a [`SelectList`] uses
//! (`handle` for arrows/Enter/Esc, `select` to drive it from a host model), so a
//! single-column list and a table share one state type.
//!
//! Chrome follows the theme by default but is overridable, the same
//! theme-by-default / explicit-override pattern as [`Boxed::border_color`]:
//! [`Table::caret`] sets the gutter marker glyph, [`Table::header_style`]
//! restyles the header row, [`Table::selection_style`] customizes the active
//! row, and [`Table::preserve_selection_fg`] keeps each column's own color
//! under the selection highlight.
//!
//! [`SelectList`]: crate::components::SelectList
//! [`Boxed::border_color`]: crate::components::Boxed::border_color

use crate::geometry::Rect;
use crate::style::Style;
use crate::text::Line;

use crate::geometry::Size;
use crate::layout::{Dimension, Item, LayoutStyle, solve};
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

use super::select::SelectState;
use super::text::line_width;
use super::{Scrollbar, SelectionAnchor, VirtualWindow};

/// One table column: a header cell and a main-axis width policy.
#[derive(Clone)]
pub struct Column {
    header: Line<'static>,
    width: Dimension,
}

impl Column {
    /// A column with an explicit [`Dimension`] width policy.
    pub fn new(header: impl Into<Line<'static>>, width: Dimension) -> Self {
        Self {
            header: header.into(),
            width,
        }
    }

    /// A column that sizes to its widest cell (header included).
    pub fn auto(header: impl Into<Line<'static>>) -> Self {
        Self::new(header, Dimension::Auto)
    }

    /// A column of exactly `cells` columns wide.
    pub fn fixed(header: impl Into<Line<'static>>, cells: u16) -> Self {
        Self::new(header, Dimension::Fixed(cells))
    }

    /// A column that grows to share leftover width, weighted by `weight`.
    pub fn flex(header: impl Into<Line<'static>>, weight: u16) -> Self {
        Self::new(header, Dimension::Flex(weight))
    }
}

/// A multi-column selectable list.
///
/// Pairs with a host-held [`SelectState`] (the row selection) exactly as
/// [`SelectList`](crate::components::SelectList) does. Each row is a `Vec` of cell
/// [`Line`]s, one per column; a short row is padded with blanks, extra cells are
/// ignored.
///
/// # Example
///
/// ```
/// use tuika::ui::Line;
/// use tuika::prelude::*;
/// use tuika::testing::{grid, render};
///
/// let cols = vec![Column::auto("name"), Column::auto("kind")];
/// let rows = vec![
///     vec![Line::from("main"), Line::from("branch")],
///     vec![Line::from("wip"), Line::from("branch")],
/// ];
/// let state = SelectState::new(); // first row selected
/// let table = Table::new(cols, rows, &state);
///
/// let g = grid(&render(&table, 20, 3, &Theme::default()));
/// let lines: Vec<&str> = g.lines().collect();
/// assert!(lines[0].contains("name") && lines[0].contains("kind")); // header row
/// assert!(lines[1].starts_with("› main"));                         // caret on selection
/// assert!(lines[1].contains("branch") && lines[2].contains("wip"));
/// ```
pub struct Table {
    columns: Vec<Column>,
    rows: Vec<Vec<Line<'static>>>,
    /// Host-provided source window, or `None` when `rows` is the whole table.
    source_window: Option<VirtualWindow>,
    selected: Option<usize>,
    viewport: Option<u16>,
    visible_window: Option<VirtualWindow>,
    scrollbar: bool,
    gutter: bool,
    show_header: bool,
    gap: u16,
    caret: char,
    header_style: Option<Style>,
    preserve_selection_fg: bool,
    selection_style: Option<Style>,
    selection_anchor: SelectionAnchor,
}

impl Table {
    /// A table of `columns` and `rows`, with the row from `state` selected.
    pub fn new(columns: Vec<Column>, rows: Vec<Vec<Line<'static>>>, state: &SelectState) -> Self {
        Self {
            columns,
            rows,
            source_window: None,
            selected: state.selected(),
            viewport: None,
            visible_window: None,
            scrollbar: true,
            gutter: true,
            show_header: true,
            gap: 2,
            caret: '›',
            header_style: None,
            preserve_selection_fg: false,
            selection_style: None,
            selection_anchor: SelectionAnchor::default(),
        }
    }

    /// Build from only the rows in `window` rather than the whole collection.
    ///
    /// `rows` correspond to `window.range()` in order, while `window.total()`
    /// preserves absolute selection and scrollbar geometry. Auto columns size
    /// from the supplied rows; use fixed or flex columns when widths must stay
    /// stable as a host exchanges windows.
    pub fn windowed(
        columns: Vec<Column>,
        rows: Vec<Vec<Line<'static>>>,
        window: VirtualWindow,
        state: &SelectState,
    ) -> Self {
        Self {
            columns,
            rows,
            source_window: Some(window),
            selected: state.selected(),
            viewport: None,
            visible_window: None,
            scrollbar: true,
            gutter: true,
            show_header: true,
            gap: 2,
            caret: '›',
            header_style: None,
            preserve_selection_fg: false,
            selection_style: None,
            selection_anchor: SelectionAnchor::default(),
        }
    }

    /// Cap the visible data rows to `rows`. By default the table fills its
    /// assigned body height; this adds a smaller upper bound when desired.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = Some(rows.max(1));
        self
    }

    /// Paint the exact persistent data-row window resolved by
    /// [`SelectViewportState::resolve`](super::SelectViewportState::resolve).
    ///
    /// `rows` remains the complete table. This is the persistent alternative
    /// to the legacy selection-centered [`viewport`](Self::viewport) behavior.
    pub fn visible_window(mut self, window: VirtualWindow) -> Self {
        self.visible_window = Some(window);
        self
    }

    /// Show the overflow scrollbar (default true; only drawn when windowed).
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    /// Show the caret gutter that marks the selected row (default true).
    pub fn gutter(mut self, show: bool) -> Self {
        self.gutter = show;
        self
    }

    /// Show the header row (default true).
    pub fn header(mut self, show: bool) -> Self {
        self.show_header = show;
        self
    }

    /// Columns of blank between adjacent columns (default 2).
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// The glyph marking the selected row in the gutter (default `›`). Let a
    /// host keep an existing marker, e.g. `▶`.
    pub fn caret(mut self, caret: char) -> Self {
        self.caret = caret;
        self
    }

    /// Style the header row explicitly, overriding the default
    /// (`theme.accent_style()`) — the same theme-by-default, explicit-override
    /// pattern as [`Boxed::border_color`](crate::components::Boxed::border_color).
    pub fn header_style(mut self, style: Style) -> Self {
        self.header_style = Some(style);
        self
    }

    /// Keep each cell's own foreground on the selected row, applying only the
    /// selection background (default off — the selected row is recolored to a
    /// uniform `selection_style`). Turn this on for a table whose columns are
    /// color-coded so those colors survive under the highlight.
    pub fn preserve_selection_fg(mut self, preserve: bool) -> Self {
        self.preserve_selection_fg = preserve;
        self
    }

    /// Override the selected row's style. By default the theme's selection
    /// style is used.
    pub fn selection_style(mut self, style: Style) -> Self {
        self.selection_style = Some(style);
        self
    }

    /// Choose how the table windows itself around the selection when it
    /// resolves its own window — i.e. under [`viewport`](Self::viewport) or a
    /// body shorter than the row count, not under
    /// [`visible_window`](Self::visible_window) or [`windowed`](Self::windowed).
    ///
    /// Defaults to [`SelectionAnchor::Center`].
    pub fn selection_anchor(mut self, anchor: SelectionAnchor) -> Self {
        self.selection_anchor = anchor;
        self
    }

    /// Cells the caret gutter occupies (caret + space), or 0 when hidden.
    fn gutter_width(&self) -> u16 {
        if self.gutter { 2 } else { 0 }
    }

    /// Rows the header occupies (0 or 1).
    fn header_rows(&self) -> u16 {
        u16::from(self.show_header)
    }

    /// The widest content in column `c` — its header and every cell — used as
    /// the intrinsic size the solver gives an [`Dimension::Auto`] column.
    fn column_intrinsic(&self, c: usize) -> u16 {
        let header_w = if self.show_header {
            line_width(&self.columns[c].header)
        } else {
            0
        };
        let cells_w = self
            .rows
            .iter()
            .filter_map(|row| row.get(c))
            .map(line_width)
            .max()
            .unwrap_or(0);
        header_w.max(cells_w)
    }

    /// Resolve each column's rect inside `cols_area` via the flexbox solver.
    fn solve_columns(&self, cols_area: Rect) -> Vec<Rect> {
        let items: Vec<Item> = self
            .columns
            .iter()
            .enumerate()
            .map(|(c, col)| Item::new(col.width, Size::new(self.column_intrinsic(c), 1)))
            .collect();
        solve(cols_area, &LayoutStyle::row().gap(self.gap), &items)
    }

    /// The visible data-row window: the whole table unless a
    /// [`viewport`](Self::viewport) smaller than the row count is set, in which
    /// case a slice placed around the selection by
    /// [`selection_anchor`](Self::selection_anchor) and clamped to the ends.
    fn window(&self, available_rows: u16) -> VirtualWindow {
        let viewport = self
            .viewport
            .map_or(available_rows, |rows| rows.min(available_rows));
        if let Some(window) = self.visible_window {
            return VirtualWindow::new(
                window.total(),
                window.len().min(usize::from(viewport)),
                window.start(),
            );
        }
        match self.source_window {
            Some(source) => VirtualWindow::new(
                source.total(),
                source.len().min(usize::from(viewport)),
                source.start(),
            ),
            None => {
                self.selection_anchor
                    .window(self.rows.len(), usize::from(viewport), self.selected)
            }
        }
    }

    /// Draw one row's cells into their column rects at row `y`, patching each
    /// cell's style with `row_style` (the selection style on the selected row).
    fn draw_cells(
        &self,
        row: &[Line<'static>],
        col_rects: &[Rect],
        y: u16,
        row_style: Option<Style>,
        surface: &mut Surface,
    ) {
        for (c, rect) in col_rects.iter().enumerate() {
            let Some(cell) = row.get(c) else {
                continue;
            };
            let mut x = rect.x;
            let right = rect.x.saturating_add(rect.width);
            for span in &cell.spans {
                if x >= right {
                    break;
                }
                let style = cell.style.patch(span.style);
                let style = match row_style {
                    Some(sel) => style.patch(sel),
                    None => style,
                };
                x = surface.set_string(x, y, span.content.as_ref(), style);
            }
        }
    }
}

impl View for Table {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let available_rows = available.height.saturating_sub(self.header_rows());
        let rows = self.window(available_rows).len();
        let cols_w: u16 = (0..self.columns.len())
            .map(|c| self.column_intrinsic(c))
            .fold(0, u16::saturating_add);
        let gaps = self
            .gap
            .saturating_mul(self.columns.len().saturating_sub(1) as u16);
        let width = self
            .gutter_width()
            .saturating_add(cols_w)
            .saturating_add(gaps);
        let height = self.header_rows().saturating_add(rows as u16);
        Size::new(width.min(available.width), height)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.width == 0 || area.height == 0 || self.columns.is_empty() {
            return;
        }
        let body_rows = area.height.saturating_sub(self.header_rows());
        let window = self.window(body_rows);
        let start = window.start();
        let win_rows = window.len();
        let overflow = window.overflows();
        let gutter_w = self.gutter_width();
        let scrollbar_w = u16::from(overflow && self.scrollbar);

        // The band the columns lay out in: full width minus the gutter and the
        // reserved scrollbar column.
        let cols_area = Rect::new(
            area.x.saturating_add(gutter_w),
            area.y,
            area.width
                .saturating_sub(gutter_w)
                .saturating_sub(scrollbar_w),
            area.height,
        );
        let col_rects = self.solve_columns(cols_area);

        // Header row — explicit `header_style` wins over the theme default.
        if self.show_header {
            let headers: Vec<Line<'static>> =
                self.columns.iter().map(|c| c.header.clone()).collect();
            let header_style = self
                .header_style
                .unwrap_or_else(|| ctx.theme.accent_style());
            self.draw_cells(&headers, &col_rects, area.y, Some(header_style), surface);
        }

        // Data rows, windowed around the selection.
        let body_top = area.y.saturating_add(self.header_rows());
        let row_span_w = area.width.saturating_sub(scrollbar_w);
        let rows_start = self.source_window.map_or(0, VirtualWindow::start);
        for i in 0..win_rows {
            let idx = start + i;
            let Some(local) = idx.checked_sub(rows_start) else {
                continue;
            };
            let Some(row) = self.rows.get(local) else {
                break;
            };
            let y = body_top.saturating_add(i as u16);
            if y >= area.bottom() {
                break;
            }
            let selected = self.selected == Some(idx);
            let row_style = if selected {
                let sel = self
                    .selection_style
                    .unwrap_or_else(|| ctx.theme.selection_style());
                // Highlight spans the whole row: gutter, columns, and gaps.
                let mut band = surface.child(Rect::new(area.x, y, row_span_w, 1));
                band.fill(sel);
                // Cells get the full selection style (uniform fg) by default, or
                // just its background when preserving each column's own color.
                Some(if self.preserve_selection_fg {
                    let mut preserve_fg = sel;
                    preserve_fg.fg = None;
                    preserve_fg
                } else {
                    sel
                })
            } else {
                None
            };
            if self.gutter {
                let caret = if selected { self.caret } else { ' ' };
                let caret_style = if selected {
                    self.selection_style
                        .unwrap_or_else(|| ctx.theme.selection_style())
                } else {
                    ctx.theme.muted_style()
                };
                surface.set(area.x, y, caret, caret_style);
            }
            self.draw_cells(row, &col_rects, y, row_style, surface);
        }

        if scrollbar_w == 1 {
            Scrollbar::vertical(window).render(
                Rect::new(
                    area.right() - 1,
                    area.y.saturating_add(self.header_rows()),
                    1,
                    body_rows.min(win_rows.min(u16::MAX as usize) as u16),
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
    use crate::event::{Event, Key, KeyCode};
    use crate::style::Theme;
    use crate::tests::support::{buffer, rainbow_theme, row};
    use crate::text::{Line, Span};
    use crate::view::{RenderCtx, View};
    use crate::{Size, Surface};

    fn sample() -> (Vec<Column>, Vec<Vec<Line<'static>>>) {
        let cols = vec![Column::auto("name"), Column::auto("kind")];
        let rows = vec![
            vec![Line::from("main"), Line::from("branch")],
            vec![Line::from("wip"), Line::from("branch")],
            vec![Line::from("hotfix"), Line::from("branch")],
        ];
        (cols, rows)
    }

    #[test]
    fn table_renders_header_and_marks_selection() {
        let (cols, rows) = sample();
        let mut state = SelectState::new();
        state.select(Some(1));
        let table = Table::new(cols, rows, &state);
        let mut buf = buffer(20, 4);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        table.render(area, &mut surface, &ctx);
        // Header on row 0, data below.
        assert!(row(&buf, 0).contains("name") && row(&buf, 0).contains("kind"));
        assert!(row(&buf, 1).contains("main"));
        assert!(row(&buf, 2).contains("wip"));
        // Caret on the selected (second) data row, at column 0.
        assert_eq!(buf[(0, 2)].symbol(), "›");
        assert_ne!(buf[(0, 1)].symbol(), "›", "unselected row has no caret");
    }

    #[test]
    fn table_selection_highlights_full_row() {
        let (cols, rows) = sample();
        let t = rainbow_theme();
        let mut state = SelectState::new();
        state.select(Some(0));
        let table = Table::new(cols, rows, &state);
        let mut buf = buffer(20, 4);
        let area = buf.area;
        let ctx = RenderCtx::new(&t);
        let mut surface = Surface::new(&mut buf, area);
        table.render(area, &mut surface, &ctx);
        // The whole selected row (row 1) carries the selection background, gutter
        // through the last content column, not just the cell text.
        let y = 1;
        for x in [0u16, 2, 8, 15] {
            assert_eq!(buf[(x, y)].bg, t.selection_bg, "row bg at col {x}");
        }
        // A non-selected data row is not highlighted.
        assert_ne!(buf[(0, 2)].bg, t.selection_bg);
    }

    #[test]
    fn table_columns_use_the_flex_solver() {
        // A fixed first column and a flex second: the flex column grows to fill
        // the leftover width. Verify via the resolved column rects.
        let cols = vec![Column::fixed("id", 4), Column::flex("desc", 1)];
        let rows = vec![vec![Line::from("1"), Line::from("x")]];
        let state = SelectState::new();
        let table = Table::new(cols, rows, &state).gutter(false).gap(1);
        // cols_area spans the full width (no gutter, no overflow scrollbar).
        let rects = table.solve_columns(Rect::new(0, 0, 20, 1));
        assert_eq!(rects[0].width, 4, "fixed column keeps its width");
        // 20 - 4 (fixed) - 1 (gap) = 15 for the flex column.
        assert_eq!(rects[1].width, 15, "flex column takes the leftover");
        assert_eq!(rects[1].x, 5, "second column starts after col1 + gap");
    }

    /// The acceptance table for [`SelectionAnchor`]: 20 rows in a 5-row pane.
    #[test]
    fn table_selection_anchor_picks_the_windowing_policy() {
        let table = |selected: usize, anchor: SelectionAnchor| {
            let rows: Vec<Vec<Line>> = (0..20)
                .map(|i| vec![Line::from(format!("row{i}"))])
                .collect();
            let mut state = SelectState::new();
            state.select(Some(selected));
            Table::new(vec![Column::auto("n")], rows, &state)
                .viewport(5)
                .selection_anchor(anchor)
                .window(5)
                .start()
        };

        // Near the top neither policy has anything to scroll.
        assert_eq!(table(2, SelectionAnchor::Edge), 0);
        assert_eq!(table(2, SelectionAnchor::Center), 0);
        // Mid-collection is where the two differ.
        assert_eq!(table(7, SelectionAnchor::Edge), 3);
        assert_eq!(table(7, SelectionAnchor::Center), 5);
        // Both clamp to the last full window at the end.
        assert_eq!(table(19, SelectionAnchor::Edge), 15);
        assert_eq!(table(19, SelectionAnchor::Center), 15);

        // Center stays the default.
        let rows: Vec<Vec<Line>> = (0..20)
            .map(|i| vec![Line::from(format!("row{i}"))])
            .collect();
        let mut state = SelectState::new();
        state.select(Some(7));
        assert_eq!(
            Table::new(vec![Column::auto("n")], rows, &state)
                .viewport(5)
                .window(5)
                .start(),
            5
        );
    }

    /// The stateless `Edge` window re-derives itself every frame; the
    /// persistent one remembers. Moving 7 -> 6 is where that shows.
    #[test]
    fn table_edge_anchor_is_stateless_where_the_viewport_state_is_not() {
        let rows: Vec<Vec<Line>> = (0..20)
            .map(|i| vec![Line::from(format!("row{i}"))])
            .collect();
        let mut state = SelectState::new();
        state.select(Some(6));
        let stateless = Table::new(vec![Column::auto("n")], rows.clone(), &state)
            .viewport(5)
            .selection_anchor(SelectionAnchor::Edge)
            .window(5);
        assert_eq!(stateless.start(), 2, "trails the selection from the top");

        let mut viewport = super::super::SelectViewportState::new();
        viewport.select(Some(7));
        assert_eq!(viewport.resolve(20, 5).start(), 3);
        viewport.select(Some(6));
        let persistent = viewport.resolve(20, 5);
        assert_eq!(persistent.start(), 3, "row 6 is already visible");
        let table = Table::new(vec![Column::auto("n")], rows, viewport.selection())
            .visible_window(persistent);
        assert_eq!(table.window(5), persistent, "explicit window wins");
    }

    #[test]
    fn table_windows_long_body_and_keeps_selection_visible() {
        let cols = vec![Column::auto("n")];
        let rows: Vec<Vec<Line>> = (0..20)
            .map(|i| vec![Line::from(format!("row{i}"))])
            .collect();
        let mut state = SelectState::new();
        state.select(Some(15));
        let table = Table::new(cols, rows, &state).viewport(4);
        let theme = Theme::default();
        // Height = header (1) + viewport (4).
        assert_eq!(
            table
                .measure(Size::new(30, 40), &RenderCtx::new(&theme))
                .height,
            5
        );
        let text = crate::testing::grid(&crate::testing::render(&table, 30, 5, &theme));
        assert!(text.contains("row15"), "selection visible:\n{text}");
        assert!(!text.contains("row0\n"), "far rows windowed out:\n{text}");
        // Scrollbar occupies the last column on a body row.
        let buf = crate::testing::render(&table, 30, 5, &theme);
        let has_bar = (1..5).any(|y| matches!(buf[(29, y)].symbol(), "█" | "│"));
        assert!(has_bar, "overflowing table draws a scrollbar:\n{text}");
    }

    #[test]
    fn table_defaults_to_the_assigned_body_height() {
        let cols = vec![Column::auto("n")];
        let rows = (0..20)
            .map(|i| vec![Line::from(format!("row{i}"))])
            .collect();
        let mut state = SelectState::new();
        state.select(Some(15));
        let table = Table::new(cols, rows, &state);
        let buf = crate::testing::render(&table, 20, 5, &Theme::default());
        let text = crate::testing::grid(&buf);
        assert!(text.contains("row15"), "selection visible:\n{text}");
        assert!((1..5).any(|y| matches!(buf[(19, y)].symbol(), "█" | "│")));
    }

    #[test]
    fn table_navigates_with_select_state() {
        // The table shares SelectState with SelectList — arrow keys move the row.
        let (_, rows) = sample();
        let mut state = SelectState::new();
        let _ = state.handle(&Event::Key(Key::new(KeyCode::Down)), rows.len());
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn table_pads_short_rows() {
        // A row with fewer cells than columns simply leaves the missing cells
        // blank rather than panicking.
        let cols = vec![Column::auto("a"), Column::auto("b")];
        let rows = vec![vec![Line::from("only")]]; // missing second cell
        let state = SelectState::new();
        let table = Table::new(cols, rows, &state);
        let theme = Theme::default();
        let text = crate::testing::grid(&crate::testing::render(&table, 20, 2, &theme));
        assert!(text.contains("only"));
    }

    #[test]
    fn table_custom_caret_marks_selection() {
        let (cols, rows) = sample();
        let mut state = SelectState::new();
        state.select(Some(0));
        let table = Table::new(cols, rows, &state).caret('▶');
        let mut buf = buffer(20, 3);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        table.render(area, &mut surface, &ctx);
        assert_eq!(
            buf[(0, 1)].symbol(),
            "▶",
            "custom caret on the selected row"
        );
    }

    #[test]
    fn table_header_style_overrides_the_theme_default() {
        use crate::style::{Color, Style};
        let (cols, rows) = sample();
        let t = rainbow_theme();
        let state = SelectState::new();
        let table =
            Table::new(cols, rows, &state).header_style(Style::default().fg(Color::Indexed(200)));
        let mut buf = buffer(20, 3);
        let area = buf.area;
        let ctx = RenderCtx::new(&t);
        let mut surface = Surface::new(&mut buf, area);
        table.render(area, &mut surface, &ctx);
        // The header cell text uses the explicit color, not theme.accent.
        assert_eq!(buf[(2, 0)].fg, Color::Indexed(200), "header 'name' fg");
        assert_ne!(
            Color::Indexed(200),
            t.accent,
            "and it differs from the default"
        );
    }

    #[test]
    fn table_preserve_selection_fg_keeps_cell_colors() {
        use crate::style::{Color, Style};
        let t = rainbow_theme();
        let cols = vec![Column::auto("c")];
        // A color-coded cell on the (only, selected) data row.
        let rows = vec![vec![Line::from(Span::styled(
            "ok",
            Style::default().fg(Color::Indexed(46)),
        ))]];
        let state = SelectState::new(); // row 0 selected

        let render = |preserve: bool| {
            let table = Table::new(cols.clone(), rows.clone(), &state)
                .preserve_selection_fg(preserve)
                .gutter(false);
            let mut buf = buffer(6, 2);
            let area = buf.area;
            let ctx = RenderCtx::new(&t);
            let mut surface = Surface::new(&mut buf, area);
            table.render(area, &mut surface, &ctx);
            let cell = &buf[(0, 1)]; // first cell of the selected data row
            (cell.fg, cell.bg)
        };

        // Preserve on: the cell keeps its own fg but gains the selection bg.
        let (fg, bg) = render(true);
        assert_eq!(
            fg,
            Color::Indexed(46),
            "column color survives the highlight"
        );
        assert_eq!(bg, t.selection_bg, "selection background still applied");
        // Default (off): the selected row is recolored to the uniform selection fg.
        let (fg_off, _) = render(false);
        assert_eq!(fg_off, t.selection_fg, "default overwrites cell fg");
    }

    #[test]
    fn table_draws_no_selection_band_when_state_is_none() {
        let (cols, rows) = sample();
        let mut state = SelectState::new();
        state.select(None);
        let t = rainbow_theme();
        let table = Table::new(cols, rows, &state);
        let buf = crate::testing::render(&table, 20, 4, &t);
        assert!((1..4).all(|y| buf[(0, y)].symbol() == " "));
        assert!((1..4).all(|y| buf[(0, y)].bg != t.selection_bg));
    }

    #[test]
    fn table_accepts_an_instance_selection_style() {
        use crate::style::{Color, Modifier};
        let (cols, rows) = sample();
        let state = SelectState::new();
        let table =
            Table::new(cols, rows, &state).selection_style(Style::default().fg(Color::Blue));
        let buf = crate::testing::render(&table, 20, 4, &Theme::default());
        assert_eq!(buf[(0, 1)].fg, Color::Blue);
        assert!(!buf[(0, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn table_cells_compose_line_style_under_span_style() {
        use crate::style::{Color, Modifier};
        let cols = vec![Column::auto("c")];
        let rows = vec![vec![
            Line::from(vec![
                Span::raw("a"),
                Span::styled("b", Style::default().fg(Color::Blue)),
            ])
            .style(Style::default().fg(Color::Red).bold()),
        ]];
        let mut state = SelectState::new();
        state.select(None);
        let table = Table::new(cols, rows, &state).gutter(false);
        let buf = crate::testing::render(&table, 4, 2, &Theme::default());
        assert_eq!(buf[(0, 1)].fg, Color::Red);
        assert_eq!(buf[(1, 1)].fg, Color::Blue);
        assert!(buf[(0, 1)].modifier.contains(Modifier::BOLD));
        assert!(buf[(1, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn table_accepts_a_persistent_visible_window() {
        let columns = vec![Column::auto("name")];
        let rows = (0..8)
            .map(|index| vec![Line::from(format!("row{index}"))])
            .collect();
        let mut state = super::super::SelectViewportState::new();
        state.set_offset(3);
        state.select(Some(4));
        let window = state.resolve(8, 3);
        let table = Table::new(columns, rows, state.selection()).visible_window(window);
        let text = crate::testing::grid(&crate::testing::render(&table, 12, 4, &Theme::default()));
        assert!(text.contains("row3") && text.contains("row5"), "{text}");
        assert!(!text.contains("row2") && !text.contains("row6"), "{text}");
    }
}
