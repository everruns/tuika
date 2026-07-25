//! The [`Markdown`](super::Markdown) view: one-shot markdown placed in a layout.
//!
//! A thin `View` over the same two passes, for static markdown. Streaming hosts
//! hold a [`MarkdownState`](super::MarkdownState) instead and draw its lines.

use ratatui_core::layout::Rect;
use ratatui_core::text::Line;

use crate::components::Image;
use crate::components::text::line_width;
use crate::geometry::Size;
use crate::highlight::{CodeHighlighter, Highlighter};
use crate::style::{StyleSheet, Theme};
use crate::surface::Surface;
use crate::term::hyperlink::{BufferLink, LinkPolicy, apply_buffer_links};
use crate::term::image::{ImageLayer, ImageSupport};
use crate::view::{RenderCtx, View};

use super::FencedBlockRenderer;
use super::flatten::flatten_linked_into;
use super::image::{ImageResolver, MarkdownImage};
use super::parse::parse_with;

/// A view that renders a static markdown string to its area — word-wrapping
/// prose to the width and drawing code and tables verbatim.
///
/// ![markdown demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/markdown.gif)
///
/// For a *streaming* transcript, hold a [`MarkdownState`](super::MarkdownState) and draw its
/// [`lines`](super::MarkdownState::lines) directly (that is what the demo above does);
/// this view is the one-shot convenience for static markdown placed in a layout.
///
/// GFM tables are laid out to the render width, cells keeping their inline
/// styles, emoji, and links:
///
/// ![markdown table demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/markdown_table.png)
///
/// # Options
///
/// | Builder | Default | Effect |
/// | --- | --- | --- |
/// | [`new(source)`](Self::new) | — | the markdown source to render |
/// | [`highlighter(&h)`](Self::highlighter) | plain | syntax-highlight fenced code |
/// | [`block_renderer(&r)`](Self::block_renderer) | off | replace recognized fences with rendered blocks |
/// | [`images(&r, s, &l)`](Self::images) | off | render `![alt](url)` as real pixels |
/// | [`link_policy(p)`](Self::link_policy) | [`LinkPolicy::WEB`] | OSC 8 schemes for links |
///
/// ```no_run
/// use tuika::prelude::*;
/// let doc = Markdown::new("# Title\n\nSome **bold** prose.");
/// // `doc` is a `View`: render it via `tuika::paint` or embed it in a `Flex`.
/// # let _ = doc;
/// ```
pub struct Markdown<'a> {
    source: String,
    highlighter: CodeHighlighter<'a>,
    block_renderer: Option<&'a dyn FencedBlockRenderer>,
    resolver: Option<&'a dyn ImageResolver>,
    image_support: ImageSupport,
    image_layer: Option<ImageLayer>,
    /// Which URL schemes become OSC 8 hyperlinks when this view paints.
    link_policy: LinkPolicy,
}

impl<'a> Markdown<'a> {
    /// A markdown view over `source`, rendering fenced code as plain text until
    /// a highlighter is attached with [`highlighter`](Self::highlighter).
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            highlighter: CodeHighlighter::Plain,
            block_renderer: None,
            resolver: None,
            image_support: ImageSupport::None,
            image_layer: None,
            link_policy: LinkPolicy::default(),
        }
    }

    /// Use `highlighter` to syntax-highlight fenced code blocks.
    pub fn highlighter(mut self, highlighter: &'a dyn Highlighter) -> Self {
        self.highlighter = CodeHighlighter::With(highlighter);
        self
    }

    /// Render recognized fenced code blocks through `renderer`.
    ///
    /// The renderer receives the current viewport width. Returning `None` leaves
    /// a fence in the ordinary themed code-block presentation.
    pub fn block_renderer(mut self, renderer: &'a dyn FencedBlockRenderer) -> Self {
        self.block_renderer = Some(renderer);
        self
    }

    /// Configure which URL schemes are emitted as OSC 8 hyperlinks.
    ///
    /// The default ([`LinkPolicy::WEB`]) links `http(s)` only.
    /// [`LinkPolicy::NONE`](crate::term::hyperlink::LinkPolicy::NONE) paints styled labels but emits no OSC 8 — useful when
    /// the host handles clicks itself or wants links inert.
    pub fn link_policy(mut self, policy: LinkPolicy) -> Self {
        self.link_policy = policy;
        self
    }

    /// Render `![alt](url)` images as real pixels.
    ///
    /// `resolver` decodes each URL to [`ImageData`](crate::term::image::ImageData) (markdown has only the URL —
    /// see [`ImageResolver`]); a resolved image becomes a block reserved in the
    /// layout and painted over by an [`Image`](crate::components::Image) using `support`, recording its
    /// placement into `layer` for the host to [`emit`](ImageLayer::emit) after the
    /// frame. Unresolved images, and every image when `support` is
    /// [`ImageSupport::None`], fall back to the text placeholder / alt text.
    pub fn images(
        mut self,
        resolver: &'a dyn ImageResolver,
        support: ImageSupport,
        layer: &ImageLayer,
    ) -> Self {
        self.resolver = Some(resolver);
        self.image_support = support;
        self.image_layer = Some(layer.clone());
        self
    }

    /// Flatten the source to lines, block images, and hyperlink runs.
    fn lines_images_links(
        &self,
        width: u16,
        theme: &Theme,
        sheet: &StyleSheet,
    ) -> (Vec<Line<'static>>, Vec<MarkdownImage>, Vec<BufferLink>) {
        let items = parse_with(&self.source, theme, sheet, self.highlighter, self.resolver);
        let mut images = Vec::new();
        let (lines, links) =
            flatten_linked_into(&items, width, theme, self.block_renderer, &mut images);
        (lines, images, links)
    }
}

impl View for Markdown<'_> {
    fn measure(&self, available: Size) -> Size {
        let (lines, _, _) =
            self.lines_images_links(available.width, &Theme::default(), &StyleSheet::default());
        let width = lines.iter().map(line_width).max().unwrap_or(0);
        Size::new(width.min(available.width), lines.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let (lines, images, links) = self.lines_images_links(area.width, ctx.theme, &ctx.sheet);
        for (row, line) in lines.iter().enumerate() {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            let mut x = area.x;
            for span in &line.spans {
                if x >= area.right() {
                    break;
                }
                x = surface.set_string(x, y, span.content.as_ref(), span.style);
            }
        }
        // Embed OSC 8 for labeled links (and bare URLs) so Ctrl+click / Ghostty
        // open the destination even when the visible text is not the URL.
        apply_buffer_links(
            surface.buffer_mut(),
            ratatui_core::layout::Position {
                x: area.x,
                y: area.y,
            },
            &links,
            self.link_policy,
        );
        // Overlay each block image on the rows it reserved, reusing the standalone
        // `Image` component for pixel emission and the alt fallback alike.
        for img in images {
            let y = area.y.saturating_add(img.row);
            let x = area.x.saturating_add(img.indent);
            if y >= area.bottom() || x >= area.right() {
                continue;
            }
            let rect = Rect {
                x,
                y,
                width: img.cols.min(area.right() - x),
                height: img.rows.min(area.bottom() - y),
            };
            let mut image = Image::new(img.data, img.cols, img.rows)
                .support(self.image_support)
                .alt(img.alt);
            if let Some(layer) = &self.image_layer {
                image = image.in_layer(layer);
            }
            image.render(rect, surface, ctx);
        }
    }
}
