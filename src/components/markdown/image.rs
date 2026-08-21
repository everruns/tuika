//! The image boundary: resolving `![alt](url)` to pixels, and sizing it in cells.
//!
//! Markdown carries only a URL, and decoding it is the host's job (see
//! [`ImageResolver`]), so this module owns the boundary between the two: what a
//! resolved image costs in cells, and where it lands once the frame is painted.

use crate::geometry::Rect;

use crate::term::image::ImageData;

/// Widest a block image is drawn, in cells, regardless of the available width —
/// so a resolved `![alt](url)` stays a reasonable inline size rather than filling
/// the whole transcript.
const MAX_IMAGE_COLS: u16 = 60;

/// Resolves a markdown image URL to decoded pixels.
///
/// Markdown carries only the URL, never pixels, so — exactly like
/// [`Highlighter`](crate::highlight::Highlighter) does for fenced code — the host supplies the decode. A
/// resolved `![alt](url)` is rendered as a block image (real pixels via the
/// terminal graphics protocol, or the alt fallback); returning `None` leaves it
/// as the inline text placeholder. Wire one up with [`Markdown::images`](super::Markdown::images).
pub trait ImageResolver {
    /// Resolve `url` to image data, or `None` to keep the text placeholder.
    fn resolve(&self, url: &str) -> Option<ImageData>;
}

/// A block image in a rendered markdown document: the row it reserved (0-based,
/// within the returned lines), its cell footprint, the alt text, and the pixels.
///
/// The [`Markdown`](super::Markdown) view overlays these itself. A host that draws
/// [`MarkdownState::lines`](super::MarkdownState::lines) manually reads [`MarkdownState::images`](super::MarkdownState::images) and paints
/// each one — build an [`Image`](crate::components::Image) from `data`/`cols`/`rows`/`alt` and render it at
/// [`rect`](Self::rect) against the area the lines were drawn into.
#[derive(Clone, Debug)]
pub struct MarkdownImage {
    /// Row the image reserved, 0-based within the rendered lines.
    pub row: u16,
    /// Left indent in cells (list / block-quote nesting).
    pub indent: u16,
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
    /// Decoded pixels to paint.
    pub data: ImageData,
    /// Alt text, shown as the fallback where graphics aren't supported.
    pub alt: String,
}

impl MarkdownImage {
    /// The absolute screen rect for this image, given the `area` the markdown
    /// lines were drawn into — clamped so it never exceeds `area`.
    pub fn rect(&self, area: Rect) -> Rect {
        let x = area.x.saturating_add(self.indent).min(area.right());
        let y = area.y.saturating_add(self.row).min(area.bottom());
        Rect {
            x,
            y,
            width: self.cols.min(area.right().saturating_sub(x)),
            height: self.rows.min(area.bottom().saturating_sub(y)),
        }
    }
}

