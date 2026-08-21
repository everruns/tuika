//! GFM table layout: per-column fitting, borders, and the narrow fallback.
//!
//! Called from [`flatten`](super::flatten) and separated from it only by size —
//! fitting columns to a width, drawing box rules, and degrading to a plain
//! layout when the table cannot fit is a third of that pass on its own.

use crate::style::Style;
use pulldown_cmark::Alignment;

use crate::style::Theme;
use crate::term::hyperlink::BufferLink;

use super::flatten::{spans_cols, wrap_rich};
use super::item::{Cell, RichSpan, TableData};

/// Lay a table out to `width` columns with box-drawing borders. Column widths
/// fit the content, shrinking the widest columns (and wrapping their cells)
/// until the whole table fits. Link hrefs in cells are preserved.
pub(super) fn render_table_linked(
    table: &TableData,
    width: u16,
    theme: &Theme,
) -> Vec<(Vec<RichSpan>, Vec<BufferLink>)> {
    let cols = table
        .header
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(1);

    if (width as usize) < 4 * cols + 1 {
        return render_table_plain_linked(table, width, cols, theme);
    }

    let mut widths = vec![0usize; cols];
    for (c, w) in widths.iter_mut().enumerate() {
        *w = spans_cols(cell_at(&table.header, c));
        for row in &table.rows {
            *w = (*w).max(spans_cols(cell_at(row, c)));
        }
        *w = (*w).max(1);
    }

    let budget = (width as usize).saturating_sub(3 * cols + 1).max(cols);
    while widths.iter().sum::<usize>() > budget {
        let (idx, _) = widths.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
        if widths[idx] <= 1 {
            break;
        }
        widths[idx] -= 1;
    }

    let border = Style::default().fg(theme.dim);
    let mut out = Vec::new();
    out.push((
        vec![RichSpan::styled(
            rule_row_text('╭', '┬', '╮', &widths),
            border,
            None,
        )],
        vec![],
    ));
    out.extend(cell_rows_linked(
        &table.header,
        &widths,
        &table.aligns,
        border,
        cols,
    ));
    out.push((
        vec![RichSpan::styled(
            rule_row_text('├', '┼', '┤', &widths),
            border,
            None,
        )],
        vec![],
    ));
    for row in &table.rows {
        out.extend(cell_rows_linked(row, &widths, &table.aligns, border, cols));
    }
    out.push((
        vec![RichSpan::styled(
            rule_row_text('╰', '┴', '╯', &widths),
            border,
            None,
        )],
        vec![],
    ));
    out
}

pub(super) fn render_table_plain_linked(
    table: &TableData,
    width: u16,
    cols: usize,
    theme: &Theme,
) -> Vec<(Vec<RichSpan>, Vec<BufferLink>)> {
    let sep = Style::default().fg(theme.dim);
    let join = |row: &[Cell]| -> Vec<RichSpan> {
        let mut spans = Vec::new();
        for c in 0..cols {
            if c > 0 {
                spans.push(RichSpan::styled(" | ".to_string(), sep, None));
            }
            spans.extend(cell_at(row, c).iter().cloned());
        }
        spans
    };

    let mut out = Vec::new();
    out.extend(wrap_rich(&join(&table.header), width));
    for row in &table.rows {
        out.extend(wrap_rich(&join(row), width));
    }
    out
}

/// The `c`th cell of `row`, or an empty span run when the row is short.
fn cell_at(row: &[Cell], c: usize) -> &[RichSpan] {
    row.get(c).map(Vec::as_slice).unwrap_or(&[])
}

fn rule_row_text(left: char, mid: char, right: char, widths: &[usize]) -> String {
    let mut s = String::new();
    s.push(left);
    for (i, w) in widths.iter().enumerate() {
        s.push_str(&"─".repeat(w + 2));
        s.push(if i + 1 == widths.len() { right } else { mid });
    }
    s
}

