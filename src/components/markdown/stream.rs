//! [`MarkdownState`](super::MarkdownState) and the settled-prefix cache.
//!
//! The reason streaming is affordable: everything before the last stable block
//! boundary is parsed *and* flattened once and kept, so a delta re-parses only
//! the in-flight tail. Without it a streamed render is O(n²) in transcript
//! length — see the comments on the cache fields.

use crate::text::Line;

use crate::highlight::CodeHighlighter;
use crate::style::{StyleSheet, Theme};
use crate::term::hyperlink::BufferLink;

use super::MarkdownBlockRenderer;
use super::flatten::flatten_linked_into;
use super::image::{ImageResolver, MarkdownImage};
use super::item::MdItem;
use super::parse::parse_with;

/// Whether `trimmed` (a line with its indentation removed) starts a list item.
///
/// Bullet (`-`, `*`, `+`) or ordered (`1.`, `1)`) markers only — a thematic
/// break (`---`) or emphasis (`*text*`) is not one, which is what the
/// delimiter-must-be-followed-by-a-space rule rules out.
fn is_list_marker(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    match chars.next() {
        Some('-' | '*' | '+') => matches!(chars.next(), None | Some(' ')),
        Some(digit) if digit.is_ascii_digit() => {
            let mut rest = trimmed
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .chars();
            matches!(rest.next(), Some('.' | ')')) && matches!(rest.next(), None | Some(' '))
        }
        _ => false,
    }
}

/// The delimiter and run length of a fence opened by `trimmed`, if it opens one.
fn fence_opener(trimmed: &str) -> Option<(char, usize)> {
    let delimiter = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = trimmed.chars().take_while(|c| *c == delimiter).count();
    (run >= 3).then_some((delimiter, run))
}

/// Byte offset of the last *stable block boundary* in `source[from..]`, in
/// absolute bytes: the position just past the last blank line that ends a block
/// outright. Blocks before it are complete and safe to cache; the tail after it
/// is still in flight.
///
/// Two constructs make a blank line *not* a block boundary, and both were found
/// by the streaming/one-shot fuzz differential:
///
/// - **An open code fence.** Everything up to the closing fence is one block, so
///   a blank inside it settles nothing. CommonMark closes a fence only on a bare
///   run of at least as many of the *same* delimiter, so `` ```rust `` inside an
///   open block is code content, not a closer.
/// - **An open list.** A blank line between items keeps one list going, so
///   settling there would parse the rest in isolation and restart the numbering
///   — a streamed `1. … 2. …` would render as `1. … 1. …`. The blank becomes a
///   boundary only once a following top-level block proves the list ended.
fn stable_boundary(source: &str, from: usize) -> usize {
    let mut fence: Option<(char, usize)> = None;
    let mut boundary = from;
    let mut pos = from;
    // Whether the prefix is inside a list, and the blank line that will become a
    // boundary if a top-level block turns out to follow it.
    let mut in_list = false;
    let mut pending_blank: Option<usize> = None;
    for line in source[from..].split_inclusive('\n') {
        let trimmed = line.trim();
        let end = pos + line.len();
        pos = end;

        if let Some((delimiter, opened)) = fence {
            let run = trimmed.chars().take_while(|c| *c == delimiter).count();
            if run >= opened && trimmed.len() == run {
                fence = None;
            }
            continue;
        }

        if trimmed.is_empty() {
            // Only a *terminated* blank line settles the prefix. A trailing line
            // with no newline yet is still in flight, and mid-stream it is often
            // whitespace-only for a few deltas — the indent of a nested list item
            // or an indented block — so treating it as blank would cut the list
            // in half and cache the halves as separate blocks.
            if line.ends_with('\n') {
                if in_list {
                    pending_blank = Some(end);
                } else {
                    boundary = end;
                }
            }
            continue;
        }

        let indented = line.starts_with("  ") || line.starts_with('\t');
        if is_list_marker(trimmed) {
            in_list = true;
            pending_blank = None;
        } else if in_list && (indented || pending_blank.is_none()) {
            // An indented continuation, or a lazy one with no blank in between:
            // either way the list item is still open.
            pending_blank = None;
        } else if line.ends_with('\n') {
            // A top-level block. It ends any open list, which makes the blank
            // line before it a real boundary.
            //
            // Only a *terminated* line may do this: mid-stream the last line is
            // a prefix of itself, and "1" is not yet the list marker "1." that
            // the next delta makes it. Settling on the partial line would cache
            // a boundary the finished document does not have.
            if let Some(blank) = pending_blank.take() {
                boundary = blank;
            }
            in_list = false;
        }
        if let Some(opener) = fence_opener(trimmed) {
            fence = Some(opener);
        }
    }
    boundary
}

