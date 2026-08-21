//! Pass two: [`MdItem`]s to width-fitted [`Line`]s.
//!
//! Wrapping, indentation, and the hyperlink runs a host applies after painting.
//! Prose word-wraps; verbatim code does not, because its indentation is
//! meaningful. Tables are wide enough a concern to live in
//! [`table`](super::table).

use crate::style::Style;
use crate::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use crate::style::{StyleSheet, Theme};
use crate::term::hyperlink::BufferLink;
use crate::width::grapheme_cols;

use super::image::{MarkdownImage, image_cell_size};
use super::item::{MdItem, RichSpan};
use super::table::render_table_linked;
use super::{MarkdownBlock, MarkdownBlockContext, MarkdownBlockRenderer};

/// Reusable blank cells for full-width code backgrounds. Keeping the padding
/// borrowed avoids allocating and copying one new string per code row.
const CODE_PADDING_CHUNK: &str = concat!(
    "                                ",
    "                                ",
    "                                ",
    "                                ",
);

/// Trim surrounding whitespace from a cell's span run: the leading edge of the
/// first span and the trailing edge of the last, dropping any span left empty.
pub(super) fn trim_spans(mut spans: Vec<RichSpan>) -> Vec<RichSpan> {
    if let Some(first) = spans.first_mut() {
        first.content = first.content.trim_start().to_string();
    }
    if let Some(last) = spans.last_mut() {
        last.content = last.content.trim_end().to_string();
    }
    spans.retain(|s| !s.content.is_empty());
    spans
}

/// Display columns of a cell's span run, grapheme-aware.
pub(super) fn spans_cols(spans: &[RichSpan]) -> usize {
    spans
        .iter()
        .map(|s| crate::width::str_cols(&s.content) as usize)
        .sum()
}

/// Flatten parsed items into lines plus [`BufferLink`]s for every hyperlink run
/// that survived wrapping — labeled markdown links included.
pub(super) fn flatten_linked<R: MarkdownBlockRenderer + ?Sized>(
    items: &[MdItem],
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    renderers: &R,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    let mut images = Vec::new();
    flatten_linked_into(items, width, theme, sheet, renderers, &mut images)
}

/// Flatten into lines and links while collecting each reserved block image and
/// its row, so [`Markdown`](super::Markdown) can overlay an
/// [`Image`](crate::components::Image) at the matching screen rect.
pub(super) fn flatten_linked_into<R: MarkdownBlockRenderer + ?Sized>(
    items: &[MdItem],
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    renderers: &R,
    images: &mut Vec<MarkdownImage>,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    let mut out = Vec::new();
    let mut links = Vec::new();
    for item in items {
        match item {
            // A blank only separates: when the block it was separating rendered
            // nothing — a dropped HTML block, with no renderer attached — it
            // must not leave a gap at the top or a double gap in the middle.
            MdItem::Blank => {
                if !out.last().is_some_and(is_spacer) {
                    out.push(Line::default());
                }
            }
            MdItem::Prose { spans, indent } => {
                let avail = width.saturating_sub(*indent).max(1);
                for (row, row_links) in wrap_rich(spans, avail) {
                    let line_idx = out.len() as u16;
                    let line = prefix_line(*indent, row);
                    for mut bl in row_links {
                        bl.line = line_idx;
                        bl.start_col = bl.start_col.saturating_add(*indent);
                        bl.end_col = bl.end_col.saturating_add(*indent);
                        links.push(bl);
                    }
                    out.push(line);
                }
            }
            MdItem::CodeBlock {
                language,
                source,
                fallback,
                indent,
            } => {
                let avail = width.saturating_sub(*indent).max(1);
                let rendered = renderers.render(
                    MarkdownBlock::Fenced { language, source },
                    MarkdownBlockContext::new(avail, theme, sheet),
                );
                if let Some(rendered) = rendered {
                    for line in rendered {
                        out.push(prefix_rendered_line(*indent, line));
                    }
                } else {
                    let background = Style::default().bg(theme.code.background);
                    for row in fallback {
                        out.push(fill_code_line(
                            *indent, avail, &row.line, row.width, background,
                        ));
                    }
                }
            }
            MdItem::Table { table, indent } => {
                let avail = width.saturating_sub(*indent).max(1);
                for (row, row_links) in render_table_linked(table, avail, theme) {
                    let line_idx = out.len() as u16;
                    let line = prefix_line(*indent, row);
                    for mut bl in row_links {
                        bl.line = line_idx;
                        bl.start_col = bl.start_col.saturating_add(*indent);
                        bl.end_col = bl.end_col.saturating_add(*indent);
                        links.push(bl);
                    }
                    out.push(line);
                }
            }
            // No renderer means the block is dropped — markdown's behavior for
            // all HTML before the boundary existed.
            MdItem::Html { source, indent } => {
                let avail = width.saturating_sub(*indent).max(1);
                if let Some(rendered) = renderers.render(
                    MarkdownBlock::Html { source },
                    MarkdownBlockContext::new(avail, theme, sheet),
                ) {
                    for line in rendered {
                        out.push(prefix_rendered_line(*indent, line));
                    }
                }
            }
            MdItem::Image { data, alt, indent } => {
                let avail = width.saturating_sub(*indent).max(1);
                let (cols, rows) = image_cell_size(data, avail);
                images.push(MarkdownImage {
                    row: out.len().min(u16::MAX as usize) as u16,
                    indent: *indent,
                    cols,
                    rows,
                    data: data.clone(),
                    alt: alt.clone(),
                });
                // Reserve the image's rows; the view paints pixels over them.
                for _ in 0..rows {
                    out.push(Line::default());
                }
            }
        }
    }
    (out, links)
}

