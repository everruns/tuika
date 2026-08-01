//! Pass two: [`MdItem`]s to width-fitted [`Line`]s.
//!
//! Wrapping, indentation, and the hyperlink runs a host applies after painting.
//! Prose word-wraps; verbatim code does not, because its indentation is
//! meaningful. Tables are wide enough a concern to live in
//! [`table`](super::table).

use ratatui_core::text::{Line, Span};

use crate::components::text::wrap_linked;
use crate::style::{StyleSheet, Theme};
use crate::term::hyperlink::BufferLink;

use super::Renderers;
use super::image::{MarkdownImage, image_cell_size};
use super::item::{MdItem, RichSpan};
use super::table::render_table_linked;

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
pub(super) fn flatten_linked(
    items: &[MdItem],
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    renderers: Renderers<'_>,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    let mut images = Vec::new();
    flatten_linked_into(items, width, theme, sheet, renderers, &mut images)
}

/// Flatten into lines and collect the block images reserved, with the row each
/// landed on, so the [`Markdown`](super::Markdown) view can overlay an [`Image`](crate::components::Image) at the matching
/// screen rect. A block image reserves `rows` blank lines here.
pub(super) fn flatten_into(
    items: &[MdItem],
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    renderers: Renderers<'_>,
    images: &mut Vec<MarkdownImage>,
) -> Vec<Line<'static>> {
    flatten_linked_into(items, width, theme, sheet, renderers, images).0
}

pub(super) fn flatten_linked_into(
    items: &[MdItem],
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    renderers: Renderers<'_>,
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
                for (row, row_links) in wrap_linked(spans, avail) {
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
                let rendered = renderers
                    .fenced
                    .and_then(|renderer| renderer.render(language, source, avail, theme));
                for line in rendered.as_ref().unwrap_or(fallback) {
                    out.push(prefix_rendered_line(*indent, line));
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
            // all HTML before the seam existed.
            MdItem::Html { source, indent } => {
                let avail = width.saturating_sub(*indent).max(1);
                if let Some(rendered) = renderers
                    .html
                    .and_then(|renderer| renderer.render(source, avail, theme, sheet))
                {
                    // The renderer numbers rows from its own first line and
                    // columns from its own left edge, since it cannot know where
                    // the block lands; rebase both onto the finished document so
                    // an `<a href>` inside the block is as clickable as one in
                    // the markdown around it.
                    let base = out.len().min(u16::MAX as usize) as u16;
                    for mut link in rendered.links {
                        link.line = link.line.saturating_add(base);
                        link.start_col = link.start_col.saturating_add(*indent);
                        link.end_col = link.end_col.saturating_add(*indent);
                        links.push(link);
                    }
                    for line in &rendered.lines {
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

/// Prefix an already-rendered line with `indent` blank columns.
fn prefix_rendered_line(indent: u16, line: &Line<'static>) -> Line<'static> {
    if indent == 0 {
        return line.clone();
    }
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(indent as usize)));
    spans.extend(line.spans.iter().cloned());
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

/// True for a spacer line — one this pass emitted for [`MdItem::Blank`].
///
/// Deliberately an emptiness check on the span list rather than a scan for
/// whitespace: the only lines that need collapsing are the ones emitted right
/// here, this runs once per blank in every reflow, and the reflow benchmark is
/// an instruction-count gate.
fn is_spacer(line: &Line<'static>) -> bool {
    line.spans.is_empty()
}