/// Incremental markdown renderer for streamed text — the state to hold across
/// frames for a live transcript.
///
/// Feed it deltas with [`push_str`](Self::push_str) (or replace the whole buffer
/// with [`set`](Self::set)); call [`lines`](Self::lines) each frame for the
/// current width-fitted rendering, then read [`links`](Self::links) and
/// [`images`](Self::images) for its out-of-band metadata. Settled blocks — everything before the last
/// blank line outside an open code fence — are parsed, highlighted, **and
/// flattened once** and cached; each delta re-does only the in-flight tail. That
/// keeps a streamed render linear in the transcript length, instead of
/// re-tokenizing and re-laying-out the whole settled prefix on every delta.
///
/// The parse/highlight cache is width-independent. The flattened-line cache is
/// per-width: a resize re-wraps the settled prefix once (then reuses it), and a
/// [`set`](Self::set) or [`Theme`] change discards everything. [`lines`](Self::lines)
/// returns a borrow of the cached line buffer — clone it with `.to_vec()` to own it.
///
/// ```
/// use tuika::prelude::*;
/// let theme = Theme::default();
/// let sheet = StyleSheet::from_theme(&theme);
/// let mut md = MarkdownState::new();
/// for delta in ["# Title\n\n", "Some **bo", "ld** text.\n"] {
///     md.push_str(delta);                                  // forward each stream delta
///     let _lines = md.lines(80, &theme, &sheet, CodeHighlighter::Plain); // render this frame
/// }
/// ```
#[derive(Default)]
pub struct MarkdownState {
    source: String,
    stable_len: usize,
    stable: Vec<MdItem>,
    cached_theme: Option<Theme>,
    cached_sheet: Option<StyleSheet>,
    // Settled lines are flattened *once*, as blocks settle, and kept here across
    // frames — never re-flattened while streaming. Without this, `lines` would
    // re-flatten (re-wrap, re-lay-out, re-clone) the whole settled prefix every
    // delta, making a streamed render O(n²) in the transcript length. Returning a
    // borrow of this buffer also avoids re-materializing the prefix per frame.
    /// Flattened settled lines followed by the current in-flight tail.
    rendered: Vec<Line<'static>>,
    /// Count of leading `rendered` entries that are settled (cached) lines; the
    /// rest is the per-frame tail, dropped and rebuilt on the next call.
    settled_lines: usize,
    /// Count of `stable` items already flattened into the settled prefix.
    flattened_items: usize,
    /// Width `rendered` was flattened at; a change re-wraps the whole prefix.
    rendered_width: Option<u16>,
    /// Optional host hook turning image URLs into pixels; off ⇒ text placeholders.
    resolver: Option<Box<dyn ImageResolver>>,
    /// Ordered host hooks parsing and laying out structured blocks.
    block_renderers: Vec<Box<dyn MarkdownBlockRenderer>>,
    /// Block images in the settled prefix, with their absolute `rendered` rows —
    /// accumulated once as blocks settle, mirroring `settled_lines`.
    settled_images: Vec<MarkdownImage>,
    /// Settled + tail images with absolute rows, rebuilt each [`lines`](Self::lines)
    /// call; returned by [`images`](Self::images).
    frame_images: Vec<MarkdownImage>,
    /// Settled + tail links with absolute rows, rebuilt by [`lines`](Self::lines).
    frame_links: Vec<BufferLink>,
    /// Count of leading `frame_links` entries that belong to settled lines.
    settled_link_count: usize,
}

