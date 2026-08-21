//! Text and paragraph components.
//!
//! [`Text`] draws pre-styled ratatui [`Line`]s faithfully (`Line::style` under
//! each `Span::style`), clipping to its area. [`Paragraph`] takes plain prose
//! plus a base style, word-wraps it to the available width, and links bare web
//! URLs. [`Wrap`] is the styled counterpart to `Paragraph`: it word-wraps
//! pre-styled `Line`s while preserving each span's style across the reflow (see
//! [`wrap_lines`]).
//!
//! Horizontal alignment is honored throughout. [`Text`] and [`Wrap`] read each
//! [`Line::alignment`] (unset = flush-left), so centered titles, right-aligned
//! totals, and centered empty-state messages built elsewhere render as intended
//! rather than snapping to the left edge; `Wrap` carries a line's alignment onto
//! every row it reflows to. [`Paragraph`] takes a single alignment for the whole
//! block via [`Paragraph::alignment`].

use std::ops::Range;

use crate::geometry::Rect;
use crate::style::Style;
use crate::text::Alignment;
use crate::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use crate::geometry::Size;
use crate::style::Role;
use crate::surface::Surface;
use crate::term::hyperlink::{BufferLink, LinkPolicy, apply_buffer_links_in, find_links};
use crate::view::{RenderCtx, View};
use crate::width::{grapheme_cols, str_cols};

/// Starting column for `content_width` columns of content placed in `area`
/// under `alignment`: flush-left, centered, or flush-right within the width.
/// Content wider than the area pins to the left edge (slack saturates to 0).
pub(crate) fn aligned_x(alignment: Alignment, content_width: u16, area: Rect) -> u16 {
    let slack = area.width.saturating_sub(content_width);
    match alignment {
        Alignment::Left => area.x,
        Alignment::Center => area.x.saturating_add(slack / 2),
        Alignment::Right => area.x.saturating_add(slack),
    }
}

