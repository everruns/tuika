//! The inline pass: text and phrasing elements to styled spans.
//!
//! Every tag resolves a [`StyleSheet`] role rather than a color, so HTML in a
//! host's transcript inherits the same look as the markdown around it — the
//! same rule tuika's own inline-HTML whitelist follows. The two implementations
//! are deliberately separate: tuika matches tag *strings* it never parses, while
//! this one walks a real tree and can see attributes and nesting.

use tuika::style::{StyleSheet, Theme};
use tuika::ui::Span;
use tuika::ui::{Modifier, Style};

use crate::dom::{self, attr, tag};
use markup5ever_rcdom::{Handle, NodeData};

/// Accumulated inline content for one block, already split at `<br>`.
#[derive(Default)]
pub(crate) struct InlineBuf {
    lines: Vec<Vec<Span<'static>>>,
}

impl InlineBuf {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines
            .iter()
            .all(|l| l.iter().all(|s| s.content.is_empty()))
    }

    /// Take the accumulated lines, leaving the buffer empty.
    pub(crate) fn take(&mut self) -> Vec<Vec<Span<'static>>> {
        std::mem::take(&mut self.lines)
    }

    fn cur(&mut self) -> &mut Vec<Span<'static>> {
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.lines.last_mut().expect("just ensured")
    }

    /// True when nothing has been written to the current line yet — used to drop
    /// the leading space HTML whitespace collapsing leaves behind.
    fn at_line_start(&self) -> bool {
        match self.lines.last() {
            None => true,
            Some(line) => line.iter().all(|s| s.content.is_empty()),
        }
    }

    fn push(&mut self, content: String, style: Style) {
        if content.is_empty() {
            return;
        }
        self.cur().push(Span::styled(content, style));
    }

    /// A `<br>`, or a block boundary inside otherwise inline content.
    pub(crate) fn hard_break(&mut self) {
        self.lines.push(Vec::new());
    }
}

/// `<sub>` / `<sup>`, rendered as Unicode when every character has a form.
///
/// The same rule tuika's own inline-HTML whitelist applies, and deliberately the
/// same table: a document must not render `H₂O` through markdown and `H2O`
/// through this crate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Script {
    Sub,
    Sup,
}

impl Script {
    fn map(self, ch: char) -> Option<char> {
        // Digits and arithmetic only. Letter forms cover part of the alphabet
        // (there is no superscript `q`), so including them would make coverage
        // depend on which letters a word happens to use.
        let table = match self {
            Script::Sub => "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎",
            Script::Sup => "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾",
        };
        table.chars().nth("0123456789+-=()".find(ch)?)
    }
}

/// Transliterate `text`, or `None` if any character lacks a form. All-or-nothing
/// on purpose: a partial mapping (`x₂ + y2`) is worse than none.
fn transliterate(text: &str, script: Script) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    text.chars().map(|c| script.map(c)).collect()
}

/// Collect one node's inline content into `buf`.
pub(crate) fn collect(
    node: &Handle,
    style: Style,
    theme: &Theme,
    sheet: &StyleSheet,
    depth: usize,
    max_depth: usize,
    buf: &mut InlineBuf,
) {
    collect_scoped(node, style, None, theme, sheet, depth, max_depth, buf);
}