impl MarkdownState {
    /// An empty renderer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Render `![alt](url)` images as real pixels, resolving each URL to
    /// [`ImageData`](crate::term::image::ImageData) via `resolver` (markdown carries only the URL — see
    /// [`ImageResolver`]). After each [`lines`](Self::lines) call, read the
    /// reserved placements from [`images`](Self::images) and paint them.
    ///
    /// The resolver may be called repeatedly for an image still in the in-flight
    /// tail (re-parsed each frame), so a host that decodes lazily should cache.
    pub fn with_image_resolver(mut self, resolver: Box<dyn ImageResolver>) -> Self {
        self.resolver = Some(resolver);
        self.reset_cache();
        self
    }

    /// Append a renderer for structured blocks such as fenced diagrams or raw
    /// block HTML.
    ///
    /// Renderers are consulted in registration order; the first returning
    /// `Some` owns a block. Call this repeatedly to compose independent
    /// companion renderers. A settled block is laid out once per width, while
    /// one still in the streaming tail may be attempted on each frame.
    pub fn with_block_renderer(mut self, renderer: Box<dyn MarkdownBlockRenderer>) -> Self {
        self.block_renderers.push(renderer);
        self.reset_cache();
        self
    }

    /// The block images reserved by the last [`lines`](Self::lines) call, with
    /// rows relative to those lines. Empty unless a resolver is attached (see
    /// [`with_image_resolver`](Self::with_image_resolver)). Paint each by
    /// rendering an [`Image`](crate::components::Image) at [`MarkdownImage::rect`] against the same area.
    pub fn images(&self) -> &[MarkdownImage] {
        &self.frame_images
    }

    /// Hyperlink runs produced by the last [`lines`](Self::lines) call, with
    /// rows relative to those lines.
    ///
    /// After painting the lines, pass the visible runs to
    /// [`apply_buffer_links`](crate::term::hyperlink::apply_buffer_links) so
    /// labeled and incrementally streamed links retain native OSC 8 activation.
    pub fn links(&self) -> &[BufferLink] {
        &self.frame_links
    }

    /// Append a streamed delta to the buffer (the settled-prefix cache is kept).
    pub fn push_str(&mut self, delta: &str) {
        self.source.push_str(delta);
    }

    /// Replace the whole buffer, discarding the cache. Use for a non-streaming
    /// re-render, or to reset between messages.
    pub fn set(&mut self, source: impl Into<String>) {
        self.source = source.into();
        self.reset_cache();
    }

    /// The accumulated source so far.
    pub fn source(&self) -> &str {
        &self.source
    }

    fn reset_cache(&mut self) {
        self.stable_len = 0;
        self.stable.clear();
        self.rendered.clear();
        self.settled_lines = 0;
        self.flattened_items = 0;
        self.rendered_width = None;
        self.settled_images.clear();
        self.frame_images.clear();
        self.frame_links.clear();
        self.settled_link_count = 0;
    }