/// Draw pre-styled `lines` top-down from `area`'s origin, clipping to `area`.
/// Each line's horizontal start honors its [`Line::alignment`] — an unset
/// alignment is flush-left, so pre-styled lines built elsewhere keep their
/// centered/right-aligned intent instead of silently snapping to the left.
/// Shared by [`Text`] and [`Wrap`].
fn draw_lines(lines: &[Line<'static>], area: Rect, surface: &mut Surface) {
    for (row, line) in lines.iter().enumerate() {
        let y = area.y.saturating_add(row as u16);
        if y >= area.bottom() {
            break;
        }
        let align = line.alignment.unwrap_or(Alignment::Left);
        let mut x = aligned_x(align, line_width(line), area);
        for span in &line.spans {
            if x >= area.right() {
                break;
            }
            x = surface.set_string(x, y, span.content.as_ref(), line.style.patch(span.style));
        }
    }
}

/// Display width of a styled line (sum of span widths).
pub fn line_width(line: &Line) -> u16 {
    line.spans
        .iter()
        .map(|s| str_cols(s.content.as_ref()))
        .fold(0, u16::saturating_add)
}

/// A block of pre-styled lines, drawn top-down and clipped.
///
/// Each line is placed horizontally by its own [`Line::alignment`]: an unset
/// alignment (the default) is flush-left, while lines built with ratatui's
/// `.centered()` / `.right_aligned()` render centered or flush-right within the
/// render width. This lets a host feed in `Line`s produced by an existing
/// formatting layer without losing their alignment.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::ui::Line;
/// # use tuika::testing::render;
/// let view = Text::new(vec![
///     Line::from("left"),
///     Line::from("mid").centered(),
///     Line::from("end").right_aligned(),
/// ]);
/// let buffer = render(&view, 7, 3, &Theme::default());
/// # use tuika::testing::grid;
/// // width 7: "left" flush-left, "mid" centered (slack 4 -> col 2),
/// // "end" flush-right (slack 4 -> col 4). `grid` keeps trailing cells.
/// assert_eq!(grid(&buffer), "left   \n  mid  \n    end");
/// ```
///
/// ![text demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/text.png)
pub struct Text {
    lines: Vec<Line<'static>>,
}

impl Text {
    /// A text block from pre-styled lines.
    pub fn new(lines: Vec<Line<'static>>) -> Self {
        Self { lines }
    }

    /// A single unstyled line from a string.
    pub fn raw(text: impl Into<String>) -> Self {
        Self::new(vec![Line::from(text.into())])
    }
}

impl View for Text {
    fn measure(&self, _available: Size, _ctx: &RenderCtx) -> Size {
        let width = self.lines.iter().map(line_width).max().unwrap_or(0);
        Size::new(width, self.lines.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, _ctx: &RenderCtx) {
        draw_lines(&self.lines, area, surface);
    }
}

/// Plain prose word-wrapped to the render width from one base style.
///
/// Bare `http(s)` URLs are styled with the active [`Role::Link`] and emitted as
/// OSC 8 hyperlinks by default. Because this component owns the complete source
/// text, links remain intact across wrapping without requiring a
/// [`HyperlinkBackend`](crate::term::hyperlink::HyperlinkBackend). Use
/// [`link_policy`](Self::link_policy) to opt out or enable another supported
/// scheme.
pub struct Paragraph {
    text: String,
    style: Style,
    align: Alignment,
    link_policy: LinkPolicy,
}

struct ParagraphLine {
    text: String,
    links: Vec<ParagraphLink>,
}

struct ParagraphLink {
    start: usize,
    end: usize,
    url: String,
}

impl Paragraph {
    /// A paragraph that wraps `text` from one base `style`, flush-left.
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            align: Alignment::Left,
            link_policy: LinkPolicy::default(),
        }
    }

    /// Horizontally align every wrapped line within the render width. Defaults
    /// to [`Alignment::Left`]; pass [`Alignment::Center`] for a centered
    /// empty-state message or [`Alignment::Right`] for a right-aligned block.
    pub fn alignment(mut self, align: Alignment) -> Self {
        self.align = align;
        self
    }

    /// Configure which bare-URL schemes become styled OSC 8 hyperlinks.
    ///
    /// The default ([`LinkPolicy::WEB`]) links `http(s)` only. Pass
    /// [`LinkPolicy::NONE`] to keep URLs literal and in the paragraph's base
    /// style.
    pub fn link_policy(mut self, policy: LinkPolicy) -> Self {
        self.link_policy = policy;
        self
    }

    fn wrap(&self, width: u16, link_policy: LinkPolicy) -> Vec<ParagraphLine> {
        if width == 0 {
            return Vec::new();
        }
        self.text
            .split('\n')
            .flat_map(|para| {
                let links = find_links(para, link_policy);
                wrap_str(para, width).into_iter().map(move |row| {
                    if links.is_empty() {
                        return ParagraphLine {
                            text: row.text,
                            links: Vec::new(),
                        };
                    }
                    // Each word is copied verbatim, so a source link range
                    // intersected with a word's range maps onto the row by a
                    // constant shift — no searching the row's text back through
                    // the source, and repeated words cannot alias.
                    let mut out = Vec::new();
                    for (row_start, src) in &row.words {
                        for &(link_start, link_end) in &links {
                            let start = link_start.max(src.start);
                            let end = link_end.min(src.end);
                            if start >= end {
                                continue;
                            }
                            out.push(ParagraphLink {
                                start: row_start + (start - src.start),
                                end: row_start + (end - src.start),
                                url: para[link_start..link_end].to_string(),
                            });
                        }
                    }
                    // A URL hard-broken across rows contributes one piece per
                    // word; render walks them in order, so keep them sorted.
                    out.sort_by_key(|link| link.start);
                    ParagraphLine {
                        text: row.text,
                        links: out,
                    }
                })
            })
            .collect()
    }
}

