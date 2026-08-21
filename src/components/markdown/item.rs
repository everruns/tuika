//! The width-independent intermediate form both passes speak.
//!
//! Parsing produces these; flattening consumes them. Keeping the two passes
//! apart is what makes streaming affordable: a settled block is parsed once and
//! only re-flattened when the render width changes.

use crate::style::Style;
use crate::text::Span;
use pulldown_cmark::Alignment;

use crate::components::code_block::CodeRow;
use crate::term::image::ImageData;

/// Columns of indentation added per level of list / block-quote nesting.
pub(super) const INDENT: u16 = 2;

/// A parsed, width-independent markdown block. Wrapping and table layout happen
/// later, at [`flatten`] time, against a concrete width.
pub(super) enum MdItem {
    /// Word-wrappable prose (a paragraph line, heading, list item, quote line).
    Prose { spans: Vec<RichSpan>, indent: u16 },
    /// A fenced or indented code block. The source stays width-independent so a
    /// [`MarkdownBlockRenderer`](super::MarkdownBlockRenderer) can lay it out for
    /// the current viewport; the ordinary themed presentation is the fallback.
    CodeBlock {
        language: String,
        source: String,
        fallback: Vec<CodeRow>,
        indent: u16,
    },
    /// A GFM table, laid out to the available width when flattened.
    Table { table: TableData, indent: u16 },
    /// A raw block-level HTML run, kept verbatim for an
    /// [`MarkdownBlockRenderer`](super::MarkdownBlockRenderer) to lay out at flatten
    /// time. With no renderer attached it renders as nothing, which is what
    /// markdown did with all HTML before the boundary existed.
    Html { source: String, indent: u16 },
    /// A resolved block image: reserves rows at flatten time and is painted by an
    /// [`Image`](crate::components::Image) overlay in the view (see [`ImageResolver`]).
    Image {
        data: ImageData,
        alt: String,
        indent: u16,
    },
    /// A blank spacer row separating blocks.
    Blank,
}

/// An inline run that may carry an OSC 8 hyperlink target.
///
/// ratatui's [`Span`] has no href field, so markdown keeps the destination here
/// through wrapping; [`flatten_linked`] turns contiguous href runs into
/// [`BufferLink`]s the host (or [`Markdown`](super::Markdown) view) applies with
/// [`apply_buffer_links`](crate::term::hyperlink::apply_buffer_links).
#[derive(Clone, Debug)]
pub(super) struct RichSpan {
    pub(super) content: String,
    pub(super) style: Style,
    /// When set, this run is a hyperlink to `href` (the visible label may differ).
    pub(super) href: Option<String>,
}

impl RichSpan {
    pub(super) fn styled(content: impl Into<String>, style: Style, href: Option<String>) -> Self {
        Self {
            content: content.into(),
            style,
            href,
        }
    }

    pub(super) fn to_span(&self) -> Span<'static> {
        Span::styled(self.content.clone(), self.style)
    }
}

/// Table contents captured during parsing: each cell is a run of pre-styled
/// inline spans (bold, links, inline code, emoji), boxed and width-fitted at
/// render time by [`render_table`].
pub(super) struct TableData {
    pub(super) aligns: Vec<Alignment>,
    pub(super) header: Vec<Cell>,
    pub(super) rows: Vec<Vec<Cell>>,
}

/// One table cell's inline content, already styled by the shared inline
/// machinery so cells carry the same markup as prose.
pub(super) type Cell = Vec<RichSpan>;