    /// Render the current buffer to final, width-fitted styled lines, advancing
    /// the settled-prefix cache.
    ///
    /// `width` word-wraps prose (code and tables stay verbatim); `theme` supplies
    /// every color via [`Theme::code`](crate::style::CodeTheme); `highlighter` colors
    /// fenced code ([`CodeHighlighter::Plain`] for none). Draw the result
    /// **without** further wrapping (e.g. ratatui `Paragraph` with no `.wrap`).
    /// After painting, apply [`links`](Self::links) to the same area.
    ///
    /// Returns a borrow of an internally-cached line buffer: settled blocks are
    /// flattened once and only the in-flight tail is recomputed per call, so a
    /// streamed render stays linear in the transcript length. The borrow is valid
    /// until the next mutation of `self`; clone with `.to_vec()` if you need to
    /// own it (e.g. to move into a ratatui `Text`).
    pub fn lines(
        &mut self,
        width: u16,
        theme: &Theme,
        sheet: &StyleSheet,
        highlighter: CodeHighlighter,
    ) -> &[Line<'static>] {
        // A theme or stylesheet change restyles everything, so every cache is
        // invalid (both feed the styles baked into the cached, parsed spans).
        if self.cached_theme != Some(*theme) || self.cached_sheet != Some(*sheet) {
            self.cached_theme = Some(*theme);
            self.cached_sheet = Some(*sheet);
            self.reset_cache();
        }
        // A width change re-wraps every settled line, but the width-independent
        // parse cache survives; drop only the flattened lines (and their images,
        // whose row offsets and sizes are width-dependent).
        if self.rendered_width != Some(width) {
            self.rendered_width = Some(width);
            self.rendered.clear();
            self.settled_lines = 0;
            self.flattened_items = 0;
            self.settled_images.clear();
            self.frame_links.clear();
            self.settled_link_count = 0;
        }

        let boundary = stable_boundary(&self.source, self.stable_len);
        if boundary > self.stable_len {
            let segment = &self.source[self.stable_len..boundary];
            let mut items =
                parse_with(segment, theme, sheet, highlighter, self.resolver.as_deref());
            // Each segment parses in isolation, so the blank-line separation the
            // boundary sits on is lost — restore it between committed segments.
            if !items.is_empty()
                && !self.stable.is_empty()
                && !matches!(self.stable.last(), Some(MdItem::Blank))
            {
                self.stable.push(MdItem::Blank);
            }
            self.stable.append(&mut items);
            self.stable_len = boundary;
        }

        // Drop the previous frame's tail (and settled/tail gap), then extend the
        // settled prefix with any blocks that settled since — flattened once.
        // `flatten` maps each item independently, so appending the new items'
        // lines equals re-flattening the whole prefix.
        self.rendered.truncate(self.settled_lines);
        self.frame_links.truncate(self.settled_link_count);
        if self.flattened_items < self.stable.len() {
            let base = self.rendered.len() as u16;
            let mut settled_imgs = Vec::new();
            let (settled, settled_links) = flatten_linked_into(
                &self.stable[self.flattened_items..],
                width,
                theme,
                sheet,
                self.block_renderers.as_slice(),
                &mut settled_imgs,
            );
            for mut link in settled_links {
                link.line = link.line.saturating_add(base);
                self.frame_links.push(link);
            }
            for mut img in settled_imgs {
                img.row = img.row.saturating_add(base);
                self.settled_images.push(img);
            }
            self.rendered.extend(settled);
            self.flattened_items = self.stable.len();
            self.settled_lines = self.rendered.len();
            self.settled_link_count = self.frame_links.len();
        }

        let tail = parse_with(
            &self.source[self.stable_len..],
            theme,
            sheet,
            highlighter,
            self.resolver.as_deref(),
        );
        let mut tail_imgs = Vec::new();
        let (tail_lines, tail_links) = flatten_linked_into(
            &tail,
            width,
            theme,
            sheet,
            self.block_renderers.as_slice(),
            &mut tail_imgs,
        );
        // The tail begins just past the boundary's blank line; keep that gap.
        if !self.rendered.is_empty()
            && !tail_lines.is_empty()
            && !is_blank_line(self.rendered.last().unwrap())
            && !is_blank_line(&tail_lines[0])
        {
            self.rendered.push(Line::default());
        }
        let tail_base = self.rendered.len() as u16;
        self.rendered.extend(tail_lines);

        // Republish this frame's placements: the settled prefix (fixed) plus the
        // in-flight tail, each shifted to its absolute row in `rendered`.
        self.frame_images.clear();
        self.frame_images
            .extend(self.settled_images.iter().cloned());
        for mut img in tail_imgs {
            img.row = img.row.saturating_add(tail_base);
            self.frame_images.push(img);
        }
        for mut link in tail_links {
            link.line = link.line.saturating_add(tail_base);
            self.frame_links.push(link);
        }
        &self.rendered
    }
}