impl View for Paragraph {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        // Hyperlink metadata and styling never affect paragraph geometry.
        let lines = self.wrap(available.width, LinkPolicy::NONE);
        let width = lines
            .iter()
            .map(|line| str_cols(line.text.as_str()))
            .max()
            .unwrap_or(0);
        Size::new(width, lines.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        ctx.record_selection_source(area, self.text.to_string());
        let mut buffer_links = Vec::new();
        let link_style = ctx.sheet.resolve(Role::Link).apply(self.style);
        for (row, line) in self
            .wrap(area.width, self.link_policy)
            .into_iter()
            .enumerate()
        {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            let x = aligned_x(self.align, str_cols(line.text.as_str()), area);
            let mut byte = 0usize;
            let mut col = x;
            for link in line.links {
                col = surface.set_string(col, y, &line.text[byte..link.start], self.style);
                let start_col = col.saturating_sub(area.x);
                col = surface.set_string(col, y, &line.text[link.start..link.end], link_style);
                let end_col = col.saturating_sub(area.x);
                if end_col > start_col {
                    buffer_links.push(BufferLink {
                        line: row.min(u16::MAX as usize) as u16,
                        start_col,
                        end_col,
                        url: link.url,
                    });
                }
                byte = link.end;
            }
            surface.set_string(col, y, &line.text[byte..], self.style);
        }
        apply_buffer_links_in(surface.buffer_mut(), area, &buffer_links, self.link_policy);
    }
}

/// Whether a grapheme cluster is a break opportunity (all-whitespace).
fn is_break(cluster: &str) -> bool {
    cluster.chars().all(char::is_whitespace)
}

/// Word-wrap pre-styled `lines` to `width` columns, preserving each span's
/// style across the wrap.
///
/// Unlike [`Paragraph`] (single style, plain text), the input may be
/// multi-styled — highlighted code, a linkified URL run, a diff line — and the
/// per-span styling survives the reflow. Wrapping is greedy and word-oriented:
/// runs of whitespace collapse to a single break opportunity, a word longer
/// than `width` is hard-broken so no output line exceeds `width`, and a blank
/// (empty or all-whitespace) input line stays exactly one blank output line.
/// Widths are counted in display columns, so wide/CJK glyphs wrap correctly.
/// A `width` of 0 returns the input unchanged.
pub fn wrap_lines(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return lines.to_vec();
    }
    let mut out = Vec::new();
    for line in lines {
        wrap_one(line, width, &mut out);
    }
    out
}

fn wrap_one(line: &Line<'static>, width: u16, out: &mut Vec<Line<'static>>) {
    // Cells are grapheme clusters, not `char`s, so a multi-scalar emoji stays
    // intact across the reflow instead of being split mid-cluster.
    let cells: Vec<(&str, Style)> = line
        .spans
        .iter()
        .flat_map(|s| {
            s.content
                .graphemes(true)
                .map(move |g| (g, line.style.patch(s.style)))
        })
        .collect();
    let before = out.len();
    let metrics: Vec<CellMetrics> = cells.iter().map(|&(g, _)| CellMetrics::of(g)).collect();
    for row in wrap_cells(&metrics, width) {
        let mut cur: Vec<(&str, Style)> = Vec::new();
        for word in row {
            // The joining space inherits the preceding cell's style so a
            // background run stays continuous across the join.
            if let Some(&(_, prev)) = cur.last() {
                cur.push((" ", prev));
            }
            cur.extend_from_slice(&cells[word]);
        }
        out.push(coalesce(&cur));
    }
    // Carry the source line's alignment onto every row it reflowed to, so a
    // centered/right-aligned line stays aligned after wrapping.
    if let Some(align) = line.alignment {
        for produced in &mut out[before..] {
            produced.alignment = Some(align);
        }
    }
}

/// What the wrap solver needs to know about one grapheme cell: how many columns
/// it advances, and whether it is a break opportunity.
#[derive(Clone, Copy)]
struct CellMetrics {
    cols: u16,
    is_break: bool,
}

impl CellMetrics {
    fn of(cluster: &str) -> Self {
        Self {
            cols: grapheme_cols(cluster),
            is_break: is_break(cluster),
        }
    }
}

