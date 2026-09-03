//! Inline HTML: a fixed whitelist of presentational tags, mapped to style roles.
//!
//! Markdown in the wild carries HTML — `<b>`, `<kbd>`, `<br>`, `<a>`, `<img>` —
//! and pulldown-cmark hands it back verbatim as [`Event::InlineHtml`]. Dropping
//! those events (what this module replaced) is lossy in both directions: the
//! markup vanishes *and* so does its meaning, so `a<br>b` silently joins two
//! lines and `<b>bold</b>` renders flat.
//!
//! What is deliberately *not* here is an HTML parser. This is a tag-name
//! whitelist over the raw string pulldown already isolated: every recognized tag
//! resolves to a [`StyleSheet`](crate::style::StyleSheet) role the surrounding
//! markdown already uses, so a host that restyles `strong` restyles `<b>` with
//! it. Anything outside the whitelist — attributes we do not read, scripts,
//! block structure — is dropped exactly as before. Block-level HTML
//! ([`Event::Html`]) is a separate concern and stays dropped; see
//! `knowledge/specs/markdown.md`.
//!
//! [`Event::InlineHtml`]: pulldown_cmark::Event::InlineHtml
//! [`Event::Html`]: pulldown_cmark::Event::Html

use crate::style::Modifier;

use crate::style::Role;

/// Deepest inline-HTML nesting tracked. Beyond this, opening tags are ignored
/// (their text still renders) so untrusted markup cannot grow the scope stack
/// without bound.
pub(super) const MAX_NESTING: usize = 32;

/// What one raw inline-HTML tag means to the renderer.
pub(super) enum Effect {
    /// Open a style scope, closed by the matching end tag.
    Open(Scope),
    /// Close the innermost scope opened by this tag name.
    Close(&'static str),
    /// `<br>`: end the current line.
    Break,
    /// `<img>`: same treatment as a markdown `![alt](src)`.
    Image { src: String, alt: String },
    /// Recognized but presentationally transparent (`<span>`), or not
    /// whitelisted at all — either way, nothing to do.
    None,
}

/// The styling one open tag contributes, and the name that closes it.
pub(super) struct Scope {
    /// Canonical (lowercased) tag name, matched by the closing tag.
    pub(super) name: &'static str,
    /// Stylesheet role overlaid on the surrounding style, if any.
    pub(super) role: Option<Role>,
    /// Modifiers added on top for tags with no role of their own.
    pub(super) modifier: Modifier,
    /// Link destination for `<a href>`.
    pub(super) href: Option<String>,
    /// Unicode transliteration applied to the text inside.
    pub(super) script: Option<Script>,
}

impl Scope {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            role: None,
            modifier: Modifier::empty(),
            href: None,
            script: None,
        }
    }

    fn role(name: &'static str, role: Role) -> Self {
        Self {
            role: Some(role),
            ..Self::new(name)
        }
    }

    fn modifier(name: &'static str, modifier: Modifier) -> Self {
        Self {
            modifier,
            ..Self::new(name)
        }
    }
}

/// `<sub>` / `<sup>`, rendered as Unicode when every character has a form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Script {
    Sub,
    Sup,
}

impl Script {
    fn map(self, ch: char) -> Option<char> {
        let table = match self {
            // Digits and arithmetic only. Letter forms exist for part of the
            // alphabet (there is no superscript `q`), so including them would
            // make coverage depend on which letters a word happens to use —
            // `transliterate` is all-or-nothing precisely to avoid that.
            Script::Sub => "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎",
            Script::Sup => "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾",
        };
        let idx = "0123456789+-=()".find(ch)?;
        table.chars().nth(idx)
    }
}

/// Transliterate `text` into `script`, or `None` if any character lacks a form.
///
/// All-or-nothing on purpose: a partial mapping (`x₂ + y2`) is worse than none,
/// and the caller falls back to rendering the text unchanged.
pub(super) fn transliterate(text: &str, script: Script) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    text.chars().map(|c| script.map(c)).collect()
}

/// Classify one raw inline-HTML tag (`<b>`, `</a>`, `<br/>`, `<img src=…>`).
pub(super) fn classify(raw: &str) -> Effect {
    let inner = raw
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(raw)
        .trim();
    // Comments, doctypes, CDATA, processing instructions: never presentational.
    if inner.starts_with('!') || inner.starts_with('?') {
        return Effect::None;
    }
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, inner),
    };
    let name_len = inner
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(inner.len());
    let name = inner[..name_len].to_ascii_lowercase();
    let attrs = &inner[name_len..];

    if closing {
        return match canonical(&name) {
            Some(name) => Effect::Close(name),
            None => Effect::None,
        };
    }

    match name.as_str() {
        "br" => Effect::Break,
        "img" => Effect::Image {
            src: attr(attrs, "src").unwrap_or_default(),
            alt: attr(attrs, "alt").unwrap_or_default(),
        },
        "a" => Effect::Open(Scope {
            role: Some(Role::Link),
            // A named anchor (`<a name=…>`) has no destination; it still opens a
            // scope so its `</a>` unwinds cleanly, just without a link.
            href: attr(attrs, "href"),
            ..Scope::new("a")
        }),
        "b" | "strong" => Effect::Open(Scope::role("strong", Role::Strong)),
        "i" | "em" | "var" | "cite" | "dfn" => Effect::Open(Scope::role("em", Role::Emphasis)),
        "code" | "kbd" | "samp" | "tt" => Effect::Open(Scope::role("code", Role::InlineCode)),
        "s" | "del" | "strike" => Effect::Open(Scope::role("del", Role::Strikethrough)),
        "u" | "ins" => Effect::Open(Scope::modifier("u", Modifier::UNDERLINED)),
        // No highlight role exists, and inventing one would add a stylesheet slot
        // for a single tag; reverse video is the terminal's own emphasis.
        "mark" => Effect::Open(Scope::modifier("mark", Modifier::REVERSED)),
        "sub" => Effect::Open(Scope {
            script: Some(Script::Sub),
            ..Scope::new("sub")
        }),
        "sup" => Effect::Open(Scope {
            script: Some(Script::Sup),
            ..Scope::new("sup")
        }),
        _ => Effect::None,
    }
}

