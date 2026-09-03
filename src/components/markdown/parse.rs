//! Pass one: CommonMark source to [`MdItem`]s.
//!
//! Everything pulldown-cmark reports — block structure, inline styling, links,
//! code fences, tables, images — is resolved here, at no particular width. The
//! output carries styles but not line breaks; [`flatten`](super::flatten) adds
//! those.

use crate::style::{Modifier, Style};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::components::code_block::code_block_rows;
use crate::highlight::CodeHighlighter;
use crate::style::{StyleBundle, StyleSheet, Theme};

use super::flatten::trim_spans;
use super::html::{self, Script};
use super::image::ImageResolver;
use super::item::{Cell, INDENT, MdItem, RichSpan, TableData};

/// Parse `source` into width-independent [`MdItem`]s. Fenced code blocks are
/// highlighted here (once), via `highlighter`, using `theme`'s code palette;
/// prose roles (headings, links, emphasis, …) are styled from `sheet`.
pub(super) fn parse(
    source: &str,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
) -> Vec<MdItem> {
    parse_with(source, theme, sheet, highlighter, None)
}

/// [`parse`] with an optional [`ImageResolver`]: a resolved `![alt](url)` becomes
/// a block [`MdItem::Image`] instead of the inline text placeholder.
pub(super) fn parse_with(
    source: &str,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    resolver: Option<&dyn ImageResolver>,
) -> Vec<MdItem> {
    let mut b = Builder::new(theme, sheet, highlighter, resolver);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    // Without this the parser never emits `TaskListMarker`, so the checkbox
    // handler below — and the `task_marker` stylesheet slot it reads — were
    // unreachable and `- [x] done` rendered as literal text.
    opts.insert(Options::ENABLE_TASKLISTS);
    for event in Parser::new_ext(source, opts) {
        b.event(event);
    }
    b.finish();
    b.items
}

/// Walks the pulldown-cmark event stream, accumulating [`MdItem`]s.
struct Builder<'a> {
    theme: &'a Theme,
    sheet: &'a StyleSheet,
    highlighter: CodeHighlighter<'a>,
    items: Vec<MdItem>,

    // Inline accumulation for the current prose block.
    inline: Vec<RichSpan>,
    style_stack: Vec<Style>,
    /// Destinations for open `[label](url)` tags (usually depth 1). Text pushed
    /// while this is non-empty inherits the innermost href.
    link_stack: Vec<String>,

    // Block nesting.
    lists: Vec<Option<u64>>, // ordered start counter per open list, `None` = bullet
    quote_depth: u16,
    pending_marker: Option<Vec<RichSpan>>,

    // Fenced/indented code block being collected.
    code: Option<(String, String)>, // (language tag, body)

    // Raw block-level HTML being collected, verbatim, between `HtmlBlock` tags.
    // pulldown-cmark ends an HTML block at a blank line, so one `<details>` with
    // blank lines inside arrives as several — that is the parser's framing, and
    // it is the same whether the source is streamed or rendered in one shot.
    html_block: Option<String>,

    // Table being collected. Cells reuse the shared `inline` accumulator, so
    // they pick up the same styling as prose; each cell drains `inline` on its
    // `TableCell` end. Header vs body rows are routed by which End event
    // (`TableHead` / `TableRow`) closes the accumulated cells.
    table: Option<TableData>,
    cur_row: Vec<Cell>,

    /// Scopes opened by inline HTML (`<b>`, `<a href>`, `<sub>`), innermost last.
    /// Each records how deep the markdown stacks were when it opened, so its end
    /// tag unwinds exactly what it pushed — and an unbalanced or stray tag can
    /// only fail to unwind, never corrupt the stacks. See [`html`](super::html).
    html: Vec<HtmlScope>,

    // Inline image being collected: (dest URL, alt-text accumulator). Set on
    // `Tag::Image`, drained on `TagEnd::Image` into a placeholder or, when the
    // resolver yields pixels, a block `MdItem::Image`.
    image: Option<(String, String)>,

    // Host hook resolving image URLs to pixels; `None` keeps the text placeholder.
    resolver: Option<&'a dyn ImageResolver>,
}