/// The wrap solver, shared by every prose path in tuika.
///
/// Greedy and word-oriented over grapheme cells: runs of whitespace collapse to
/// a single break opportunity, and a word wider than `width` is hard-broken so
/// no row exceeds it. Each output row is the list of cell-index ranges of the
/// words on it — callers decide how to join them, which is the only thing
/// [`wrap_lines`] (a styled space) and [`wrap_str`] (a plain one) disagree
/// about. Blank (empty or all-whitespace) input yields exactly one empty row, so
/// a blank source line survives as a blank output line rather than vanishing.
///
/// Breaking only at whitespace is deliberate. A Unicode line-break table would
/// also offer `/` and `-`, which reads better for prose but splits a bare URL
/// after its scheme — and [`Paragraph`] turns bare URLs into OSC 8 hyperlinks,
/// so keeping one contiguous is worth more than the ragged edge it costs.
fn wrap_cells(cells: &[CellMetrics], width: u16) -> Vec<Vec<Range<usize>>> {
    let mut rows: Vec<Vec<Range<usize>>> = Vec::new();
    let mut cur: Vec<Range<usize>> = Vec::new();
    let mut cur_w = 0u16;
    let mut i = 0;
    let n = cells.len();
    while i < n {
        // Collapse a run of whitespace into a single break opportunity.
        if cells[i].is_break {
            i += 1;
            continue;
        }
        // Gather one word (a maximal run of non-whitespace).
        let start = i;
        let mut word_w = 0u16;
        while i < n && !cells[i].is_break {
            word_w = word_w.saturating_add(cells[i].cols);
            i += 1;
        }
        let sep = u16::from(!cur.is_empty());
        // Saturating throughout: `width` is `u16::MAX` for a max-content
        // measurement, so a paragraph long enough to fill a row at that width
        // would otherwise overflow the column counter and panic. Saturating
        // keeps the row "still fits", which is the right answer when the caller
        // asked how wide the text wants to be.
        if word_w <= width && cur_w.saturating_add(sep).saturating_add(word_w) <= width {
            // Fits on the current row, with a joining space if it is not first.
            cur.push(start..i);
            cur_w = cur_w.saturating_add(sep).saturating_add(word_w);
        } else if word_w <= width {
            // Doesn't fit; break to a new row, then place the word.
            if !cur.is_empty() {
                rows.push(std::mem::take(&mut cur));
            }
            cur.push(start..i);
            cur_w = word_w;
        } else {
            // Word wider than the row: hard-break it across rows. Each piece is
            // its own word, so no joining space is inserted between them.
            if !cur.is_empty() {
                rows.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            let mut piece = start;
            for (cell, metrics) in cells.iter().enumerate().take(i).skip(start) {
                let w = metrics.cols;
                if cur_w.saturating_add(w) > width && cell > piece {
                    cur.push(piece..cell);
                    rows.push(std::mem::take(&mut cur));
                    piece = cell;
                    cur_w = 0;
                }
                cur_w = cur_w.saturating_add(w);
            }
            cur.push(piece..i);
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    // Preserve a blank (empty or all-whitespace) input line as one blank row.
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// One row of [`wrap_str`]: the wrapped text, plus where each of its pieces came
/// from in the source.
pub(crate) struct WrappedRow {
    /// The row's text, words joined by a single space.
    pub text: String,
    /// `(row byte offset, source byte range)` per word, in order. Each word is
    /// copied verbatim, so the byte at `row_offset + k` is the source byte at
    /// `src.start + k` — which is what lets a caller carry source metadata (a
    /// link range, a search hit) onto the wrapped row exactly.
    pub words: Vec<(usize, Range<usize>)>,
}

/// Word-wrap plain `text` to `width` columns.
///
/// The plain-string counterpart to [`wrap_lines`], on the same solver and so
/// with the same rules — including tuika's grapheme-aware column widths, which
/// is what keeps a paragraph's wrap and its measurement from disagreeing about a
/// ZWJ emoji or a wide glyph. `text` must not contain a newline; callers split
/// on `\n` first, so a hard break stays a hard break.
pub(crate) fn wrap_str(text: &str, width: u16) -> Vec<WrappedRow> {
    let cells: Vec<(usize, &str)> = text.grapheme_indices(true).collect();
    let metrics: Vec<CellMetrics> = cells.iter().map(|&(_, g)| CellMetrics::of(g)).collect();
    wrap_cells(&metrics, width)
        .into_iter()
        .map(|row| {
            let mut out = WrappedRow {
                text: String::new(),
                words: Vec::new(),
            };
            for word in row {
                if !out.text.is_empty() {
                    out.text.push(' ');
                }
                let (last_offset, last) = cells[word.end - 1];
                let start = cells[word.start].0;
                let end = last_offset + last.len();
                out.words.push((out.text.len(), start..end));
                out.text.push_str(&text[start..end]);
            }
            out
        })
        .collect()
}

/// Merge a run of styled grapheme cells into a [`Line`], coalescing adjacent
/// cells with equal style into one [`Span`].
fn coalesce(cells: &[(&str, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut run: Option<Style> = None;
    for &(g, st) in cells {
        match run {
            Some(s) if s == st => buf.push_str(g),
            _ => {
                if let Some(s) = run.take() {
                    spans.push(Span::styled(std::mem::take(&mut buf), s));
                }
                run = Some(st);
                buf.push_str(g);
            }
        }
    }
    if let Some(s) = run {
        spans.push(Span::styled(buf, s));
    }
    Line::from(spans)
}

/// Multi-styled text, word-wrapped to the render width with per-span styles
/// preserved.
///
/// This is the styled counterpart to [`Paragraph`]: feed it the styled
/// [`Line`]s a host already builds — syntax-highlighted code, linkified URLs, a
/// colored diff — and it reflows them to the available width without flattening
/// the styling (see [`wrap_lines`] for the exact wrapping rules).
pub struct Wrap {
    lines: Vec<Line<'static>>,
}

impl Wrap {
    /// A wrapping view over pre-styled lines.
    pub fn new(lines: Vec<Line<'static>>) -> Self {
        Self { lines }
    }
}

impl View for Wrap {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let wrapped = wrap_lines(&self.lines, available.width);
        let width = wrapped.iter().map(line_width).max().unwrap_or(0);
        Size::new(width.min(available.width), wrapped.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, _ctx: &RenderCtx) {
        let wrapped = wrap_lines(&self.lines, area.width);
        draw_lines(&wrapped, area, surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::style::Theme;
    use crate::style::{Color, Modifier, Style};
    use crate::term::hyperlink::ctrl_click_url;
    use crate::tests::support::{buffer, row};
    use crate::text::{Line, Span};
    use crate::view::{RenderCtx, View};
    use crate::{Mouse, MouseButton, MouseKind, Size, Surface};

    /// Concatenated text of a line's spans.
    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn link_at(buffer: &crate::buffer::Buffer, area: Rect, col: u16, row: u16) -> Option<String> {
        let mut event = Mouse::at(MouseKind::Up(MouseButton::Left), col, row);
        event.ctrl = true;
        ctrl_click_url(&event, buffer, area)
    }

    #[test]
    fn text_renders_and_clips_to_width() {
        let mut buf = buffer(6, 2);
        let text = Text::new(vec![Line::from("hello world"), Line::from("hi")]);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        text.render(area, &mut surface, &ctx);
        // Clipped to 6 columns ("hello " with a trailing space, which `row` trims).
        assert_eq!(row(&buf, 0), "hello");
        assert_eq!(row(&buf, 1), "hi");
    }

    #[test]
    fn text_composes_line_style_under_span_style() {
        let line = Line::from(vec![
            Span::raw("a"),
            Span::styled("b", Style::default().fg(Color::Blue)),
        ])
        .style(Style::default().fg(Color::Red).bold());
        let buf = crate::testing::render(&Text::new(vec![line]), 2, 1, &Theme::default());
        assert_eq!(buf[(0, 0)].fg, Color::Red);
        assert_eq!(buf[(1, 0)].fg, Color::Blue);
        assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn paragraph_wraps_to_width() {
        let p = Paragraph::new("the quick brown fox", Style::default());
        let theme = Theme::default();
        let size = p.measure(Size::new(10, 10), &RenderCtx::new(&theme));
        assert!(size.height >= 2, "expected wrap, got {size:?}");
        assert!(size.width <= 10);
    }

    #[test]
    fn paragraph_links_web_urls_by_default() {
        let theme = Theme::default();
        let buf = crate::testing::render(
            &Paragraph::new("see https://a.dev now", Style::default()),
            24,
            1,
            &theme,
        );
        assert_eq!(
            link_at(&buf, Rect::new(0, 0, 24, 1), 6, 0).as_deref(),
            Some("https://a.dev")
        );
        let linked_cell = &buf[(4, 0)];
        assert_eq!(linked_cell.fg, theme.code.link);
        assert!(linked_cell.modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn paragraph_link_policy_none_keeps_urls_literal() {
        let buf = crate::testing::render(
            &Paragraph::new("https://a.dev", Style::default())
                .link_policy(crate::term::hyperlink::LinkPolicy::NONE),
            20,
            1,
            &Theme::default(),
        );
        assert!(
            buf.content
                .iter()
                .all(|cell| !cell.symbol().contains("\x1b]8;;"))
        );
        assert!(!buf[(0, 0)].modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn paragraph_keeps_full_link_target_across_hard_wraps() {
        let url = "https://example.com/very-long-path";
        let buf = crate::testing::render(
            &Paragraph::new(url, Style::default()),
            10,
            4,
            &Theme::default(),
        );
        for row in 0..4 {
            assert_eq!(
                link_at(&buf, Rect::new(0, 0, 10, 4), 1, row).as_deref(),
                Some(url),
                "wrapped row {row} should resolve the complete URL"
            );
        }
    }

    #[test]
    fn paragraph_aligns_link_metadata_with_visible_text() {
        let url = "https://a.dev";
        let buf = crate::testing::render(
            &Paragraph::new(url, Style::default()).alignment(Alignment::Center),
            20,
            1,
            &Theme::default(),
        );
        assert_eq!(
            link_at(&buf, Rect::new(0, 0, 20, 1), 3, 0).as_deref(),
            Some(url)
        );
        assert_eq!(link_at(&buf, Rect::new(0, 0, 20, 1), 0, 0), None);
    }

    #[test]
    fn paragraph_links_survive_degenerate_sizes() {
        let paragraph = Paragraph::new(
            "read https://example.com/a-very-long-path",
            Style::default(),
        );
        let sizes = (0..=20).flat_map(|width| (0..=4).map(move |height| (width, height)));
        assert_eq!(
            crate::testing::render_sizes(&paragraph, sizes, &Theme::default()).len(),
            105
        );
    }

    #[test]
    fn paragraph_measures_the_width_it_wrapped_to() {
        // `wrap_str` and `measure` must count columns the same way. A ZWJ family
        // is one double-width cell to tuika but six scalars wide to a per-`char`
        // width model, so a wrapper that disagreed would break early and report
        // a height taller than the text needs.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let text = format!("{family} {family} {family}");
        let theme = Theme::default();
        let p = Paragraph::new(text, Style::default());
        // Three 2-column clusters plus two joining spaces = 8 columns.
        let size = p.measure(Size::new(8, 4), &RenderCtx::new(&theme));
        assert_eq!(size, Size::new(8, 1), "all three should fit on one row");
        let size = p.measure(Size::new(5, 4), &RenderCtx::new(&theme));
        assert_eq!(size, Size::new(5, 2), "two rows: 2+1+2 columns, then 2");
    }

    #[test]
    fn paragraph_breaks_only_at_whitespace_and_collapses_runs() {
        // `Paragraph` now wraps on the same solver as `Wrap`, so it agrees with
        // it on two things textwrap decided differently: a run of whitespace is
        // one break opportunity (not copied through verbatim), and `/` is not
        // one — which is what keeps a linkified URL contiguous instead of split
        // after its scheme.
        let rows = wrap_str("see  https://a.dev  now", 13);
        assert_eq!(rows[0].text, "see");
        assert_eq!(rows[1].text, "https://a.dev");
        assert_eq!(rows[2].text, "now");
        let buf = crate::testing::render(
            &Paragraph::new("see  https://a.dev  now", Style::default()),
            13,
            3,
            &Theme::default(),
        );
        assert_eq!(
            link_at(&buf, Rect::new(0, 0, 13, 3), 0, 1).as_deref(),
            Some("https://a.dev"),
            "the whole URL stays on one row and resolves as one link"
        );
    }

    #[test]
    fn paragraph_links_each_repeated_url_on_its_own_row() {
        let buf = crate::testing::render(
            &Paragraph::new("go https://a.dev go https://b.dev", Style::default()),
            16,
            2,
            &Theme::default(),
        );
        let area = Rect::new(0, 0, 16, 2);
        assert_eq!(link_at(&buf, area, 4, 0).as_deref(), Some("https://a.dev"));
        assert_eq!(link_at(&buf, area, 4, 1).as_deref(), Some("https://b.dev"));
    }

    #[test]
    fn wrap_str_maps_every_word_back_to_its_source_bytes() {
        let src = "alpha  beta gamma";
        let rows = wrap_str(src, 11);
        assert_eq!(rows.len(), 2);
        // Repeated whitespace collapses in the output text...
        assert_eq!(rows[0].text, "alpha beta");
        assert_eq!(rows[1].text, "gamma");
        // ...but each word still points at the bytes it was copied from.
        for row in &rows {
            for (row_start, src_range) in &row.words {
                let word = &row.text[*row_start..row_start + src_range.len()];
                assert_eq!(word, &src[src_range.clone()]);
            }
        }
    }

    #[test]
    fn wrap_str_keeps_a_blank_line_blank() {
        // One empty row per blank source line is what lets a caller splitting on
        // '\n' preserve a deliberate gap between paragraphs.
        assert_eq!(wrap_str("", 10).len(), 1);
        assert_eq!(wrap_str("", 10)[0].text, "");
        assert_eq!(wrap_str("   ", 10).len(), 1);
        assert_eq!(wrap_str("   ", 10)[0].text, "");
    }

    #[test]
    fn wrapping_at_max_content_width_does_not_overflow_the_column_counter() {
        // `Availability::MaxContent` measures at `u16::MAX`, so prose long
        // enough to exceed 65_535 columns on one row — a pasted log line, a
        // generated markdown paragraph — reaches the wrap solver's column
        // arithmetic at its limit. Untrusted text must degrade, never panic.
        let long = vec!["aaaaaaaa"; 9000].join(" ");
        let rows = wrap_str(&long, u16::MAX);
        assert_eq!(rows.len(), 1, "everything fits at max-content width");
        let one_word_per_gap = long.split(' ').count();
        assert_eq!(rows[0].words.len(), one_word_per_gap);

        // The styled path shares the solver, so it is covered by the same fix.
        let out = wrap_lines(&[Line::from(long)], u16::MAX);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn wrap_str_hard_breaks_an_overlong_word_without_exceeding_the_width() {
        let rows = wrap_str("short supercalifragilistic", 8);
        assert!(
            rows.iter().all(|r| str_cols(&r.text) <= 8),
            "no row may exceed the width: {:?}",
            rows.iter().map(|r| &r.text).collect::<Vec<_>>()
        );
        let joined: String = rows.iter().map(|r| r.text.replace(' ', "")).collect();
        assert_eq!(joined, "shortsupercalifragilistic");
    }

    #[test]
    fn wrap_lines_breaks_on_word_boundaries() {
        let out = wrap_lines(&[Line::from("the quick brown fox jumps")], 9);
        assert!(
            out.iter().all(|l| line_width(l) <= 9),
            "no output line may exceed the width: {out:?}"
        );
        // Every word survives, in order, un-split.
        let words: Vec<String> = out
            .iter()
            .flat_map(|l| {
                line_text(l)
                    .split_whitespace()
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(words, ["the", "quick", "brown", "fox", "jumps"]);
    }

    #[test]
    fn wrap_lines_preserves_span_styles() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let line = Line::from(vec![
            Span::styled("red", red),
            Span::raw(" "),
            Span::styled("blue", blue),
        ]);
        // Wide enough that nothing wraps.
        let out = wrap_lines(&[line], 40);
        assert_eq!(out.len(), 1);
        let spans = &out[0].spans;
        assert!(
            spans
                .iter()
                .any(|s| s.content.starts_with("red") && s.style.fg == Some(Color::Red)),
            "red run lost its style: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| s.content.contains("blue") && s.style.fg == Some(Color::Blue)),
            "blue run lost its style: {spans:?}"
        );
    }

    #[test]
    fn wrap_lines_style_survives_a_break() {
        let accent = Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED);
        // "aaaa bbbb", all accent, width 4 -> two lines, both still accent.
        let out = wrap_lines(&[Line::from(Span::styled("aaaa bbbb", accent))], 4);
        assert_eq!(out.len(), 2, "{out:?}");
        for l in &out {
            assert!(
                l.spans.iter().all(|s| s.style.fg == Some(Color::Blue)
                    && s.style.add_modifier.contains(Modifier::UNDERLINED)),
                "wrapped line dropped styling: {l:?}"
            );
        }
    }

    #[test]
    fn wrap_lines_hard_breaks_overlong_word() {
        let word = "x".repeat(20);
        let out = wrap_lines(&[Line::from(word.clone())], 8);
        assert!(out.len() >= 3, "a 20-col word at width 8 needs >=3 lines");
        assert!(out.iter().all(|l| line_width(l) <= 8));
        let joined: String = out.iter().map(|l| line_text(l)).collect();
        assert_eq!(joined, word, "hard-break must not lose characters");
    }

    #[test]
    fn wrap_lines_counts_wide_glyphs() {
        // Each CJK glyph is 2 columns; no whitespace, so it hard-breaks at width 4.
        let out = wrap_lines(&[Line::from("你好世界")], 4);
        assert!(out.iter().all(|l| line_width(l) <= 4), "{out:?}");
        let joined: String = out.iter().map(|l| line_text(l)).collect();
        assert_eq!(joined, "你好世界");
    }

    #[test]
    fn wrap_lines_keeps_emoji_clusters_intact() {
        // "❤️" carries VS16 → width 2. A grapheme must never be split mid-cluster
        // by the wrapper, and each output line must respect the width budget.
        let out = wrap_lines(&[Line::from("❤\u{FE0F} 你 ok")], 4);
        assert!(out.iter().all(|l| line_width(l) <= 4), "{out:?}");
        let joined: String = out.iter().map(|l| line_text(l)).collect();
        assert!(
            joined.contains("❤\u{FE0F}"),
            "heart+VS16 survived: {joined:?}"
        );
    }

    #[test]
    fn wrap_lines_keeps_blank_lines() {
        let lines = vec![Line::from("a"), Line::from(""), Line::from("b")];
        let out = wrap_lines(&lines, 10);
        assert_eq!(
            out.len(),
            3,
            "a blank line must stay one blank row: {out:?}"
        );
        assert_eq!(line_text(&out[1]), "");
    }

    #[test]
    fn wrap_lines_zero_width_is_identity() {
        let out = wrap_lines(&[Line::from("hello world")], 0);
        assert_eq!(out.len(), 1);
        assert_eq!(line_text(&out[0]), "hello world");
    }

    #[test]
    fn text_honors_line_alignment() {
        let mut buf = buffer(7, 3);
        let text = Text::new(vec![
            Line::from("ab"),
            Line::from("cd").centered(),
            Line::from("ef").right_aligned(),
        ]);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        text.render(area, &mut surface, &ctx);
        assert_eq!(row(&buf, 0), "ab", "unset alignment is flush-left");
        // width 7, content 2 -> slack 5, centered start = 5/2 = 2.
        assert_eq!(row(&buf, 1), "  cd", "centered line offset by slack/2");
        // right start = x + slack = 5.
        assert_eq!(row(&buf, 2), "     ef", "right-aligned pins to right edge");
    }

    #[test]
    fn paragraph_alignment_positions_each_wrapped_line() {
        // "aa bb" at width 6 stays one line; center slack = 1, start col 0 (1/2).
        let mut buf = buffer(6, 2);
        let p = Paragraph::new("aa bb", Style::default()).alignment(Alignment::Right);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        p.render(area, &mut surface, &ctx);
        // width 6, content 5 -> slack 1, right start = 1.
        assert_eq!(row(&buf, 0), " aa bb");
    }

    #[test]
    fn wrap_carries_alignment_onto_reflowed_rows() {
        // A right-aligned line wide enough to wrap keeps its alignment per row.
        let out = wrap_lines(&[Line::from("aa bb cc").right_aligned()], 5);
        assert!(out.len() >= 2, "expected a wrap: {out:?}");
        assert!(
            out.iter().all(|l| l.alignment == Some(Alignment::Right)),
            "every reflowed row keeps the source alignment: {out:?}"
        );
    }

    #[test]
    fn wrap_component_renders_reflowed() {
        // "aa bb cc" at width 5 wraps to "aa bb" / "cc".
        let mut buf = buffer(5, 3);
        let w = Wrap::new(vec![Line::from("aa bb cc")]);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let area = buf.area;
        let mut surface = Surface::new(&mut buf, area);
        w.render(area, &mut surface, &ctx);
        assert_eq!(row(&buf, 0), "aa bb");
        assert_eq!(row(&buf, 1), "cc");
    }
}