/// Whether a rendered line is visually blank (no spans, or only whitespace).
fn is_blank_line(line: &Line) -> bool {
    line.spans.iter().all(|s| s.content.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::super::{Renderers, to_lines, to_lines_with};
    use super::*;

    use crate::style::StyleBundle;

    use crate::highlight::CodeHighlighter;
    use crate::style::{StyleSheet, Theme};

    use crate::text::Line;
    use std::cell::Cell;
    use std::rc::Rc;

    use super::super::{MarkdownBlock, MarkdownBlockContext, MarkdownBlockRenderer};

    #[test]
    fn streaming_preserves_link_targets_across_tail_settling_and_resize() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let mut state = MarkdownState::new();

        state.push_str("see <https://docs.rs/");
        state.lines(40, &theme, &sheet, CodeHighlighter::Plain);

        state.push_str("tuika>.\n\nnext");
        state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        assert_eq!(state.links().len(), 1);
        assert_eq!(state.links()[0].url, "https://docs.rs/tuika");

        state.lines(20, &theme, &sheet, CodeHighlighter::Plain);
        assert!(!state.links().is_empty());
        assert!(
            state
                .links()
                .iter()
                .all(|link| link.url == "https://docs.rs/tuika")
        );
    }

    #[test]
    fn sheet_change_invalidates_stream_cache() {
        use crate::style::Color;
        let theme = Theme::default();
        let mut state = MarkdownState::new();
        state.set("A [link](https://ex.com) in prose.");

        let default_sheet = StyleSheet::from_theme(&theme);
        let link_fg = |lines: &[Line<'static>]| {
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .find(|s| s.content.contains("link"))
                .expect("link span")
                .style
                .fg
        };
        assert_eq!(
            link_fg(state.lines(60, &theme, &default_sheet, CodeHighlighter::Plain)),
            Some(theme.code.link)
        );

        // Same theme, different stylesheet: the cached spans must be rebuilt.
        let recolored = StyleSheet {
            link: StyleBundle::new().fg(Color::Green),
            ..default_sheet
        };
        assert_eq!(
            link_fg(state.lines(60, &theme, &recolored, CodeHighlighter::Plain)),
            Some(Color::Green),
            "a stylesheet change must invalidate the stream cache"
        );
    }

    #[test]
    fn streaming_matches_one_shot_render() {
        let full = "# Heading\n\nA paragraph of text.\n\n```rust\nfn main() {}\n```\n\nDone.";
        let theme = Theme::default();
        let one_shot: Vec<String> = to_lines(
            full,
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        )
        .iter()
        .map(text)
        .collect();

        // Feed the same content in awkward chunks.
        let mut state = MarkdownState::new();
        let mut streamed = Vec::new();
        for chunk in [
            "# Head",
            "ing\n\nA para",
            "graph of text.\n\n```rus",
            "t\nfn main() {}\n```\n\nDone.",
        ] {
            state.push_str(chunk);
            streamed = state
                .lines(
                    40,
                    &theme,
                    &StyleSheet::from_theme(&theme),
                    CodeHighlighter::Plain,
                )
                .iter()
                .map(text)
                .collect();
        }
        assert_eq!(streamed, one_shot);
    }

    #[test]
    fn streaming_caches_settled_rendered_blocks_until_resize() {
        struct CountingRenderer(Rc<Cell<usize>>);

        impl MarkdownBlockRenderer for CountingRenderer {
            fn render(
                &self,
                block: MarkdownBlock<'_>,
                context: MarkdownBlockContext<'_>,
            ) -> Option<Vec<Line<'static>>> {
                let MarkdownBlock::Fenced { language, .. } = block else {
                    return None;
                };
                (language == "diagram").then(|| {
                    self.0.set(self.0.get() + 1);
                    vec![Line::raw(format!("width {}", context.width))]
                })
            }
        }

        let calls = Rc::new(Cell::new(0));
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let mut state =
            MarkdownState::new().with_block_renderer(Box::new(CountingRenderer(Rc::clone(&calls))));
        state.set("```diagram\nA --> B\n```\n\n");

        let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        assert_eq!(calls.get(), 1, "settled block should stay flattened");

        let _ = state.lines(20, &theme, &sheet, CodeHighlighter::Plain);
        assert_eq!(calls.get(), 2, "resize must lay out the block again");
    }

    #[test]
    fn streaming_an_indented_line_one_char_at_a_time_matches_one_shot() {
        // A provider streams tokens, so a nested item's indent arrives as its own
        // partial line: mid-stream the buffer ends in "   ", which *looks* blank.
        // Committing there settles half a list, and the two halves then parse as
        // unrelated top-level lists — indent lost, blank lines injected. Only a
        // newline-terminated blank line may settle the prefix.
        let full = "## Steps\n\n1. one\n   - nested\n2. two\n";
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let one_shot: Vec<String> = to_lines(full, 40, &theme, &sheet, CodeHighlighter::Plain)
            .iter()
            .map(text)
            .collect();

        let mut state = MarkdownState::new();
        for ch in full.chars() {
            state.push_str(&ch.to_string());
            // Render every delta: the cache advances per call, so a bad boundary
            // is committed permanently rather than only affecting one frame.
            let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        }
        let streamed: Vec<String> = state
            .lines(40, &theme, &sheet, CodeHighlighter::Plain)
            .iter()
            .map(text)
            .collect();
        assert_eq!(streamed, one_shot);
    }

    #[test]
    fn streaming_inline_html_one_char_at_a_time_matches_one_shot() {
        // The cache commits at block boundaries, so an inline-HTML scope that
        // outlived its block would style the tail while the tail is still being
        // re-parsed, and stop once it settled — a render that changes after the
        // stream ends. A tag also arrives in pieces (`<`, `b`, `>`), which must
        // not leave a half-open scope behind.
        let full = "Intro <b>bold <i>both</i></b> tail.\n\n\
                    <a href=\"https://ex.com\">link</a> then H<sub>2</sub>O.\n\n\
                    <b>never closed\n\nplain after.\n";
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        // Compare the *styles* too: a leaked scope changes how the tail is
        // painted without changing a single character of it.
        let styled = |lines: &[Line<'static>]| -> Vec<(String, crate::style::Style)> {
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .map(|s| (s.content.to_string(), s.style))
                .collect()
        };
        let one_shot = styled(&to_lines(full, 40, &theme, &sheet, CodeHighlighter::Plain));

        let mut state = MarkdownState::new();
        for ch in full.chars() {
            state.push_str(&ch.to_string());
            let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        }
        let streamed = styled(state.lines(40, &theme, &sheet, CodeHighlighter::Plain));
        assert_eq!(streamed, one_shot);
    }

    #[test]
    fn streaming_html_blocks_match_one_shot_and_cache_settled_ones() {
        // pulldown-cmark ends an HTML block at a blank line, so a `<details>`
        // with blank lines inside is several blocks — in one shot as much as
        // while streaming. That is what lets the cache settle an HTML block at
        // all: the framing does not depend on how much source has arrived.
        struct CountingHtml(Rc<Cell<usize>>);
        impl MarkdownBlockRenderer for CountingHtml {
            fn render(
                &self,
                block: MarkdownBlock<'_>,
                _: MarkdownBlockContext<'_>,
            ) -> Option<Vec<Line<'static>>> {
                let MarkdownBlock::Html { source } = block else {
                    return None;
                };
                self.0.set(self.0.get() + 1);
                Some(vec![Line::from(source.trim().to_string())])
            }
        }

        let full = "intro\n\n<details>\n<summary>more</summary>\n</details>\n\ndone\n";
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let one_shot: Vec<String> = to_lines_with(
            full,
            40,
            &theme,
            &sheet,
            CodeHighlighter::Plain,
            Renderers::new().renderer(&CountingHtml(Rc::new(Cell::new(0)))),
        )
        .iter()
        .map(text)
        .collect();

        let calls = Rc::new(Cell::new(0));
        let mut state =
            MarkdownState::new().with_block_renderer(Box::new(CountingHtml(Rc::clone(&calls))));
        for ch in full.chars() {
            state.push_str(&ch.to_string());
            let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        }
        let streamed: Vec<String> = state
            .lines(40, &theme, &sheet, CodeHighlighter::Plain)
            .iter()
            .map(text)
            .collect();
        assert_eq!(streamed, one_shot);

        // Once settled, the block is not re-rendered on further frames.
        let settled = calls.get();
        let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        assert_eq!(calls.get(), settled, "a settled HTML block stays flattened");
    }

    #[test]
    fn streaming_then_resize_matches_one_shot_at_new_width() {
        // The settled-prefix line cache is width-specific: a width change must
        // re-wrap the whole prefix, not serve stale lines flattened at the old
        // width. Settle several blocks at a wide width, then render narrower.
        let theme = Theme::default();
        let chunks = [
            "# A wide head",
            "ing that wraps when narrow\n\nA para",
            "graph long enough to wrap differently at 24 columns than at 60.\n\n",
            "- a bullet that also wraps\n\nDone.",
        ];
        let full: String = chunks.concat();

        let mut state = MarkdownState::new();
        for chunk in chunks {
            state.push_str(chunk);
            let _ = state.lines(
                60,
                &theme,
                &StyleSheet::from_theme(&theme),
                CodeHighlighter::Plain,
            );
        }
        let resized: Vec<String> = state
            .lines(
                24,
                &theme,
                &StyleSheet::from_theme(&theme),
                CodeHighlighter::Plain,
            )
            .iter()
            .map(text)
            .collect();
        let one_shot: Vec<String> = to_lines(
            &full,
            24,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        )
        .iter()
        .map(text)
        .collect();
        assert_eq!(
            resized, one_shot,
            "resized stream must equal a one-shot render at the new width"
        );
    }

    #[test]
    fn streaming_commits_a_stable_prefix() {
        let theme = Theme::default();
        let mut state = MarkdownState::new();
        state.push_str("First paragraph.\n\nSecond a");
        let _ = state.lines(
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        // The blank line after the first paragraph is a stable boundary, so its
        // bytes are committed to the cache and won't be re-parsed.
        assert!(state.stable_len > 0, "expected a committed prefix");
        assert!(!state.stable.is_empty());
    }

    #[test]
    fn stable_boundary_never_splits_open_code_fence() {
        // A blank line *inside* an unterminated fence is not a boundary.
        let src = "```\ncode\n\nmore code";
        assert_eq!(stable_boundary(src, 0), 0);
        // Once the fence closes, the trailing blank becomes a boundary.
        let closed = "```\ncode\n```\n\nafter";
        assert!(stable_boundary(closed, 0) > 0);
    }

    #[test]
    fn a_fence_line_with_an_info_string_does_not_close_a_fence() {
        // CommonMark closes a fence only on a bare run of the same delimiter, so
        // a "```rust" *inside* an open block is code content. Toggling on it
        // settled a boundary the one-shot parse puts inside the block, and every
        // paragraph after it then rendered as markdown in a streamed transcript
        // but as code in a re-render. Found by the streaming/one-shot fuzz
        // differential.
        assert_eq!(stable_boundary("```\n```rust\ncode\n\nmore", 0), 0);
        // A shorter run cannot close a longer fence, either.
        assert_eq!(stable_boundary("````\n```\ncode\n\nmore", 0), 0);
        // A longer run closes a shorter fence, and the blank after it settles.
        assert!(stable_boundary("```\ncode\n`````\n\nafter", 0) > 0);
        // Tildes and backticks do not close each other.
        assert_eq!(stable_boundary("~~~\n```\ncode\n\nmore", 0), 0);
    }

    #[test]
    fn a_blank_line_between_list_items_does_not_settle_the_prefix() {
        // A list continues across a blank line, so settling there parses the
        // rest in isolation and restarts the numbering. Found by the
        // streaming/one-shot fuzz differential, where a streamed "1. … 2. …"
        // rendered as "1. … 1. …".
        assert_eq!(stable_boundary("1. one\n\n2. two\n", 0), 0);
        assert_eq!(stable_boundary("- one\nlazy continuation\n\n- two\n", 0), 0);
        // A top-level block after the blank proves the list ended there.
        let ended = "- one\n\nparagraph\n";
        assert_eq!(stable_boundary(ended, 0), "- one\n\n".len());
        // An indented continuation keeps the item open.
        assert_eq!(stable_boundary("- one\n\n  still the item\n", 0), 0);
    }

    #[test]
    fn an_in_flight_line_does_not_end_a_list() {
        // Mid-stream the last line is a prefix of itself: "1" before "1." makes
        // it a marker. Settling the blank before it would cache a boundary the
        // finished document does not have, and the rest of the list would then
        // be parsed in isolation.
        let settled = "1. one\n\n";
        assert_eq!(stable_boundary(&format!("{settled}1"), 0), 0);
        assert_eq!(stable_boundary(&format!("{settled}1. two\n"), 0), 0);
        // A *terminated* top-level line still ends the list at that blank.
        assert_eq!(
            stable_boundary(&format!("{settled}paragraph\n"), 0),
            settled.len()
        );
    }

    #[test]
    fn streaming_a_list_across_a_blank_line_keeps_its_numbering() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let doc = "1. first\n\n2. second\n\n3. third\n";
        let mut state = MarkdownState::new();
        for delta in doc.split_inclusive('\n') {
            state.push_str(delta);
            let _ = state.lines(40, &theme, &sheet, CodeHighlighter::Plain);
        }
        let streamed: Vec<String> = state
            .lines(40, &theme, &sheet, CodeHighlighter::Plain)
            .iter()
            .map(text)
            .collect();
        let one_shot: Vec<String> = to_lines(doc, 40, &theme, &sheet, CodeHighlighter::Plain)
            .iter()
            .map(text)
            .collect();
        assert_eq!(streamed, one_shot);
        assert!(
            streamed.iter().any(|line| line.contains("2.")),
            "the second item keeps its number: {streamed:?}"
        );
    }

    #[test]
    fn theme_change_invalidates_cache() {
        let mut state = MarkdownState::new();
        state.push_str("Para one.\n\nPara two.\n\ntail");
        let a = Theme::default();
        let _ = state.lines(40, &a, &StyleSheet::from_theme(&a), CodeHighlighter::Plain);
        assert!(state.stable_len > 0);

        let mut b = Theme::default();
        b.code.heading = crate::style::Color::Indexed(200);
        let _ = state.lines(40, &b, &StyleSheet::from_theme(&b), CodeHighlighter::Plain);
        // Cache was rebuilt under the new theme; still consistent, no stale panic.
        assert_eq!(state.cached_theme, Some(b));
    }

    #[test]
    fn streaming_state_reports_a_block_image_placement() {
        let theme = Theme::default();
        let mut md = MarkdownState::new().with_image_resolver(Box::new(StubResolver));
        md.set("intro line\n\n![a cat](ok.png)\n\ntail line");
        let lines = md
            .lines(
                40,
                &theme,
                &StyleSheet::from_theme(&theme),
                CodeHighlighter::Plain,
            )
            .to_vec();
        let imgs = md.images();
        assert_eq!(imgs.len(), 1, "one block image reported");
        let img = &imgs[0];
        assert_eq!(img.alt, "a cat");
        // The reported row is inside the rendered lines and is a reserved blank.
        assert!((img.row as usize) < lines.len(), "row within lines");
        assert!(
            is_blank_line(&lines[img.row as usize]),
            "reserved row is blank"
        );
        // "intro line" is above the image, "tail line" below it.
        assert!(text(&lines[0]).contains("intro"));
    }

    #[test]
    fn streaming_image_row_matches_one_shot() {
        // Feeding the same document incrementally lands the image on the same row
        // as parsing it whole — the settled/tail offset bookkeeping is consistent.
        let theme = Theme::default();
        let doc = "# Title\n\nbefore\n\n![pic](ok.png)\n\nafter paragraph here";

        let mut whole = MarkdownState::new().with_image_resolver(Box::new(StubResolver));
        whole.set(doc);
        let _ = whole.lines(
            30,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let whole_row = whole.images()[0].row;

        let mut streamed = MarkdownState::new().with_image_resolver(Box::new(StubResolver));
        for chunk in [
            "# Title\n\nbe",
            "fore\n\n![pic](ok",
            ".png)\n\nafter ",
            "paragraph here",
        ] {
            streamed.push_str(chunk);
            let _ = streamed.lines(
                30,
                &theme,
                &StyleSheet::from_theme(&theme),
                CodeHighlighter::Plain,
            );
        }
        assert_eq!(streamed.images().len(), 1);
        assert_eq!(
            streamed.images()[0].row,
            whole_row,
            "streamed image row matches one-shot"
        );
    }

    #[test]
    fn no_resolver_means_no_streaming_placements() {
        let theme = Theme::default();
        let mut md = MarkdownState::new();
        md.set("![a cat](ok.png)");
        let _ = md.lines(
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        assert!(md.images().is_empty(), "images() empty without a resolver");
    }
}
