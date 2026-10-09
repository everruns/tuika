//! The [`Highlighter`] boundary — how a host plugs syntax highlighting into
//! [`CodeBlock`](crate::components::CodeBlock) and
//! [`Markdown`](crate::components::Markdown) without tuika taking on any grammar
//! dependency.
//!
//! tuika deliberately depends only on `ratatui-core`, `crossterm`, the
//! `unicode-*` crates, and `pulldown-cmark`; a real highlighter (tree-sitter,
//! syntect, …) pulls in
//! far more. So the toolkit owns only this trait and the *presentation* of code
//! (framing, background, language label, wrapping), while the host supplies the
//! token colors. The companion crate `tuika-codeformatters` ships a tree-sitter
//! implementation; hosts can also write their own.

use crate::text::Span;

use crate::style::Theme;
use std::cell::RefCell;
use std::collections::VecDeque;

type Highlighted = Option<Vec<Vec<Span<'static>>>>;

struct CachedBlock {
    lang: String,
    lines: Vec<String>,
    theme: Theme,
    result: Highlighted,
    bytes: usize,
}

/// A bounded, reusable cache around a host highlighter.
///
/// Hold this beside application state and pass it to `CodeBlock` or `Markdown`.
/// Requests above the source budget return `None`, preserving all code as plain
/// text. Cached successes and failures are keyed by exact source, language, and
/// theme; resizing therefore reuses syntax work. This bounds input and retained
/// storage, not elapsed time inside a host-provided synchronous highlighter.
pub struct CachedHighlighter<'a> {
    inner: &'a dyn Highlighter,
    max_source_bytes: usize,
    max_cache_bytes: usize,
    cache: RefCell<VecDeque<CachedBlock>>,
}

impl<'a> CachedHighlighter<'a> {
    /// Cache up to 4 MiB of estimated source/span storage across at most 16
    /// blocks (excluding allocator overhead); highlight at most 256 KiB of
    /// source in one request, including line separators.
    pub fn new(inner: &'a dyn Highlighter) -> Self {
        Self {
            inner,
            max_source_bytes: 256 * 1024,
            max_cache_bytes: 4 * 1024 * 1024,
            cache: RefCell::new(VecDeque::new()),
        }
    }

    /// Override source and cache byte budgets. Zero disables that operation.
    pub fn with_limits(mut self, max_source_bytes: usize, max_cache_bytes: usize) -> Self {
        self.max_source_bytes = max_source_bytes;
        self.max_cache_bytes = max_cache_bytes;
        self.cache.get_mut().clear();
        self
    }

    /// Forget results after the wrapped highlighter's configuration changes.
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
    }
}

impl Highlighter for CachedHighlighter<'_> {
    fn highlight(&self, lang: &str, lines: &[&str], theme: &Theme) -> Highlighted {
        let mut source_bytes = lines.len().saturating_sub(1);
        if source_bytes > self.max_source_bytes || self.max_source_bytes == 0 {
            return None;
        }
        for line in lines {
            source_bytes = source_bytes.saturating_add(line.len());
            if source_bytes > self.max_source_bytes {
                return None;
            }
        }
        {
            let mut cache = self.cache.borrow_mut();
            if let Some(index) = cache.iter().position(|entry| {
                entry.lang == lang
                    && entry.theme == *theme
                    && entry.lines.len() == lines.len()
                    && entry.lines.iter().zip(lines).all(|(a, b)| a == b)
            }) {
                let entry = cache.remove(index).expect("existing cache entry");
                let result = entry.result.clone();
                cache.push_back(entry);
                return result;
            }
        }
        // Never hold the cache borrow while calling host code.
        let result = self.inner.highlight(lang, lines, theme);
        let bytes = source_bytes
            .saturating_add(lang.len())
            .saturating_add(std::mem::size_of::<CachedBlock>())
            .saturating_add(lines.len().saturating_mul(std::mem::size_of::<String>()))
            .saturating_add(result.as_ref().map_or(0, |rows| {
                rows.capacity() * std::mem::size_of::<Vec<Span<'static>>>()
                    + rows
                        .iter()
                        .map(|row| {
                            row.capacity() * std::mem::size_of::<Span<'static>>()
                                + row.iter().map(|span| span.content.len()).sum::<usize>()
                        })
                        .sum::<usize>()
            }));
        if bytes <= self.max_cache_bytes {
            let mut cache = self.cache.borrow_mut();
            while cache.len() >= 16
                || cache
                    .iter()
                    .map(|entry| entry.bytes)
                    .sum::<usize>()
                    .saturating_add(bytes)
                    > self.max_cache_bytes
            {
                cache.pop_front();
            }
            cache.push_back(CachedBlock {
                lang: lang.to_owned(),
                lines: lines.iter().map(|line| (*line).to_owned()).collect(),
                theme: *theme,
                result: result.clone(),
                bytes,
            });
        }
        result
    }
}

