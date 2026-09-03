//! The block pass: a parsed tree to width-fitted lines.
//!
//! Block elements each know how to place themselves at an indent and a width;
//! everything else accumulates into the enclosing block's inline run and is
//! word-wrapped when that block closes. This is the same shape tuika's markdown
//! renderer uses, minus the width-independent middle form — HTML is not
//! streamed token by token, so there is no settled prefix to cache.

use tuika::components::text::wrap_lines;
use tuika::style::{StyleSheet, Theme};
use tuika::ui::{Line, Span};
use tuika::ui::{Modifier, Style};
use tuika::width::str_cols;

use markup5ever_rcdom::{Handle, NodeData};

use crate::Limits;
use crate::dom::{self, attr, tag};
use crate::inline::{InlineBuf, collect};
use crate::table;

/// Columns of indentation per level of list, quote, or `<details>` nesting.
pub(crate) const INDENT: u16 = 2;

/// One render of one fragment.
pub(crate) struct Layout<'a> {
    pub(crate) theme: &'a Theme,
    pub(crate) sheet: &'a StyleSheet,
    pub(crate) limits: Limits,
    out: Vec<Line<'static>>,
    /// Lines still allowed. Kept separately from `out.len()` because list items
    /// render into a detached buffer, so the length there is not the total.
    budget: usize,
    /// Set while opening a container whose first child must not be preceded by a
    /// blank line (a list item's own content, a `<details>` body). Consumed by
    /// the first [`blank`](Self::blank) call.
    suppress_blank: bool,
}

impl<'a> Layout<'a> {
    pub(crate) fn new(theme: &'a Theme, sheet: &'a StyleSheet, limits: Limits) -> Self {
        Self {
            theme,
            sheet,
            limits,
            out: Vec::new(),
            budget: limits.max_lines,
            suppress_blank: false,
        }
    }

