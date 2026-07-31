//! Parsing, and the tag vocabulary the layout passes ask about.
//!
//! html5ever does the part that is genuinely hard about HTML — implied end tags
//! (`<li>a<li>b`), the `<tbody>` a table gets whether or not it was written,
//! recovery from malformed input — so everything downstream can walk a
//! well-formed tree instead of guessing.

use html5ever::tendril::TendrilSink;
use html5ever::{QualName, local_name, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Parse a fragment (not a document): the input is a run of markup from inside
/// a markdown file or a `<div>`, never a whole page with `<html>` around it.
pub(crate) fn parse(source: &str) -> Handle {
    let context = QualName::new(None, ns!(html), local_name!("div"));
    let dom =
        parse_fragment(RcDom::default(), Default::default(), context, vec![], false).one(source);
    dom.document
}

/// The lowercased tag name of an element node.
pub(crate) fn tag(node: &Handle) -> Option<String> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.to_string()),
        _ => None,
    }
}

/// One attribute's value, by lowercase name.
pub(crate) fn attr(node: &Handle, wanted: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|a| a.name.local.eq_str_ignore_ascii_case(wanted))
        .map(|a| a.value.to_string())
}

/// Elements dropped whole, content included.
///
/// Two kinds: markup that carries no text a terminal can show (`<script>`,
/// `<style>`, the document head), and embedded objects whose *content* is a
/// fallback for a medium we do not have. Printing either would put noise —
/// or a page's entire stylesheet — into a transcript.
pub(crate) fn is_dropped(tag: &str) -> bool {
    matches!(
        tag,
        "script"
            | "style"
            | "head"
            | "meta"
            | "link"
            | "title"
            | "base"
            | "template"
            | "noscript"
            | "iframe"
            | "object"
            | "embed"
            | "canvas"
            | "svg"
            | "math"
            | "audio"
            | "video"
    )
}

/// Elements that start a new line box. Everything else is inline, and inline
/// content accumulates into the enclosing block.
pub(crate) fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "center"
            | "dd"
            | "details"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "html"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "ul"
    )
}

/// HTML elements that never nest: they have no end tag, so a run of them is
/// wide, not deep.
const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// The deepest element nesting `source` opens, counted without building a tree.
///
/// This exists because the *parser* recurses. html5ever builds an arbitrarily
/// deep tree for arbitrarily deep markup, and both that construction and the
/// recursive drop of the resulting `Rc` chain overflow the stack well inside the
/// input-size bound — a 140 KiB fragment of nested `<b>` is enough. Capping the
/// walk cannot help: the crash happens before any walk begins. So the depth is
/// measured on the bytes first, and over-nested input is refused rather than
/// parsed.
///
/// Approximate by design, and conservative in the safe direction: it counts tag
/// opens and closes, skips void elements, self-closing tags, comments, and
/// doctypes, and ignores `<` inside quoted attribute values. A miscount can
/// only refuse a fragment that would have rendered, never admit one that
/// crashes.
pub(crate) fn max_depth(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut depth: usize = 0;
    let mut deepest = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i + 1..];
        // Comments, doctypes, CDATA, processing instructions: no nesting — and
        // their *contents* are not markup, so skip past them rather than
        // resuming inside, where `<!-- <div><div> -->` would read as depth 2.
        if rest.starts_with("!--") {
            i = source[i..]
                .find("-->")
                .map_or(bytes.len(), |end| i + end + 3);
            continue;
        }
        if rest.starts_with('!') || rest.starts_with('?') {
            i = source[i..].find('>').map_or(bytes.len(), |end| i + end + 1);
            continue;
        }
        let closing = rest.starts_with('/');
        let name_start = if closing { 1 } else { 0 };
        let name: String = rest[name_start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            i += 1;
            continue;
        }
        // Walk to the tag's `>`, tracking quotes so an attribute value carrying
        // `<` or `>` cannot shift the count.
        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        let mut self_closing = false;
        while j < bytes.len() {
            match (quote, bytes[j]) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, c @ (b'"' | b'\'')) => quote = Some(c),
                (None, b'>') => break,
                (None, b'/') => self_closing = true,
                (None, c) if !c.is_ascii_whitespace() => self_closing = false,
                (None, _) => {}
            }
            j += 1;
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else if !self_closing && !VOID.contains(&name.as_str()) {
            depth += 1;
            deepest = deepest.max(depth);
        }
        i = j + 1;
    }
    deepest
}

/// Text with control bytes removed.
///
/// A diagram fence, a code sample, or a hostile document can carry raw escape
/// bytes; they must never reach the terminal as cells. Tabs become spaces so
/// verbatim `<pre>` keeps its shape without a terminal tab stop deciding it.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '\t' => Some(' '),
            '\n' => Some('\n'),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Collapse HTML whitespace: any run of spaces, tabs, or newlines is one space.
pub(crate) fn collapse(text: &str) -> String {
    // Leading and trailing whitespace collapse to one space rather than being
    // dropped: it still separates this run from the text around it, and only the
    // caller knows whether the line is empty enough for it to matter.
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
    }
    if space {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_recovers_implied_end_tags() {
        let root = parse("<ul><li>a<li>b</ul>");
        let mut names = Vec::new();
        collect_tags(&root, &mut names);
        assert_eq!(names.iter().filter(|t| *t == "li").count(), 2, "{names:?}");
    }

    #[test]
    fn parsing_inserts_the_table_body_section() {
        let root = parse("<table><tr><td>x</td></tr></table>");
        let mut names = Vec::new();
        collect_tags(&root, &mut names);
        assert!(names.contains(&"tbody".to_string()), "{names:?}");
    }

    #[test]
    fn sanitize_drops_escape_bytes_and_keeps_shape() {
        assert_eq!(sanitize("a\u{1b}[31mb\tc\nd"), "a[31mb c\nd");
    }

    #[test]
    fn depth_counts_nesting_without_building_a_tree() {
        assert_eq!(max_depth("<div><p>x</p></div>"), 2);
        assert_eq!(max_depth("plain text"), 0);
        // Siblings are wide, not deep.
        assert_eq!(max_depth("<p>a</p><p>b</p>"), 1);
    }

    #[test]
    fn depth_ignores_void_and_self_closing_elements() {
        assert_eq!(max_depth(&"<br>".repeat(100)), 0);
        assert_eq!(max_depth(&"<img src=x>".repeat(100)), 0);
        assert_eq!(max_depth("<div/><div/>"), 0);
    }

    #[test]
    fn depth_ignores_markup_inside_attribute_values() {
        // A `<` in a quoted value must not open a level, or ordinary markup
        // would be refused.
        assert_eq!(max_depth(r#"<div title="<b>a</b>">x</div>"#), 1);
        assert_eq!(max_depth("<!-- <div><div> --><p>x</p>"), 1);
    }

    #[test]
    fn depth_survives_unbalanced_markup() {
        assert_eq!(max_depth("</div></div>"), 0);
        assert_eq!(max_depth("<div><div>"), 2);
        assert_eq!(max_depth("<"), 0);
    }

    #[test]
    fn collapse_folds_whitespace_runs() {
        assert_eq!(collapse("a \n\t b"), "a b");
        assert_eq!(collapse(" lead"), " lead");
        assert_eq!(collapse("trail "), "trail ");
    }

    fn collect_tags(node: &Handle, out: &mut Vec<String>) {
        if let Some(name) = tag(node) {
            out.push(name);
        }
        for child in node.children.borrow().iter() {
            collect_tags(child, out);
        }
    }
}