/// Turns a fenced code block's source into per-line styled spans.
///
/// Implementations receive the fence's language tag, the block's body split
/// into lines, and the active [`Theme`] (so highlighted tokens follow the host
/// palette via [`Theme::code`](crate::style::CodeTheme)). They return **one span
/// vector per input line** — reconstructing each source line exactly, only
/// restyled — or [`None`] when the language is unknown or the source fails to
/// parse, in which case the caller falls back to unstyled code text.
///
/// The one-line-per-line contract lets callers zip the result against the
/// source without re-deriving line boundaries; an implementation that cannot
/// uphold it (event stream desynced from source lines) must return [`None`].
pub trait Highlighter {
    /// Highlight `lines` of `lang` source, returning one styled span vector per
    /// input line (same length and order), or [`None`] if the language is
    /// unknown or the source cannot be highlighted line-for-line.
    fn highlight(
        &self,
        lang: &str,
        lines: &[&str],
        theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>>;
}

/// A [`Highlighter`] that highlights nothing — every block renders as plain,
/// theme-colored code text. The default when a caller supplies no highlighter.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlainHighlighter;

impl Highlighter for PlainHighlighter {
    fn highlight(
        &self,
        _lang: &str,
        _lines: &[&str],
        _theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>> {
        None
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use std::cell::Cell;

    struct Counting(Cell<usize>);
    impl Highlighter for Counting {
        fn highlight(&self, lang: &str, lines: &[&str], _: &Theme) -> Highlighted {
            self.0.set(self.0.get() + 1);
            (lang != "unknown").then(|| {
                lines
                    .iter()
                    .map(|line| vec![Span::raw((*line).to_owned())])
                    .collect()
            })
        }
    }

    #[test]
    fn syntax_cache_keys_source_language_and_theme_and_caches_declines() {
        let inner = Counting(Cell::new(0));
        let cached = CachedHighlighter::new(&inner);
        let theme = Theme::default();
        for _ in 0..3 {
            assert_eq!(
                cached.highlight("rust", &["世界"], &theme).unwrap()[0][0].content,
                "世界"
            );
            assert!(cached.highlight("unknown", &["世界"], &theme).is_none());
        }
        assert_eq!(inner.0.get(), 2);
        cached.highlight("rust", &["changed"], &theme);
        cached.highlight("other", &["世界"], &theme);
        let mut changed = theme;
        changed.code.text = crate::style::Color::Red;
        cached.highlight("rust", &["世界"], &changed);
        assert_eq!(inner.0.get(), 5);
        cached.clear();
        cached.highlight("rust", &["世界"], &theme);
        assert_eq!(inner.0.get(), 6);
    }

    #[test]
    fn source_budget_includes_newlines_and_preserves_plain_fallback() {
        let inner = Counting(Cell::new(0));
        let cached = CachedHighlighter::new(&inner).with_limits(5, 4096);
        assert!(
            cached
                .highlight("rust", &["123", "45"], &Theme::default())
                .is_none()
        );
        assert_eq!(inner.0.get(), 0);
        assert!(
            cached
                .highlight("rust", &["12345"], &Theme::default())
                .is_some()
        );
        assert_eq!(inner.0.get(), 1);
    }

    #[test]
    fn cache_evicts_old_blocks_and_respects_storage_budget() {
        let inner = Counting(Cell::new(0));
        let cached = CachedHighlighter::new(&inner);
        for n in 0..17 {
            cached.highlight("rust", &[&n.to_string()], &Theme::default());
        }
        assert_eq!(cached.cache.borrow().len(), 16);
        cached.highlight("rust", &["0"], &Theme::default());
        assert_eq!(inner.0.get(), 18);
        let tiny = CachedHighlighter::new(&inner).with_limits(100, 1);
        for _ in 0..2 {
            tiny.highlight("rust", &["x"], &Theme::default());
        }
        assert!(tiny.cache.borrow().is_empty());
        assert_eq!(inner.0.get(), 20);
    }

    #[test]
    fn storage_eviction_keeps_the_recently_used_block() {
        let inner = Counting(Cell::new(0));
        let cached = CachedHighlighter::new(&inner);
        let theme = Theme::default();
        cached.highlight("rust", &["a"], &theme);
        let entry_bytes = cached.cache.borrow()[0].bytes;
        let cached = cached.with_limits(100, entry_bytes * 2);
        cached.highlight("rust", &["a"], &theme);
        cached.highlight("rust", &["b"], &theme);
        cached.highlight("rust", &["a"], &theme);
        cached.highlight("rust", &["c"], &theme);
        cached.highlight("rust", &["a"], &theme);
        assert_eq!(inner.0.get(), 4);
        assert!(
            cached
                .cache
                .borrow()
                .iter()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                <= entry_bytes * 2
        );
        cached.highlight("rust", &["b"], &theme);
        assert_eq!(inner.0.get(), 5);
    }
}

/// A borrowed highlighter, or the plain fallback. Lets [`CodeBlock`] and
/// [`Markdown`] carry an optional highlighter without a generic parameter
/// leaking through the whole view tree.
///
/// [`CodeBlock`]: crate::components::CodeBlock
/// [`Markdown`]: crate::components::Markdown
#[derive(Clone, Copy, Default)]
pub enum CodeHighlighter<'a> {
    /// No highlighting; render plain theme-colored code.
    #[default]
    Plain,
    /// Delegate to a host-supplied highlighter.
    With(&'a dyn Highlighter),
}

impl<'a> CodeHighlighter<'a> {
    /// Highlight `lines`, or `None` for the plain fallback.
    pub fn highlight(
        &self,
        lang: &str,
        lines: &[&str],
        theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>> {
        match self {
            CodeHighlighter::Plain => None,
            CodeHighlighter::With(h) => h.highlight(lang, lines, theme),
        }
    }
}

impl<'a> From<&'a dyn Highlighter> for CodeHighlighter<'a> {
    fn from(h: &'a dyn Highlighter) -> Self {
        CodeHighlighter::With(h)
    }
}