/// The scope name a closing tag unwinds, for tags that share one (`<i>`/`<em>`).
fn canonical(name: &str) -> Option<&'static str> {
    Some(match name {
        "b" | "strong" => "strong",
        "i" | "em" | "var" | "cite" | "dfn" => "em",
        "code" | "kbd" | "samp" | "tt" => "code",
        "s" | "del" | "strike" => "del",
        "u" | "ins" => "u",
        "mark" => "mark",
        "sub" => "sub",
        "sup" => "sup",
        "a" => "a",
        _ => return None,
    })
}

/// Read one attribute value out of a raw tag's attribute text.
///
/// Enough HTML to read `href`, `src`, and `alt`: quoted or bare values, with
/// character references decoded (raw HTML reaches us verbatim, so `&amp;` in a
/// URL is still escaped where markdown's own destinations are not).
fn attr(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    while let Some(at) = rest.to_ascii_lowercase().find(name) {
        let before_ok = at == 0
            || rest[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || c == '"' || c == '\'');
        let after = rest[at + name.len()..].trim_start();
        if before_ok && let Some(value) = after.strip_prefix('=') {
            let value = value.trim_start();
            let raw = match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
                _ => {
                    let end = value
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(value.len());
                    &value[..end]
                }
            };
            return Some(unescape(raw));
        }
        rest = &rest[at + name.len()..];
    }
    None
}

/// Decode the character references an attribute value can carry.
fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        // A reference is short; anything longer is a literal `&`.
        let end = tail[1..].find(';').map(|i| i + 2).filter(|&e| e <= 12);
        let decoded = end.and_then(|end| match &tail[1..end - 1] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            num => {
                let digits = num.strip_prefix('#')?;
                let code = match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => digits.parse().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (decoded, end) {
            (Some(c), Some(end)) => {
                out.push(c);
                rest = &tail[end..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(raw: &str) -> Scope {
        match classify(raw) {
            Effect::Open(scope) => scope,
            _ => panic!("expected an opening scope for {raw:?}"),
        }
    }

    #[test]
    fn aliases_share_one_closing_name() {
        // `<i>x</em>` is malformed but common; both spellings must unwind the
        // same scope or the style leaks to the end of the block.
        assert_eq!(scope("<i>").name, "em");
        assert_eq!(scope("<em>").name, "em");
        assert!(matches!(classify("</i>"), Effect::Close("em")));
        assert!(matches!(classify("</em>"), Effect::Close("em")));
    }

    #[test]
    fn tag_names_are_case_insensitive_and_tolerate_whitespace() {
        assert_eq!(scope("<STRONG>").name, "strong");
        assert!(matches!(classify("</ B >"), Effect::Close("strong")));
        assert!(matches!(classify("<br />"), Effect::Break));
    }

    #[test]
    fn unknown_and_non_element_tags_are_inert() {
        for raw in ["<script>", "<div>", "<!-- note -->", "<!DOCTYPE html>"] {
            assert!(matches!(classify(raw), Effect::None), "{raw}");
        }
    }

    #[test]
    fn anchor_reads_href_with_entities_decoded() {
        let scope = scope("<a class='x' href=\"/q?a=1&amp;b=2\">");
        assert_eq!(scope.href.as_deref(), Some("/q?a=1&b=2"));
        assert_eq!(scope.role, Some(Role::Link));
    }

    #[test]
    fn anchor_without_href_still_opens_a_scope() {
        assert_eq!(scope("<a name=top>").href, None);
    }

    #[test]
    fn image_reads_src_and_alt() {
        match classify("<img src=pic.png alt='a cat' width=10>") {
            Effect::Image { src, alt } => {
                assert_eq!(src, "pic.png");
                assert_eq!(alt, "a cat");
            }
            _ => panic!("expected an image"),
        }
    }

    #[test]
    fn attribute_lookup_does_not_match_a_suffix() {
        // `data-src` and `srcset` both contain "src"; neither is `src`.
        match classify("<img data-src=no srcset=also-no src=yes>") {
            Effect::Image { src, .. } => assert_eq!(src, "yes"),
            _ => panic!("expected an image"),
        }
    }

    #[test]
    fn transliteration_is_all_or_nothing() {
        assert_eq!(transliterate("2", Script::Sub).as_deref(), Some("₂"));
        assert_eq!(transliterate("-9", Script::Sup).as_deref(), Some("⁻⁹"));
        // No subscript form for letters, so the caller keeps the plain text.
        assert_eq!(transliterate("th", Script::Sup), None);
        assert_eq!(transliterate("", Script::Sub), None);
    }

    #[test]
    fn unescape_leaves_lone_ampersands_alone() {
        assert_eq!(unescape("a & b"), "a & b");
        assert_eq!(unescape("&#x41;&#66;&nope;"), "AB&nope;");
    }
}