/// Prefix and right-pad one fallback code row in a single span allocation.
fn fill_code_line(
    indent: u16,
    available: u16,
    line: &Line<'static>,
    line_width: u16,
    background: Style,
) -> Line<'static> {
    let padding = usize::from(available.saturating_sub(line_width));
    let chunks = padding.div_ceil(CODE_PADDING_CHUNK.len());
    let mut spans = Vec::with_capacity(
        line.spans
            .len()
            .saturating_add(usize::from(indent > 0))
            .saturating_add(chunks),
    );
    if indent > 0 {
        spans.push(Span::raw(" ".repeat(indent as usize)));
    }
    spans.extend(line.spans.iter().cloned());

    let mut remaining = padding;
    while remaining > 0 {
        let width = remaining.min(CODE_PADDING_CHUNK.len());
        spans.push(Span::styled(&CODE_PADDING_CHUNK[..width], background));
        remaining -= width;
    }
    Line::from(spans)
}

/// Prefix an already-rendered line with `indent` blank columns.
fn prefix_rendered_line(indent: u16, line: Line<'static>) -> Line<'static> {
    if indent == 0 {
        return line;
    }
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(indent as usize)));
    spans.extend(line.spans);
    Line::from(spans)
}

/// Prefix `spans` with `indent` blank columns.
pub(super) fn prefix_line(indent: u16, mut spans: Vec<RichSpan>) -> Line<'static> {
    if indent == 0 {
        return Line::from(spans.into_iter().map(|s| s.to_span()).collect::<Vec<_>>());
    }
    let mut line = vec![Span::raw(" ".repeat(indent as usize))];
    line.extend(spans.drain(..).map(|s| s.to_span()));
    Line::from(line)
}

