//! The [`Markdown`](super::Markdown) view: one-shot markdown placed in a layout.
//!
//! A thin `View` over the same two passes, for static markdown. Streaming hosts
//! hold a [`MarkdownState`](super::MarkdownState) instead and draw its lines.

use crate::geometry::Rect;
use crate::text::Line;

use crate::components::Image;
use crate::components::text::line_width;
use crate::geometry::Size;
use crate::highlight::{CodeHighlighter, Highlighter};
use crate::surface::Surface;
use crate::term::hyperlink::{BufferLink, LinkPolicy, apply_buffer_links_in};
use crate::term::image::{ImageLayer, ImageSupport};
use crate::term::text_size::{SizedRun, apply_sized_runs_in};
use crate::view::{RenderCtx, View};

use super::MarkdownBlockRenderer;
use super::flatten::flatten_linked_into;
use super::image::{ImageResolver, MarkdownImage};
use super::parse::parse_with;

/// A view that renders a static markdown string to its area — word-wrapping
/// prose to the width and drawing code and tables verbatim.
///
/// ![markdown demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/markdown.gif)
///
/// For a *streaming* transcript, hold a [`MarkdownState`](super::MarkdownState), draw its
/// [`lines`](super::MarkdownState::lines), and apply its
/// [`links`](super::MarkdownState::links) directly (that is what the demo above does);
/// this view is the one-shot convenience for static markdown placed in a layout.
///
/// The presentational inline HTML tags render through the same style roles as the
/// markdown they mirror, so a document that carries `<b>`, `<kbd>`, `<a>`, or
/// `<sub>` loses nothing:
///
/// ![markdown inline HTML demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/markdown_html.png)
///
/// Block-level HTML is a boundary handled by [`block_renderer`](Self::block_renderer).
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
/// | [`block_renderer(&r)`](Self::block_renderer) | off | append a structured block renderer |
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
    block_renderers: Vec<&'a dyn MarkdownBlockRenderer>,
    resolver: Option<&'a dyn ImageResolver>,
    image_support: ImageSupport,
    image_layer: Option<ImageLayer>,
    /// Which URL schemes become OSC 8 hyperlinks when this view paints.
    link_policy: LinkPolicy,
    /// Draw level-one headings large where the terminal sizes text.
    large_headings: bool,
}

impl<'a> Markdown<'a> {
    /// A markdown view over `source`, rendering fenced code as plain text until
    /// a highlighter is attached with [`highlighter`](Self::highlighter).
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            highlighter: CodeHighlighter::Plain,
            block_renderers: Vec::new(),
            resolver: None,
            image_support: ImageSupport::None,
            image_layer: None,
            link_policy: LinkPolicy::default(),
            large_headings: false,
        }
    }

    /// Draw level-one headings at twice the cell height on a terminal that
    /// implements OSC 66 text sizing (see [`term::text_size`](crate::term::text_size)).
    ///
    /// Takes effect only when the render context declares support
    /// ([`RenderCtx::with_text_sizing`]; [`Runner`](crate::Runner) detects it).
    /// Elsewhere headings render exactly as they do with this off.
    pub fn large_headings(mut self, enabled: bool) -> Self {
        self.large_headings = enabled;
        self
    }

    /// Use `highlighter` to syntax-highlight fenced code blocks.
    pub fn highlighter(mut self, highlighter: &'a dyn Highlighter) -> Self {
        self.highlighter = CodeHighlighter::With(highlighter);
        self
    }

    /// Append a renderer for structured blocks such as fenced diagrams or raw
    /// block HTML.
    ///
    /// Renderers are consulted in registration order; the first returning
    /// `Some` owns the block. Returning `None` leaves a fence in the ordinary
    /// themed code-block presentation and drops raw block HTML. Call this
    /// repeatedly to compose independent companion renderers.
    pub fn block_renderer(mut self, renderer: &'a dyn MarkdownBlockRenderer) -> Self {
        self.block_renderers.push(renderer);
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

    /// Flatten the source to lines, block images, hyperlink runs, and the
    /// heading runs to draw large.
    fn flatten(&self, width: u16, ctx: &RenderCtx) -> Flattened {
        let (theme, sheet) = (ctx.theme, &ctx.sheet);
        let items = parse_with(&self.source, theme, sheet, self.highlighter, self.resolver);
        let mut images = Vec::new();
        let mut sized = Vec::new();
        let sizing = self.large_headings && ctx.text_sizing().is_supported();
        let (lines, links) = flatten_linked_into(
            &items,
            width,
            theme,
            sheet,
            self.block_renderers.as_slice(),
            &mut images,
            sizing.then_some(&mut sized),
        );
        Flattened {
            lines,
            images,
            links,
            sized,
        }
    }
}

/// One flatten pass, as the view paints it.
struct Flattened {
    lines: Vec<Line<'static>>,
    images: Vec<MarkdownImage>,
    links: Vec<BufferLink>,
    sized: Vec<SizedRun>,
}