/// Cell footprint for a block image at `avail` columns: capped at
/// [`MAX_IMAGE_COLS`], with the row count derived from the pixel aspect ratio
/// (terminal cells are about twice as tall as wide, so the height is halved).
pub(super) fn image_cell_size(data: &ImageData, avail: u16) -> (u16, u16) {
    let cols = avail.clamp(1, MAX_IMAGE_COLS);
    let rows = (cols as u32 * data.pixel_height() / (data.pixel_width().max(1) * 2)).clamp(1, 30);
    (cols, rows as u16)
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::super::{Markdown, to_lines};
    use super::*;

    use crate::geometry::Rect;
    use crate::highlight::CodeHighlighter;
    use crate::style::Modifier;
    use crate::style::{StyleSheet, Theme};
    use crate::term::image::{ImageData, ImageLayer, ImageSupport};

    #[test]
    fn image_renders_alt_as_a_marked_placeholder() {
        let theme = Theme::default();
        let lines = to_lines(
            "look: ![a cat](https://ex.com/cat.png) ok",
            60,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let whole: String = lines.iter().map(text).collect::<Vec<_>>().join("\n");
        // The alt text shows behind an image marker; the surrounding prose stays.
        assert!(whole.contains("🖼 a cat"), "expected marked alt: {whole:?}");
        assert!(whole.contains("look:") && whole.contains("ok"));
        // The alt label is link-styled, not plain body text.
        let label = lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content.contains("a cat"))
            .expect("alt span");
        assert_eq!(label.style.fg, Some(theme.code.link));
        assert!(label.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn image_without_alt_shows_the_url_so_it_is_never_dropped() {
        // Before Tag::Image handling, an alt-less image vanished entirely — URL
        // and all. Now the URL itself is the visible, link-styled label.
        let out = plain("![](https://ex.com/x.png)", 60).join("\n");
        assert!(out.contains("🖼 "), "marker present: {out:?}");
        assert!(out.contains("https://ex.com/x.png"), "url shown: {out:?}");
    }

    #[test]
    fn resolved_block_image_reserves_rows_and_records_a_placement() {
        use crate::testing::render;
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let resolver = StubResolver;
        let view = Markdown::new("text before\n\n![a cat](ok.png)\n\nafter").images(
            &resolver,
            ImageSupport::Kitty,
            &layer,
        );
        // Render into a wide/tall buffer; the block image reserves rows and
        // records exactly one placement into the layer.
        let _buf = render(&view, 40, 12, &theme);
        assert_eq!(layer.len(), 1, "one block image recorded");
    }

    #[test]
    fn unresolved_image_stays_an_inline_placeholder_even_with_images_enabled() {
        use crate::testing::render;
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let resolver = StubResolver;
        // URL lacks "ok", so the resolver declines → inline placeholder, no pixels.
        let view =
            Markdown::new("![a dog](nope.png)").images(&resolver, ImageSupport::Kitty, &layer);
        let buf = render(&view, 40, 4, &theme);
        let mut whole = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                whole.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(whole.contains("🖼 "), "placeholder marker shown: {whole:?}");
        assert!(layer.is_empty(), "declined image records no placement");
    }

    #[test]
    fn block_image_without_graphics_support_shows_alt_fallback() {
        use crate::testing::render;
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let resolver = StubResolver;
        // Resolver yields pixels, but the terminal has no graphics support.
        let view = Markdown::new("![a cat](ok.png)").images(&resolver, ImageSupport::None, &layer);
        let buf = render(&view, 40, 6, &theme);
        let mut whole = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                whole.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            whole.contains("[image: a cat]"),
            "alt fallback painted: {whole:?}"
        );
        assert!(layer.is_empty(), "no placement without graphics support");
    }

    #[test]
    fn block_image_reserves_height_proportional_to_aspect() {
        // A 40x2 image (wide) reserves few rows; a 4x40 image (tall) reserves more.
        let wide = ImageData::from_rgba(40, 2, vec![0u8; 40 * 2 * 4]).unwrap();
        let tall = ImageData::from_rgba(4, 40, vec![0u8; 4 * 40 * 4]).unwrap();
        let (_, wr) = image_cell_size(&wide, 40);
        let (_, tr) = image_cell_size(&tall, 40);
        assert!(wr < tr, "tall image reserves more rows: {wr} vs {tr}");
        assert!(wr >= 1 && tr <= 30, "rows are clamped: {wr}, {tr}");
    }

    #[test]
    fn markdown_image_rect_offsets_by_area_and_row() {
        let img = MarkdownImage {
            row: 3,
            indent: 2,
            cols: 10,
            rows: 4,
            data: ImageData::from_rgba(2, 2, vec![0u8; 16]).unwrap(),
            alt: String::new(),
        };
        let rect = img.rect(Rect::new(5, 1, 40, 20));
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (7, 4, 10, 4));
        // Clamped to the area when it would overflow.
        let tight = img.rect(Rect::new(5, 1, 8, 5));
        assert_eq!(tight.width, 6, "clamped to area right edge");
    }
}