#[allow(clippy::too_many_arguments)]
fn collect_scoped(
    node: &Handle,
    style: Style,
    script: Option<Script>,
    theme: &Theme,
    sheet: &StyleSheet,
    depth: usize,
    max_depth: usize,
    buf: &mut InlineBuf,
) {
    if depth > max_depth {
        return;
    }
    match &node.data {
        NodeData::Text { contents } => {
            let text = dom::collapse(&dom::sanitize(&contents.borrow()));
            let text = if buf.at_line_start() {
                text.trim_start().to_string()
            } else {
                text
            };
            // Inside `<sub>`/`<sup>`, text becomes Unicode when it fully maps;
            // otherwise it falls through and renders unchanged.
            let text = script
                .and_then(|script| transliterate(&text, script))
                .unwrap_or(text);
            buf.push(text, style);
        }
        NodeData::Element { .. } => {
            let name = tag(node).unwrap_or_default();
            if dom::is_dropped(&name) {
                return;
            }
            match name.as_str() {
                "br" => buf.hard_break(),
                // No resolver reaches this far — an HTML block is laid out to
                // lines, not to reserved cells — so an image is always its
                // alt-text placeholder here.
                "img" => {
                    let alt = attr(node, "alt").unwrap_or_default();
                    let src = attr(node, "src").unwrap_or_default();
                    let label = if alt.trim().is_empty() { src } else { alt };
                    if !label.is_empty() {
                        buf.push("🖼 ".to_string(), sheet.image_marker.apply(style));
                        buf.push(dom::sanitize(&label), sheet.link.apply(style));
                    }
                }
                "wbr" => {}
                _ => {
                    let inner = inline_style(&name, style, theme, sheet);
                    let script = match name.as_str() {
                        "sub" => Some(Script::Sub),
                        "sup" => Some(Script::Sup),
                        _ => script,
                    };
                    for child in node.children.borrow().iter() {
                        collect_scoped(
                            child,
                            inner,
                            script,
                            theme,
                            sheet,
                            depth + 1,
                            max_depth,
                            buf,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

/// The style an inline element contributes on top of its surroundings.
///
/// Unrecognized elements are transparent rather than dropped: a `<span>`, a
/// custom element, or a tag we have no styling for still shows its text.
pub(crate) fn inline_style(name: &str, base: Style, theme: &Theme, sheet: &StyleSheet) -> Style {
    match name {
        "b" | "strong" => sheet.strong.apply(base),
        "i" | "em" | "var" | "cite" | "dfn" | "address" | "figcaption" => {
            sheet.emphasis.apply(base)
        }
        "code" | "kbd" | "samp" | "tt" => sheet.inline_code.apply(base),
        "s" | "del" | "strike" => sheet.strikethrough.apply(base),
        "u" | "ins" => base.add_modifier(Modifier::UNDERLINED),
        // No highlight role exists in the stylesheet, and one tag does not earn
        // a slot; reverse video is the terminal's own highlighter.
        "mark" => base.add_modifier(Modifier::REVERSED),
        "a" => sheet.link.apply(base),
        "small" => base.fg(theme.dim),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::parse;

    fn spans(html: &str) -> Vec<(String, Style)> {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let mut buf = InlineBuf::new();
        let base = Style::default().fg(theme.text);
        // Parsing yields a document node; callers walk its children, as the
        // block pass does.
        for child in parse(html).children.borrow().iter() {
            collect(child, base, &theme, &sheet, 0, 32, &mut buf);
        }
        buf.take()
            .into_iter()
            .flatten()
            .map(|s| (s.content.to_string(), s.style))
            .collect()
    }

    fn text(html: &str) -> String {
        spans(html).into_iter().map(|(c, _)| c).collect()
    }

    #[test]
    fn whitespace_collapses_like_html() {
        assert_eq!(text("<p>a   \n  b</p>"), "a b");
    }

    #[test]
    fn nested_styles_compose() {
        let out = spans("<b>bold <i>and italic</i></b>");
        let italic = out
            .iter()
            .find(|(c, _)| c.contains("italic"))
            .expect("italic span");
        assert!(italic.1.add_modifier.contains(Modifier::BOLD));
        assert!(italic.1.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn dropped_elements_take_their_content_with_them() {
        assert_eq!(text("<p>keep<script>alert(1)</script></p>"), "keep");
        assert_eq!(text("<style>body{color:red}</style>"), "");
    }

    #[test]
    fn unknown_elements_stay_transparent() {
        assert_eq!(text("<custom-thing>visible</custom-thing>"), "visible");
    }

    #[test]
    fn images_fall_back_to_their_alt_text() {
        assert!(text("<img src=p.png alt='a cat'>").contains("a cat"));
        // No alt: the source is better than nothing.
        assert!(text("<img src=p.png>").contains("p.png"));
    }

    #[test]
    fn sub_and_sup_become_unicode_when_every_character_maps() {
        assert_eq!(text("H<sub>2</sub>O"), "H₂O");
        assert_eq!(text("2<sup>10</sup>"), "2¹⁰");
        assert_eq!(text("x<sup>-9</sup>"), "x⁻⁹");
        // No form for letters, so the text renders unchanged rather than partly
        // transliterated — the same rule tuika core applies.
        assert_eq!(text("4<sup>th</sup>"), "4th");
    }

    #[test]
    fn control_bytes_never_reach_a_span() {
        assert_eq!(text("<p>a\u{1b}[31mb</p>"), "a[31mb");
    }
}