/// One open inline-HTML scope: what closes it, and the stack depths to restore.
struct HtmlScope {
    name: &'static str,
    style_len: usize,
    link_len: usize,
    script: Option<Script>,
}

impl<'a> Builder<'a> {
    fn new(
        theme: &'a Theme,
        sheet: &'a StyleSheet,
        highlighter: CodeHighlighter<'a>,
        resolver: Option<&'a dyn ImageResolver>,
    ) -> Self {
        Self {
            theme,
            sheet,
            highlighter,
            items: Vec::new(),
            inline: Vec::new(),
            style_stack: vec![Style::default().fg(theme.text)],
            link_stack: Vec::new(),
            lists: Vec::new(),
            quote_depth: 0,
            pending_marker: None,
            code: None,
            html_block: None,
            table: None,
            cur_row: Vec::new(),
            html: Vec::new(),
            image: None,
            resolver,
        }
    }

    fn cur_style(&self) -> Style {
        *self.style_stack.last().unwrap()
    }

    fn cur_href(&self) -> Option<String> {
        self.link_stack.last().cloned()
    }

    /// Indentation for the current block, from list + quote nesting.
    fn indent(&self) -> u16 {
        (self.lists.len().saturating_sub(1) as u16 + self.quote_depth) * INDENT
    }

    /// True when we are inside a list or block quote (nested, not top level).
    fn nested(&self) -> bool {
        !self.lists.is_empty() || self.quote_depth > 0
    }

    /// Insert a blank spacer before a new top-level block (never inside a list
    /// or quote, and never doubling blanks).
    ///
    /// Opening a block always ends whatever inline run was in flight: in a
    /// *tight* list an item's text carries no `Paragraph` of its own, so a nested
    /// list, quote, or fence starts while the parent item is still buffered.
    /// Without the flush the child's first line is appended to the parent's
    /// ("• item onenested item"), or a fence lands ahead of the item it follows.
    /// Only real content is flushed — a bare pending marker stays pending so a
    /// loose item's first paragraph still gets its bullet.
    fn separate(&mut self) {
        if !self.inline.is_empty() {
            self.flush();
        }
        if self.nested() {
            return;
        }
        if matches!(self.items.last(), None | Some(MdItem::Blank)) {
            return;
        }
        self.items.push(MdItem::Blank);
    }

    /// Flush the current inline run as one prose line, attaching a pending list
    /// marker to the item's first line.
    fn flush(&mut self) {
        if self.inline.is_empty() && self.pending_marker.is_none() {
            return;
        }
        let indent = self.indent();
        let mut spans = self.pending_marker.take().unwrap_or_default();
        spans.append(&mut self.inline);
        self.items.push(MdItem::Prose { spans, indent });
    }

    fn push_text(&mut self, text: &str) {
        if let Some((_, body)) = self.code.as_mut() {
            body.push_str(text);
            return;
        }
        // Inside an image, text is the alt description — collect it for the
        // placeholder rather than emitting it as prose.
        if let Some((_, alt)) = self.image.as_mut() {
            alt.push_str(text);
            return;
        }
        // `<sub>`/`<sup>` content becomes Unicode when every character has a
        // form; otherwise it falls through and renders as ordinary text.
        if let Some(script) = self.cur_script()
            && let Some(mapped) = html::transliterate(text, script)
        {
            let style = self.cur_style();
            self.inline
                .push(RichSpan::styled(mapped, style, self.cur_href()));
            return;
        }
        let style = self.cur_style();
        let href = self.cur_href();
        // Only linkify bare URLs in plain body text, not inside links/headings.
        // Inside an explicit link the destination is already on `href`.
        if href.is_some() || style.add_modifier.contains(Modifier::UNDERLINED) {
            self.inline
                .push(RichSpan::styled(text.to_string(), style, href));
        } else {
            for span in linkify(text, style, self.sheet.link) {
                self.inline.push(span);
            }
        }
    }