fn cell_rows_linked(
    row: &[Cell],
    widths: &[usize],
    aligns: &[Alignment],
    border: Style,
    cols: usize,
) -> Vec<(Vec<RichSpan>, Vec<BufferLink>)> {
    let wrapped: Vec<Vec<(Vec<RichSpan>, Vec<BufferLink>)>> = (0..cols)
        .map(|c| {
            let cell = cell_at(row, c);
            if cell.is_empty() {
                vec![(Vec::new(), Vec::new())]
            } else {
                let lines = wrap_rich(cell, widths[c].max(1) as u16);
                if lines.is_empty() {
                    vec![(Vec::new(), Vec::new())]
                } else {
                    lines
                }
            }
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let empty: (Vec<RichSpan>, Vec<BufferLink>) = (Vec::new(), Vec::new());
    let mut lines = Vec::new();
    for r in 0..height {
        let mut spans = vec![RichSpan::styled("│".to_string(), border, None)];
        let mut links = Vec::new();
        let mut col = 1u16; // after the leading │
        for (c, width) in widths.iter().enumerate() {
            let (content, cell_links) = wrapped[c].get(r).unwrap_or(&empty);
            let content_w = spans_cols(content) as u16;
            let pad = (*width as u16).saturating_sub(content_w);
            let align = aligns.get(c).copied().unwrap_or(Alignment::None);
            let (left, right) = match align {
                Alignment::Right => (pad, 0),
                Alignment::Center => (pad / 2, pad - pad / 2),
                _ => (0, pad),
            };
            // gutter space
            spans.push(RichSpan::styled(" ".to_string(), Style::default(), None));
            col += 1;
            if left > 0 {
                spans.push(RichSpan::styled(
                    " ".repeat(left as usize),
                    Style::default(),
                    None,
                ));
                col += left;
            }
            for mut bl in cell_links.iter().cloned() {
                bl.start_col = bl.start_col.saturating_add(col);
                bl.end_col = bl.end_col.saturating_add(col);
                links.push(bl);
            }
            spans.extend(content.iter().cloned());
            col = col.saturating_add(content_w);
            if right > 0 {
                spans.push(RichSpan::styled(
                    " ".repeat(right as usize),
                    Style::default(),
                    None,
                ));
                col += right;
            }
            spans.push(RichSpan::styled(" ".to_string(), Style::default(), None));
            col += 1;
            spans.push(RichSpan::styled("│".to_string(), border, None));
            col += 1;
        }
        lines.push((spans, links));
    }
    lines
}

#[cfg(test)]
mod tests {

    use super::super::testutil::*;
    use super::super::to_lines;
    use crate::components::text::line_width;

    use crate::highlight::CodeHighlighter;
    use crate::style::{StyleSheet, Theme};
    use crate::text::Span;

    use crate::style::Modifier;

    #[test]
    fn table_renders_boxed_with_headers() {
        let src = "| A | B |\n| - | - |\n| 1 | 2 |";
        let out = plain(src, 40);
        assert!(out.iter().any(|l| l.contains('╭')), "top border: {out:?}");
        assert!(
            out.iter().any(|l| l.contains('A') && l.contains('B')),
            "header: {out:?}"
        );
        assert!(
            out.iter().any(|l| l.contains('1') && l.contains('2')),
            "row: {out:?}"
        );
    }

    #[test]
    fn table_header_cells_are_bold_and_themed() {
        let theme = Theme::default();
        let src = "| Name | Kind |\n| --- | --- |\n| a | b |";
        let lines = to_lines(
            src,
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let head = lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content.contains("Name"))
            .expect("header cell span");
        assert!(head.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(head.style.fg, Some(theme.code.heading));
    }

    #[test]
    fn table_cell_link_is_styled() {
        let theme = Theme::default();
        let src = "| Site |\n| --- |\n| [yolop](https://everruns.dev) |";
        let lines = to_lines(
            src,
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let link = lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content.contains("yolop"))
            .expect("link cell span");
        assert_eq!(link.style.fg, Some(theme.code.link));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn table_cell_bold_and_inline_code_survive() {
        let theme = Theme::default();
        let src = "| Col |\n| --- |\n| **hi** and `cargo` |";
        let lines = to_lines(
            src,
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let spans: Vec<&Span> = lines.iter().flat_map(|l| &l.spans).collect();
        let bold = spans
            .iter()
            .find(|s| s.content.contains("hi"))
            .expect("bold span");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let code = spans
            .iter()
            .find(|s| s.content.contains("cargo"))
            .expect("code span");
        assert_eq!(code.style.bg, Some(theme.code.background));
    }

    #[test]
    fn table_cell_emoji_keeps_borders_aligned() {
        // A wide emoji is measured grapheme-aware, so every boxed row stays the
        // same rendered width and the borders line up.
        let src = "| Status |\n| --- |\n| ok ✅ |\n| bad |";
        let lines = to_lines(
            src,
            40,
            &Theme::default(),
            &StyleSheet::default(),
            CodeHighlighter::Plain,
        );
        let box_rows: Vec<u16> = lines
            .iter()
            .filter(|l| text(l).contains('│'))
            .map(line_width)
            .collect();
        assert!(box_rows.len() >= 2, "expected boxed rows: {box_rows:?}");
        assert!(
            box_rows.windows(2).all(|w| w[0] == w[1]),
            "boxed rows must share one width: {box_rows:?}"
        );
    }

    #[test]
    fn table_falls_back_to_plain_when_too_narrow_and_always_fits() {
        let src = "| Col A | Col B |\n| --- | --- |\n| alpha | beta |";
        for width in [4u16, 8, 12, 20, 48] {
            let lines = to_lines(
                src,
                width,
                &Theme::default(),
                &StyleSheet::default(),
                CodeHighlighter::Plain,
            );
            for line in &lines {
                assert!(
                    line_width(line) <= width,
                    "table line exceeded width {width}: {:?}",
                    text(line)
                );
            }
            let body: String = lines.iter().map(text).collect::<Vec<_>>().join("\n");
            // "Col" (3 cols) survives intact even at the narrowest width; wider
            // cells may wrap across rows below the boxing threshold.
            assert!(body.contains("Col"), "content survives at width {width}");
        }
    }
}