    /// Render `node`'s children as blocks and return the finished lines.
    pub(crate) fn render(mut self, node: &Handle, width: u16) -> Vec<Line<'static>> {
        self.blocks(node, 0, width, 0);
        while self.out.last().is_some_and(is_blank) {
            self.out.pop();
        }
        self.out
    }

    pub(crate) fn base_style(&self) -> Style {
        Style::default().fg(self.theme.text)
    }

    fn push(&mut self, line: Line<'static>) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        self.out.push(line);
    }

    /// Separate this block from the previous one, never leading and never
    /// doubling.
    fn blank(&mut self) {
        if std::mem::take(&mut self.suppress_blank) {
            return;
        }
        if self.out.is_empty() || self.out.last().is_some_and(is_blank) {
            return;
        }
        self.push(Line::default());
    }

    /// Walk one container's children, splitting block elements out of the
    /// inline run they interrupt.
    pub(crate) fn blocks(&mut self, node: &Handle, indent: u16, width: u16, depth: usize) {
        if depth > self.limits.max_depth || self.budget == 0 {
            return;
        }
        let mut buf = InlineBuf::new();
        let style = self.base_style();
        for child in node.children.borrow().iter() {
            let name = tag(child).unwrap_or_default();
            if !name.is_empty() && dom::is_dropped(&name) {
                continue;
            }
            if !name.is_empty() && dom::is_block(&name) {
                self.flush(&mut buf, indent, width);
                self.block(child, &name, indent, width, depth);
            } else {
                collect(
                    child,
                    style,
                    self.theme,
                    self.sheet,
                    depth,
                    self.limits.max_depth,
                    &mut buf,
                );
            }
        }
        self.flush(&mut buf, indent, width);
    }

    /// Emit the accumulated inline run as a wrapped paragraph.
    fn flush(&mut self, buf: &mut InlineBuf, indent: u16, width: u16) {
        if buf.is_empty() {
            buf.take();
            return;
        }
        self.blank();
        let lines: Vec<Line<'static>> = buf
            .take()
            .into_iter()
            .map(|spans| Line::from(trim_edges(spans)))
            .collect();
        self.emit_wrapped(lines, indent, width);
    }

    fn emit_wrapped(&mut self, lines: Vec<Line<'static>>, indent: u16, width: u16) {
        let avail = width.saturating_sub(indent).max(1);
        for line in wrap_lines(&lines, avail) {
            self.push(prefix(indent, line));
        }
    }

    fn block(&mut self, node: &Handle, name: &str, indent: u16, width: u16, depth: usize) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let mut style = self.sheet.heading.apply(self.base_style());
                // Same convention as tuika's markdown: the deeper headings pick
                // up italic so six levels stay distinguishable in a terminal
                // that has only a handful of attributes.
                if matches!(name, "h3" | "h4" | "h5" | "h6") {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                self.inline_block(node, style, indent, width, depth);
            }
            "p" | "div" | "section" | "article" | "main" | "aside" | "header" | "footer"
            | "nav" | "figure" | "form" | "fieldset" | "hgroup" | "center" | "body" | "html" => {
                self.blocks(node, indent, width, depth + 1);
            }
            "address" | "figcaption" => {
                let style = self.sheet.emphasis.apply(self.base_style());
                self.inline_block(node, style, indent, width, depth);
            }
            "blockquote" => {
                self.blank();
                self.suppress_blank = true;
                self.blocks(node, indent + INDENT, width, depth + 1);
            }
            "ul" | "ol" => self.list(node, name == "ol", indent, width, depth),
            // A stray `<li>` outside any list still reads as an item.
            "li" => self.item(node, "• ".to_string(), indent, width, depth),
            "dl" => {
                self.blank();
                self.suppress_blank = true;
                self.blocks(node, indent, width, depth + 1);
            }
            "dt" => {
                let style = self.sheet.strong.apply(self.base_style());
                self.inline_block(node, style, indent, width, depth);
            }
            // A definition belongs to the term above it, so it hangs directly
            // under it rather than floating a blank line away.
            "dd" => {
                self.suppress_blank = true;
                self.blocks(node, indent + INDENT, width, depth + 1);
            }
            "pre" => self.pre(node, indent, width),
            "hr" => {
                self.blank();
                let cols = width.saturating_sub(indent).clamp(1, 24);
                self.push(prefix(
                    indent,
                    Line::from(Span::styled(
                        "─".repeat(cols as usize),
                        self.sheet.rule.apply(self.base_style()),
                    )),
                ));
            }
            "table" => {
                self.blank();
                let avail = width.saturating_sub(indent).max(1);
                for line in table::render(node, avail, self) {
                    self.push(prefix(indent, line));
                }
            }
            "details" => self.details(node, indent, width, depth),
            // `<summary>` outside a `<details>`: still a label for what follows.
            "summary" => {
                let style = self.sheet.strong.apply(self.base_style());
                self.inline_block(node, style, indent, width, depth);
            }
            _ => self.blocks(node, indent, width, depth + 1),
        }
    }

    /// A block whose children are all inline, rendered in one style.
    fn inline_block(&mut self, node: &Handle, style: Style, indent: u16, width: u16, depth: usize) {
        let mut buf = InlineBuf::new();
        for child in node.children.borrow().iter() {
            collect(
                child,
                style,
                self.theme,
                self.sheet,
                depth,
                self.limits.max_depth,
                &mut buf,
            );
        }
        self.flush(&mut buf, indent, width);
    }

    fn list(&mut self, node: &Handle, ordered: bool, indent: u16, width: u16, depth: usize) {
        self.blank();
        let mut n: u64 = attr(node, "start")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        for child in node.children.borrow().iter() {
            if tag(child).as_deref() != Some("li") {
                continue;
            }
            let marker = if ordered {
                let m = format!("{n}. ");
                n = n.saturating_add(1);
                m
            } else {
                "• ".to_string()
            };
            self.item(child, marker, indent, width, depth);
        }
    }

    /// One list item: its content is laid out on its own, then the marker rides
    /// the first line and the rest hang under it.
    fn item(&mut self, node: &Handle, marker: String, indent: u16, width: u16, depth: usize) {
        let hang = str_cols(&marker);
        let inner = self.detached(node, width.saturating_sub(indent + hang).max(1), depth + 1);
        let marker_style = self.sheet.list_marker.apply(self.base_style());
        for (row, line) in inner.into_iter().enumerate() {
            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            if row == 0 {
                spans.push(Span::styled(marker.clone(), marker_style));
            } else {
                spans.push(Span::raw(" ".repeat(hang as usize)));
            }
            spans.extend(line.spans);
            self.push(prefix(indent, Line::from(spans)));
        }
    }

    fn details(&mut self, node: &Handle, indent: u16, width: u16, depth: usize) {
        self.blank();
        let open = attr(node, "open").is_some();
        let children = node.children.borrow();
        let summary = children
            .iter()
            .find(|c| tag(c).as_deref() == Some("summary"));
        // A collapsed `<details>` still shows its summary — and the marker says
        // which way it was written, since a terminal cannot expand it.
        let marker = if open { "▾ " } else { "▸ " };
        let style = self.sheet.heading.apply(self.base_style());
        let mut buf = InlineBuf::new();
        if let Some(summary) = summary {
            for child in summary.children.borrow().iter() {
                collect(
                    child,
                    style,
                    self.theme,
                    self.sheet,
                    depth,
                    self.limits.max_depth,
                    &mut buf,
                );
            }
        }
        let mut head: Vec<Line<'static>> = buf
            .take()
            .into_iter()
            .map(|spans| Line::from(trim_edges(spans)))
            .collect();
        if head.is_empty() {
            head.push(Line::from(Span::styled("Details", style)));
        }
        let marker_span = Span::styled(marker.to_string(), self.sheet.list_marker.apply(style));
        let avail = width.saturating_sub(indent + str_cols(marker)).max(1);
        for (row, line) in wrap_lines(&head, avail).into_iter().enumerate() {
            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            spans.push(if row == 0 {
                marker_span.clone()
            } else {
                Span::raw("  ".to_string())
            });
            spans.extend(line.spans);
            self.push(prefix(indent, Line::from(spans)));
        }

        // The body is everything but the summary, indented under it.
        self.suppress_blank = true;
        let body_indent = indent + INDENT;
        let mut buf = InlineBuf::new();
        let base = self.base_style();
        for child in children.iter() {
            let name = tag(child).unwrap_or_default();
            if name == "summary" || (!name.is_empty() && dom::is_dropped(&name)) {
                continue;
            }
            if !name.is_empty() && dom::is_block(&name) {
                self.flush(&mut buf, body_indent, width);
                self.block(child, &name, body_indent, width, depth + 1);
            } else {
                collect(
                    child,
                    base,
                    self.theme,
                    self.sheet,
                    depth,
                    self.limits.max_depth,
                    &mut buf,
                );
            }
        }
        self.flush(&mut buf, body_indent, width);
    }

    /// `<pre>`: verbatim. Indentation is the content, so nothing is wrapped and
    /// nothing is collapsed — an over-wide line clips, exactly like a fenced
    /// code block in markdown.
    fn pre(&mut self, node: &Handle, indent: u16, width: u16) {
        self.blank();
        let text = dom::sanitize(&raw_text(node));
        let style = Style::default()
            .fg(self.theme.code.text)
            .bg(self.theme.code.background);
        let body = text.strip_prefix('\n').unwrap_or(&text);
        let body = body.strip_suffix('\n').unwrap_or(body);
        let avail = width.saturating_sub(indent).max(1) as usize;
        for line in body.split('\n') {
            // Pad to the width so the code background reads as a block rather
            // than as ragged highlighting behind the text.
            let mut content = line.to_string();
            let cols = str_cols(&content) as usize;
            if cols < avail {
                content.push_str(&" ".repeat(avail - cols));
            }
            self.push(prefix(indent, Line::from(Span::styled(content, style))));
        }
    }

    /// Lay out a subtree into its own line buffer, sharing this render's budget.
    fn detached(&mut self, node: &Handle, width: u16, depth: usize) -> Vec<Line<'static>> {
        let saved = std::mem::take(&mut self.out);
        let saved_suppress = std::mem::replace(&mut self.suppress_blank, true);
        self.blocks(node, 0, width, depth);
        self.suppress_blank = saved_suppress;
        std::mem::replace(&mut self.out, saved)
    }
}

/// All text under `node`, ignoring markup — `<pre>`'s content model.
fn raw_text(node: &Handle) -> String {
    let mut out = String::new();
    fn walk(node: &Handle, out: &mut String) {
        match &node.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            NodeData::Element { .. } => {
                for child in node.children.borrow().iter() {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }
    for child in node.children.borrow().iter() {
        walk(child, &mut out);
    }
    out
}

/// Drop the collapsed whitespace at a wrapped line's edges.
fn trim_edges(mut spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    if let Some(first) = spans.first_mut() {
        first.content = first.content.trim_start().to_string().into();
    }
    if let Some(last) = spans.last_mut() {
        last.content = last.content.trim_end().to_string().into();
    }
    spans.retain(|s| !s.content.is_empty());
    spans
}

/// Prefix a finished line with `indent` blank columns.
pub(crate) fn prefix(indent: u16, line: Line<'static>) -> Line<'static> {
    if indent == 0 {
        return line;
    }
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(indent as usize)));
    spans.extend(line.spans);
    Line::from(spans)
}

fn is_blank(line: &Line<'static>) -> bool {
    line.spans
        .iter()
        .all(|s| s.content.chars().all(char::is_whitespace))
}
