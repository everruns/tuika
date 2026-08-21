//! `<table>` layout: cells to a fitted, box-drawn grid.
//!
//! Same shape as markdown's table renderer — natural widths, then shrink the
//! widest column until the grid fits, then a boxless fallback when even that
//! cannot — because the two sit side by side in one document and a table should
//! not change character depending on which syntax wrote it.

use tuika::components::text::wrap_lines;
use tuika::ui::Style;
use tuika::ui::{Line, Span};
use tuika::width::str_cols;

use markup5ever_rcdom::Handle;

use crate::block::Layout;
use crate::dom::tag;
use crate::inline::{InlineBuf, collect};

/// Minimum content columns a column keeps while shrinking.
const MIN_COL: u16 = 3;

/// One cell's inline content.
type Cell = Vec<Span<'static>>;

/// Render a `<table>` element to `width` columns.
pub(crate) fn render(node: &Handle, width: u16, cx: &mut Layout<'_>) -> Vec<Line<'static>> {
    let mut header: Vec<Cell> = Vec::new();
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    collect_rows(node, cx, &mut header, &mut rows);

    if header.is_empty() && rows.is_empty() {
        return Vec::new();
    }
    // A table with no `<th>` anywhere is all body; the box still needs a top.
    if header.is_empty() {
        header = rows.remove(0);
    }
    let cols = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if cols == 0 {
        return Vec::new();
    }
    pad(&mut header, cols);
    for row in &mut rows {
        pad(row, cols);
    }

    // Borders cost `│ ` before every column and ` │` after the last.
    let chrome = 3 * cols as u16 + 1;
    match fit(&header, &rows, cols, width.saturating_sub(chrome)) {
        Some(widths) => boxed(&header, &rows, &widths),
        None => plain(&header, &rows, width),
    }
}

fn collect_rows(
    node: &Handle,
    cx: &mut Layout<'_>,
    header: &mut Vec<Cell>,
    rows: &mut Vec<Vec<Cell>>,
) {
    for child in node.children.borrow().iter() {
        match tag(child).as_deref() {
            Some("thead") | Some("tbody") | Some("tfoot") => collect_rows(child, cx, header, rows),
            Some("tr") => {
                let mut cells = Vec::new();
                let mut is_header = false;
                for cell in child.children.borrow().iter() {
                    let name = tag(cell);
                    match name.as_deref() {
                        Some("th") => {
                            is_header = true;
                            cells.push(cell_spans(cell, cx, true));
                        }
                        Some("td") => cells.push(cell_spans(cell, cx, false)),
                        _ => {}
                    }
                }
                if cells.is_empty() {
                    continue;
                }
                if is_header && header.is_empty() {
                    *header = cells;
                } else {
                    rows.push(cells);
                }
            }
            _ => {}
        }
    }
}

fn cell_spans(node: &Handle, cx: &mut Layout<'_>, header: bool) -> Cell {
    let style = if header {
        cx.sheet.heading.apply(cx.base_style())
    } else {
        cx.base_style()
    };
    let mut buf = InlineBuf::new();
    for child in node.children.borrow().iter() {
        collect(
            child,
            style,
            cx.theme,
            cx.sheet,
            0,
            cx.limits.max_depth,
            &mut buf,
        );
    }
    // A cell is one line: a `<br>` or a nested block inside it becomes a space,
    // the same collapse markdown applies to a hard break in a cell.
    let mut spans: Cell = Vec::new();
    for (i, line) in buf.take().into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ".to_string(), style));
        }
        spans.extend(line);
    }
    trim(spans)
}

fn trim(mut spans: Cell) -> Cell {
    if let Some(first) = spans.first_mut() {
        first.content = first.content.trim_start().to_string().into();
    }
    if let Some(last) = spans.last_mut() {
        last.content = last.content.trim_end().to_string().into();
    }
    spans.retain(|s| !s.content.is_empty());
    spans
}

fn pad(row: &mut Vec<Cell>, cols: usize) {
    while row.len() < cols {
        row.push(Vec::new());
    }
    row.truncate(cols);
}

/// Column widths that fit `content` columns, or `None` when even the minimum
/// does not.
fn fit(header: &[Cell], rows: &[Vec<Cell>], cols: usize, content: u16) -> Option<Vec<u16>> {
    if content < MIN_COL * cols as u16 {
        return None;
    }
    let mut widths: Vec<u16> = (0..cols)
        .map(|c| {
            std::iter::once(header)
                .chain(rows.iter().map(Vec::as_slice))
                .map(|row| row.get(c).map(cell_cols).unwrap_or(0))
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    // Shrink the widest column until the row fits; cells in it wrap.
    while widths.iter().sum::<u16>() > content {
        let widest = widths
            .iter()
            .enumerate()
            .max_by_key(|(_, w)| **w)
            .map(|(i, _)| i)?;
        if widths[widest] <= MIN_COL {
            return None;
        }
        widths[widest] -= 1;
    }
    Some(widths)
}

fn cell_cols(cell: &Cell) -> u16 {
    cell.iter().map(|s| str_cols(&s.content)).sum()
}

fn boxed(header: &[Cell], rows: &[Vec<Cell>], widths: &[u16]) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    out.push(rule('╭', '┬', '╮', widths));
    out.extend(row_lines(header, widths));
    out.push(rule('├', '┼', '┤', widths));
    for row in rows {
        out.extend(row_lines(row, widths));
    }
    out.push(rule('╰', '┴', '╯', widths));
    out
}

fn rule(left: char, mid: char, right: char, widths: &[u16]) -> Line<'static> {
    let mut s = String::new();
    s.push(left);
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            s.push(mid);
        }
        s.push_str(&"─".repeat(*w as usize + 2));
    }
    s.push(right);
    Line::from(Span::raw(s))
}

/// One table row, which is as tall as its tallest wrapped cell.
fn row_lines(row: &[Cell], widths: &[u16]) -> Vec<Line<'static>> {
    let wrapped: Vec<Vec<Line<'static>>> = row
        .iter()
        .zip(widths)
        .map(|(cell, w)| {
            let line = Line::from(cell.clone());
            let lines = wrap_lines(std::slice::from_ref(&line), *w);
            if lines.is_empty() {
                vec![Line::default()]
            } else {
                lines
            }
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    (0..height)
        .map(|r| {
            let mut spans = vec![Span::raw("│ ".to_string())];
            for (c, w) in widths.iter().enumerate() {
                if c > 0 {
                    spans.push(Span::raw(" │ ".to_string()));
                }
                // `.get` on both axes: the row/width lengths are equalized by
                // `pad` above, but that is an invariant maintained at a
                // distance, and this runs on untrusted markup.
                let cell = wrapped.get(c).and_then(|lines| lines.get(r));
                let used = cell.map(|l| l.spans.iter().map(|s| str_cols(&s.content)).sum::<u16>());
                if let Some(cell) = cell {
                    spans.extend(cell.spans.iter().cloned());
                }
                let pad = w.saturating_sub(used.unwrap_or(0));
                if pad > 0 {
                    spans.push(Span::raw(" ".repeat(pad as usize)));
                }
            }
            spans.push(Span::raw(" │".to_string()));
            Line::from(spans)
        })
        .collect()
}

/// Too narrow for a grid: ` | `-joined rows that word-wrap, so the content
/// survives even when the shape cannot.
fn plain(header: &[Cell], rows: &[Vec<Cell>], width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" | ".to_string(), Style::default()));
            }
            spans.extend(cell.iter().cloned());
        }
        lines.push(Line::from(spans));
    }
    wrap_lines(&lines, width.max(1))
}