    /// Render an inline image as a visible placeholder: a small marker glyph plus
    /// the alt text — or the URL when there is no alt — link-styled, so an image
    /// is never silently dropped. Actually painting the pixels in markdown
    /// (resolving the URL to [`ImageData`](crate::term::image::ImageData) and emitting
    /// through an [`ImageLayer`](crate::term::image::ImageLayer)) is a later phase; see
    /// `knowledge/specs/images.md`.
    fn push_image_placeholder(&mut self, url: &str, alt: &str) {
        let label = if alt.trim().is_empty() { url } else { alt };
        self.inline.push(RichSpan::styled(
            "🖼 ",
            self.sheet.image_marker.to_style(),
            None,
        ));
        // Placeholder label points at the image URL so Ctrl+click / OSC 8 can
        // still open it when the host applies buffer links.
        self.inline.push(RichSpan::styled(
            label.to_string(),
            self.sheet.link.to_style(),
            Some(url.to_string()),
        ));
    }

    /// The transliteration in force, from the innermost `<sub>`/`<sup>` scope.
    fn cur_script(&self) -> Option<Script> {
        self.html.iter().rev().find_map(|s| s.script)
    }

    /// Apply one raw inline-HTML tag. Everything outside the whitelist is
    /// dropped, which is what markdown did with all of it before.
    fn html_tag(&mut self, raw: &str) {
        // Inside a fence the tag is code; inside an image it is part of the alt
        // text. Both are already captured verbatim by `push_text`.
        if self.code.is_some() || self.image.is_some() {
            self.push_text(raw);
            return;
        }
        match html::classify(raw) {
            html::Effect::Open(scope) => {
                if self.html.len() >= html::MAX_NESTING {
                    return;
                }
                let style_len = self.style_stack.len();
                let link_len = self.link_stack.len();
                let mut style = self.cur_style();
                if let Some(role) = scope.role {
                    style = self.sheet.resolve(role).apply(style);
                }
                style = style.add_modifier(scope.modifier);
                self.style_stack.push(style);
                if let Some(href) = scope.href {
                    self.link_stack.push(href);
                }
                self.html.push(HtmlScope {
                    name: scope.name,
                    style_len,
                    link_len,
                    script: scope.script,
                });
            }
            // Unwind to the matching open tag, dropping any scope left open
            // inside it (`<b><i>x</b>`); a close with no open is ignored.
            html::Effect::Close(name) => {
                if let Some(at) = self.html.iter().rposition(|s| s.name == name) {
                    let scope = &self.html[at];
                    self.style_stack.truncate(scope.style_len);
                    self.link_stack.truncate(scope.link_len);
                    self.html.truncate(at);
                }
            }
            // Same split as a markdown hard break: a cell cannot become two
            // blocks, so there it collapses to a space.
            html::Effect::Break => {
                if self.table.is_some() {
                    self.inline
                        .push(RichSpan::styled(" ", self.cur_style(), self.cur_href()));
                } else {
                    self.flush();
                }
            }
            html::Effect::Image { src, alt } => self.push_image(&src, &alt),
            html::Effect::None => {}
        }
    }

    /// Close every open inline-HTML scope, restoring the markdown stacks.
    ///
    /// Called where a block of inline content ends. An unclosed `<b>` must not
    /// style the rest of the document — and must not *stream* differently than
    /// it renders in one shot: [`MarkdownState`](super::MarkdownState) caches at
    /// block boundaries, so a scope that survived one would style the tail only
    /// while the tail is still being re-parsed.
    fn close_html_scopes(&mut self) {
        if let Some(first) = self.html.first() {
            self.style_stack.truncate(first.style_len);
            self.link_stack.truncate(first.link_len);
            self.html.clear();
        }
    }