/// Word-wrap rich spans to `width`, preserving style and href across the reflow.
/// Returns each output row as `(spans, link runs relative to column 0)`.
pub(super) fn wrap_rich(spans: &[RichSpan], width: u16) -> Vec<(Vec<RichSpan>, Vec<BufferLink>)> {
    if width == 0 {
        let links = link_runs(spans, 0);
        return vec![(spans.to_vec(), links)];
    }
    // Grapheme cells carry style + href so a multi-scalar emoji stays intact and
    // a labeled link keeps its destination across the wrap.
    let cells: Vec<(&str, Style, Option<&str>)> = spans
        .iter()
        .flat_map(|s| {
            let href = s.href.as_deref();
            s.content.graphemes(true).map(move |g| (g, s.style, href))
        })
        .collect();
    let mut out: Vec<(Vec<RichSpan>, Vec<BufferLink>)> = Vec::new();
    let mut cur: Vec<(&str, Style, Option<&str>)> = Vec::new();
    let mut cur_w = 0u16;
    let mut i = 0;
    let n = cells.len();
    let is_break = |g: &str| g.chars().all(char::is_whitespace);
    while i < n {
        let mut separator = None;
        while i < n && is_break(cells[i].0) {
            // A separator may sit outside the preceding style scope (`[link] next`).
            // Preserve its own style and href when collapsing whitespace.
            separator.get_or_insert((cells[i].1, cells[i].2));
            i += 1;
        }
        if i == n {
            break;
        }
        let start = i;
        let mut word_w = 0u16;
        while i < n && !is_break(cells[i].0) {
            word_w = word_w.saturating_add(grapheme_cols(cells[i].0));
            i += 1;
        }
        let word = &cells[start..i];
        let sep = u16::from(!cur.is_empty() && separator.is_some());
        if word_w <= width && cur_w + sep + word_w <= width {
            if sep == 1 {
                let (st, href) = separator.expect("sep implies source whitespace");
                cur.push((" ", st, href));
                cur_w += 1;
            }
            cur.extend_from_slice(word);
            cur_w += word_w;
        } else if word_w <= width {
            if !cur.is_empty() {
                out.push(coalesce_rich(&cur));
                cur.clear();
            }
            cur.extend_from_slice(word);
            cur_w = word_w;
        } else {
            if !cur.is_empty() {
                out.push(coalesce_rich(&cur));
                cur.clear();
                cur_w = 0;
            }
            for &cell in word {
                let w = grapheme_cols(cell.0);
                if cur_w + w > width && !cur.is_empty() {
                    out.push(coalesce_rich(&cur));
                    cur.clear();
                    cur_w = 0;
                }
                cur.push(cell);
                cur_w += w;
            }
        }
    }
    if !cur.is_empty() {
        out.push(coalesce_rich(&cur));
    }
    if out.is_empty() {
        out.push((Vec::new(), Vec::new()));
    }
    out
}

pub(super) fn coalesce_rich(
    cells: &[(&str, Style, Option<&str>)],
) -> (Vec<RichSpan>, Vec<BufferLink>) {
    let mut spans: Vec<RichSpan> = Vec::new();
    let mut buf = String::new();
    let mut run_style: Option<Style> = None;
    let mut run_href: Option<String> = None;
    let flush = |spans: &mut Vec<RichSpan>,
                 buf: &mut String,
                 style: &mut Option<Style>,
                 href: &mut Option<String>| {
        if let Some(st) = style.take() {
            spans.push(RichSpan::styled(std::mem::take(buf), st, href.take()));
        }
    };
    for &(g, st, href) in cells {
        let href = href.map(str::to_string);
        match (&run_style, &run_href) {
            (Some(s), h) if *s == st && *h == href => buf.push_str(g),
            _ => {
                flush(&mut spans, &mut buf, &mut run_style, &mut run_href);
                run_style = Some(st);
                run_href = href;
                buf.push_str(g);
            }
        }
    }
    flush(&mut spans, &mut buf, &mut run_style, &mut run_href);
    let links = link_runs(&spans, 0);
    (spans, links)
}

/// Contiguous href runs in `spans`, with columns relative to `col_offset`.
pub(super) fn link_runs(spans: &[RichSpan], col_offset: u16) -> Vec<BufferLink> {
    let mut links = Vec::new();
    let mut col = col_offset;
    let mut i = 0;
    while i < spans.len() {
        let Some(url) = spans[i].href.clone() else {
            col = col.saturating_add(crate::width::str_cols(&spans[i].content));
            i += 1;
            continue;
        };
        let start = col;
        while i < spans.len() && spans[i].href.as_deref() == Some(url.as_str()) {
            col = col.saturating_add(crate::width::str_cols(&spans[i].content));
            i += 1;
        }
        if col > start {
            links.push(BufferLink {
                line: 0, // filled in by flatten
                start_col: start,
                end_col: col,
                url,
            });
        }
    }
    links
}

/// True for a spacer line — one this pass emitted for [`MdItem::Blank`].
///
/// Deliberately an emptiness check on the span list rather than a scan for
/// whitespace: the only lines that need collapsing are the ones emitted right
/// here, this runs once per blank in every reflow, and the reflow benchmark is
/// an instruction-count gate.
fn is_spacer(line: &Line<'static>) -> bool {
    line.spans.is_empty()
}