impl View for Markdown<'_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let lines = self.flatten(available.width, ctx).lines;
        let width = lines.iter().map(line_width).max().unwrap_or(0);
        Size::new(width.min(available.width), lines.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let Flattened {
            lines,
            images,
            links,
            sized,
        } = self.flatten(area.width, ctx);
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
        // Sized headings read the plain cells just painted, so they go before
        // the link markers. A block the clip cuts stays at normal size.
        if !sized.is_empty() {
            let visible = surface.area().intersection(area);
            apply_sized_runs_in(
                surface.buffer_mut(),
                area,
                &sized_within(&sized, area, visible),
            );
        }
        // Embed OSC 8 for labeled links (and bare URLs) so Ctrl+click / Ghostty
        // open the destination even when the visible text is not the URL.
        apply_buffer_links_in(surface.buffer_mut(), area, &links, self.link_policy);
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

/// The runs whose scaled block lies inside `visible`, the part of `area` the
/// surface may paint. A block straddling the clip would be erased by the cells
/// around it, so it is left at normal size.
fn sized_within(runs: &[SizedRun], area: Rect, visible: Rect) -> Vec<SizedRun> {
    runs.iter()
        .copied()
        .filter(|run| {
            let (width, height) = run.scaled_size();
            let block = Rect::new(
                area.x.saturating_add(run.start_col),
                area.y.saturating_add(run.line),
                width,
                height,
            );
            block.intersection(visible) == block
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{MarkdownBlock, MarkdownBlockContext, MarkdownBlockRenderer};
    use crate::style::StyleBundle;
    use crate::style::{Color, StyleSheet, Theme};
    use crate::testing::{grid, render_with_sheet};
    use crate::text::{Line, Span};

    struct ContextBlock;

    impl MarkdownBlockRenderer for ContextBlock {
        fn render(
            &self,
            block: MarkdownBlock<'_>,
            context: MarkdownBlockContext<'_>,
        ) -> Option<Vec<Line<'static>>> {
            if !matches!(block, MarkdownBlock::Html { .. }) {
                return None;
            }
            if context.theme.accent == Color::Cyan
                && context.sheet.heading.fg == Some(Color::Magenta)
            {
                Some(vec![
                    Line::from(Span::raw("alpha")),
                    Line::from(Span::raw("beta")),
                    Line::from(Span::raw("omega")),
                ])
            } else {
                Some(vec![Line::from(Span::raw("wrong context"))])
            }
        }
    }

    #[test]
    fn measurement_and_render_share_the_active_theme_and_sheet() {
        let theme = Theme {
            accent: Color::Cyan,
            ..Theme::default()
        };
        let sheet = StyleSheet {
            heading: StyleBundle::new().fg(Color::Magenta),
            ..StyleSheet::from_theme(&theme)
        };
        let renderer = ContextBlock;
        let view = Markdown::new("<div>context</div>").block_renderer(&renderer);
        let ctx = RenderCtx::new(&theme).with_sheet(sheet);

        assert_eq!(view.measure(Size::new(10, 10), &ctx), Size::new(5, 3));
        assert_eq!(
            grid(&render_with_sheet(&view, 10, 3, &theme, sheet)),
            "alpha     \nbeta      \nomega     "
        );
    }

    #[test]
    fn large_headings_draw_level_one_headings_sized() {
        use crate::buffer::CellDiffOption;
        use crate::term::text_size::{TextSizing, visible_text};
        use crate::testing::render_with_context;

        let theme = Theme::default();
        let view = Markdown::new("# Big\n\n## Small\n\nBody").large_headings(true);
        let sized = RenderCtx::new(&theme).with_text_sizing(TextSizing::Supported);
        assert_eq!(view.measure(Size::new(20, 20), &sized), Size::new(5, 6));
        let buf = render_with_context(&view, 20, 6, &sized);
        assert!(buf[(0, 0)].symbol().starts_with("\x1b]66;s=2:w=1;"));
        assert_eq!(
            (0..3)
                .map(|i| visible_text(buf[(i * 2, 0)].symbol()))
                .collect::<String>(),
            "Big"
        );
        assert_eq!(buf[(0, 1)].diff_option, CellDiffOption::Skip);
        // The blank after the heading survives, and H2 stays normal size.
        assert_eq!(buf[(0, 2)].symbol(), " ");
        assert_eq!(buf[(0, 3)].symbol(), "S");
        assert_eq!(buf[(0, 5)].symbol(), "B");

        // Without terminal support the option changes nothing.
        let plain = RenderCtx::new(&theme);
        let off = Markdown::new("# Big\n\n## Small\n\nBody");
        assert_eq!(
            grid(&render_with_context(&view, 20, 6, &plain)),
            grid(&render_with_context(&off, 20, 6, &plain))
        );
    }

    #[test]
    fn large_headings_wrap_to_half_the_width() {
        use crate::term::text_size::{TextSizing, visible_text};
        use crate::testing::render_with_context;

        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme).with_text_sizing(TextSizing::Supported);
        let view = Markdown::new("# one two").large_headings(true);
        // Ten columns hold five sized columns: "one" and "two" each get a row.
        let buf = render_with_context(&view, 10, 4, &ctx);
        assert_eq!(visible_text(buf[(0, 0)].symbol()), "o");
        assert_eq!(visible_text(buf[(0, 2)].symbol()), "t");
        assert_eq!(visible_text(buf[(4, 2)].symbol()), "o");
    }
}