    /// Render `![alt](url)` — or `<img src alt>` — as pixels when the host's
    /// resolver has them, and as a visible placeholder otherwise.
    fn push_image(&mut self, url: &str, alt: &str) {
        match self.resolver.and_then(|r| r.resolve(url)) {
            // Resolved: promote to a block image on its own rows. Any inline run
            // so far is flushed first so the image stands apart from surrounding
            // text.
            Some(data) => {
                self.flush();
                let indent = self.indent();
                self.items.push(MdItem::Image {
                    data,
                    alt: alt.to_string(),
                    indent,
                });
            }
            None => self.push_image_placeholder(url, alt),
        }
    }

    fn event(&mut self, event: Event<'a>) {
        match event {
            Event::InlineHtml(raw) => self.html_tag(&raw),
            Event::Html(raw) => {
                if let Some(buf) = self.html_block.as_mut() {
                    buf.push_str(&raw);
                }
            }
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.push_text(&t),
            Event::Code(t) => {
                self.inline.push(RichSpan::styled(
                    t.to_string(),
                    self.sheet.inline_code.to_style(),
                    self.cur_href(),
                ));
            }
            Event::SoftBreak => {
                if self.code.is_none() {
                    self.inline
                        .push(RichSpan::styled(" ", self.cur_style(), self.cur_href()));
                }
            }
            // A hard break inside a cell can't split it into another block, so
            // it collapses to a space; elsewhere it flushes the prose line.
            Event::HardBreak if self.table.is_some() => {
                self.inline
                    .push(RichSpan::styled(" ", self.cur_style(), self.cur_href()))
            }
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.separate();
                self.items.push(MdItem::Prose {
                    spans: vec![RichSpan::styled(
                        "─".repeat(24),
                        self.sheet.rule.to_style(),
                        None,
                    )],
                    indent: self.indent(),
                });
            }
            Event::TaskListMarker(done) => {
                let glyph = if done { "[x] " } else { "[ ] " };
                self.inline.push(RichSpan::styled(
                    glyph.to_string(),
                    self.sheet.task_marker.to_style(),
                    None,
                ));
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'a>) {
        match tag {
            Tag::Paragraph => self.separate(),
            Tag::Heading { level, .. } => {
                self.separate();
                let mut style = self.sheet.heading.to_style();
                if level > HeadingLevel::H2 {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                self.style_stack.push(style);
            }
            Tag::BlockQuote(_) => {
                self.separate();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.separate();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.separate();
                self.html_block = Some(String::new());
            }
            Tag::List(start) => {
                self.separate();
                self.lists.push(start);
            }
            Tag::Item => {
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *self.lists.last_mut().unwrap() = Some(n.saturating_add(1));
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.pending_marker = Some(vec![RichSpan::styled(
                    marker,
                    self.sheet.list_marker.to_style(),
                    None,
                )]);
            }
            Tag::Emphasis => self.push_style_bundle(self.sheet.emphasis),
            Tag::Strong => self.push_style_bundle(self.sheet.strong),
            Tag::Strikethrough => self.push_style_bundle(self.sheet.strikethrough),
            Tag::Link { dest_url, .. } => {
                self.link_stack.push(dest_url.to_string());
                self.style_stack.push(self.sheet.link.to_style());
            }
            Tag::Image { dest_url, .. } => {
                // Capture the target; alt text accrues via `push_text` until the
                // matching `TagEnd::Image` renders the placeholder.
                self.image = Some((dest_url.to_string(), String::new()));
            }
            Tag::Table(aligns) => {
                self.separate();
                self.table = Some(TableData {
                    aligns,
                    header: Vec::new(),
                    rows: Vec::new(),
                });
            }
            Tag::TableHead => {
                self.cur_row.clear();
                // Header cells render in the heading style; push it as the cell
                // base so plain header text picks it up, while links and inline
                // code inside a header keep their own styling on top.
                self.style_stack.push(self.sheet.heading.to_style());
            }
            Tag::TableRow => self.cur_row.clear(),
            Tag::TableCell => self.inline.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        // Inline HTML never outlives the block it opened in; see
        // `close_html_scopes`. Ends that only close an *inline* markdown span
        // (emphasis, links) are excluded — an `<b>` may legitimately wrap one.
        if matches!(
            tag,
            TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::TableCell
                | TagEnd::BlockQuote(_)
        ) {
            self.close_html_scopes();
        }
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.style_stack.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if let Some((lang, body)) = self.code.take() {
                    self.push_code_block(lang, body);
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(source) = self.html_block.take()
                    && !source.trim().is_empty()
                {
                    let indent = self.indent();
                    self.items.push(MdItem::Html { source, indent });
                }
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link => {
                self.style_stack.pop();
                self.link_stack.pop();
            }
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    self.push_image(&url, &alt);
                }
            }
            TagEnd::TableCell => {
                let cell = trim_spans(std::mem::take(&mut self.inline));
                self.cur_row.push(cell);
            }
            TagEnd::TableHead => {
                self.style_stack.pop();
                if let Some(t) = self.table.as_mut() {
                    t.header = std::mem::take(&mut self.cur_row);
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(std::mem::take(&mut self.cur_row));
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    let indent = self.indent();
                    self.items.push(MdItem::Table { table, indent });
                }
            }
            _ => {}
        }
    }

    /// Push a style derived by overlaying `bundle` onto the current style — used
    /// for inline roles (emphasis, strong, strikethrough) that add a modifier
    /// (and possibly a color) on top of the surrounding text.
    fn push_style_bundle(&mut self, bundle: StyleBundle) {
        let s = bundle.apply(self.cur_style());
        self.style_stack.push(s);
    }

    fn finish(&mut self) {
        // A still-streaming table (open at end of input) never renders partially;
        // drop its in-progress cell rather than flushing it as stray prose.
        if self.table.is_some() {
            self.inline.clear();
        }
        // Close any dangling paragraph in truncated (still-streaming) input.
        self.flush();
        self.close_html_scopes();
        if let Some((lang, body)) = self.code.take() {
            self.push_code_block(lang, body);
        }
        // A still-open HTML block at end of input (truncated, still streaming)
        // is handed over as far as it got, exactly like a dangling fence.
        if let Some(source) = self.html_block.take()
            && !source.trim().is_empty()
        {
            let indent = self.indent();
            self.items.push(MdItem::Html { source, indent });
        }
    }

    fn push_code_block(&mut self, language: String, body: String) {
        let source = body.strip_suffix('\n').unwrap_or(&body).to_string();
        let lines: Vec<&str> = source.split('\n').collect();
        let fallback = code_block_rows(&language, &lines, self.theme, self.highlighter, true, None);
        self.items.push(MdItem::CodeBlock {
            language,
            source,
            fallback,
            indent: self.indent(),
        });
    }
}

/// Style bare `http(s)://` URLs in `text` with the `link` role, leaving the rest
/// at `base`. The link role is overlaid onto `base`, so a URL inside otherwise
/// plain prose keeps that prose's context and gains the link color + underline.
/// Each URL span also carries `href = Some(url)` so OSC 8 / Ctrl+click can open
/// it after wrapping.
pub(super) fn linkify(text: &str, base: Style, link: StyleBundle) -> Vec<RichSpan> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("http://").or_else(|| rest.find("https://")) {
        let (before, from) = rest.split_at(start);
        if !before.is_empty() {
            spans.push(RichSpan::styled(before.to_string(), base, None));
        }
        let raw = from.find(char::is_whitespace).unwrap_or(from.len());
        let end = from[..raw]
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']'])
            .len();
        let (url, after) = from.split_at(end.max(1));
        spans.push(RichSpan::styled(
            url.to_string(),
            link.apply(base),
            Some(url.to_string()),
        ));
        rest = after;
    }
    if !rest.is_empty() {
        spans.push(RichSpan::styled(rest.to_string(), base, None));
    }
    if spans.is_empty() {
        spans.push(RichSpan::styled(text.to_string(), base, None));
    }
    spans
}
