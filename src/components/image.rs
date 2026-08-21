//! The [`Image`] view: cell reservation and the text fallback.
//!
//! An image is the one component whose pixels tuika cannot paint into the cell
//! buffer, so this view owns only the half that *is* cells — it reserves a
//! `cols × rows` footprint, keeps those cells blank when the picture will cover
//! them, and paints alt text when it won't. The pixels themselves are recorded
//! into an [`ImageLayer`] and emitted after the frame by
//! [`term::image`](crate::term::image), which owns the graphics protocols.

use crate::geometry::Rect;
use crate::style::{Modifier, Style};

use crate::geometry::Size;
use crate::surface::Surface;
use crate::term::image::{ImageData, ImageLayer, ImageSupport};
use crate::view::{RenderCtx, View};

/// A view that displays a decoded image over the cells it reserves.
///
/// It always reserves `cols × rows` cells (bounded by the area it's given) and
/// always paints those cells — blank when the image will cover them, or the alt
/// text when the terminal can't. [`Runner`](crate::Runner) supplies detected
/// graphics support and a layer automatically. Custom hosts can attach them
/// explicitly so [`ImageLayer::emit`] paints the picture over the reserved
/// cells.
pub struct Image {
    data: ImageData,
    cols: u16,
    rows: u16,
    alt: String,
    support: Option<ImageSupport>,
    layer: Option<ImageLayer>,
}

impl Image {
    /// An image occupying `cols × rows` cells. A [`Runner`](crate::Runner) host
    /// renders it adaptively; a custom host can attach [`ImageSupport`] and an
    /// [`ImageLayer`] with [`support`](Self::support) and
    /// [`in_layer`](Self::in_layer).
    pub fn new(data: ImageData, cols: u16, rows: u16) -> Self {
        Self {
            data,
            cols,
            rows,
            alt: String::new(),
            support: None,
            layer: None,
        }
    }

    /// Override the graphics protocol supplied by the render context.
    pub fn support(mut self, support: ImageSupport) -> Self {
        self.support = Some(support);
        self
    }

    /// Override the image layer supplied by the render context.
    pub fn in_layer(mut self, layer: &ImageLayer) -> Self {
        self.layer = Some(layer.clone());
        self
    }

    /// Text shown (dimmed) when the resolved graphics context cannot paint.
    pub fn alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = alt.into();
        self
    }

    /// Whether this image will be emitted through a graphics protocol (as
    /// opposed to falling back to text).
    fn graphics<'a>(&'a self, ctx: &'a RenderCtx<'_>) -> (ImageSupport, Option<&'a ImageLayer>) {
        let inherited = ctx.image_graphics();
        let support = self
            .support
            .or_else(|| inherited.map(|(support, _)| support))
            .unwrap_or(ImageSupport::None);
        let layer = self
            .layer
            .as_ref()
            .or_else(|| inherited.map(|(_, layer)| layer));
        (support, layer)
    }
}

impl View for Image {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(self.cols, self.rows).clamp_to(available)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let (support, layer) = self.graphics(ctx);
        if support != ImageSupport::None && layer.is_some() {
            // The graphics escape will cover these cells after the frame; keep
            // them blank so nothing shows through at the image's edges and the
            // ratatui diff over the region is stable.
            surface.fill(Style::default().bg(ctx.theme.background));
            if let Some(layer) = layer {
                layer.record(area, self.data.clone(), support);
            }
        } else {
            self.render_fallback(area, surface, ctx);
        }
    }
}

impl Image {
    /// Paint the alt-text placeholder centered in `area` — what a terminal
    /// without graphics support shows.
    fn render_fallback(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        surface.fill(Style::default().bg(ctx.theme.background));
        let label = if self.alt.is_empty() {
            "[image]".to_string()
        } else {
            format!("[image: {}]", self.alt)
        };
        let style = Style::default()
            .fg(ctx.theme.muted)
            .add_modifier(Modifier::ITALIC);
        // Center within the reserved box; truncation is handled by set_string's
        // clip against the surface.
        let width = crate::width::str_cols(&label);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height / 2;
        surface.set_string(x, y, &label, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Theme;
    use crate::testing::{render, render_with_context};
    use crate::view::element;

    /// A tiny solid RGBA image: `w × h` pixels, all the same color.
    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> ImageData {
        let buf: Vec<u8> = std::iter::repeat_n(rgba, (w * h) as usize)
            .flatten()
            .collect();
        ImageData::from_rgba(w, h, buf).expect("valid rgba")
    }

    #[test]
    fn image_reserves_its_cell_footprint() {
        let img = Image::new(solid(2, 2, [0, 0, 0, 255]), 6, 3);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        assert_eq!(img.measure(Size::new(80, 24), &ctx), Size::new(6, 3));
        // Clamped to the available area.
        assert_eq!(img.measure(Size::new(4, 2), &ctx), Size::new(4, 2));
    }

    #[test]
    fn supported_image_records_a_placement_and_leaves_cells_blank() {
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let view = element(
            Image::new(solid(2, 2, [0, 0, 0, 255]), 4, 2)
                .support(ImageSupport::Kitty)
                .in_layer(&layer)
                .alt("cat"),
        );
        let buf = render(&view, 4, 2, &theme);
        // The reserved cells are blank (the image will cover them post-frame),
        // so no alt text leaks into the buffer.
        let text: String = (0..buf.area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(
            !text.contains("cat"),
            "supported image must not paint alt text"
        );
        assert_eq!(layer.len(), 1, "a placement was recorded");
        assert!(!layer.is_empty());
    }

    #[test]
    fn unsupported_image_paints_alt_text_and_records_nothing() {
        let theme = Theme::default();
        let layer = ImageLayer::new();
        // Support::None → fallback, even with a layer attached.
        let view = element(
            Image::new(solid(2, 2, [0, 0, 0, 255]), 20, 3)
                .in_layer(&layer)
                .alt("cat"),
        );
        let buf = render(&view, 20, 3, &theme);
        let mut whole = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                whole.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            whole.contains("[image: cat]"),
            "fallback should show alt text"
        );
        assert!(layer.is_empty(), "fallback records no placement");
    }

    #[test]
    fn image_uses_graphics_from_the_render_context() {
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let ctx = RenderCtx::new(&theme).with_image_graphics(ImageSupport::Kitty, &layer);
        let image = Image::new(solid(2, 2, [0, 0, 0, 255]), 4, 2).alt("cat");

        let buffer = render_with_context(&image, 4, 2, &ctx);

        assert_eq!(layer.len(), 1);
        assert!(
            buffer.content().iter().all(|cell| cell.symbol() == " "),
            "host graphics suppress the fallback"
        );
    }

    #[test]
    fn explicit_none_disables_host_graphics() {
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let ctx = RenderCtx::new(&theme).with_image_graphics(ImageSupport::Kitty, &layer);
        let image = Image::new(solid(2, 2, [0, 0, 0, 255]), 20, 3)
            .support(ImageSupport::None)
            .alt("cat");

        let buffer = render_with_context(&image, 20, 3, &ctx);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("[image: cat]"));
        assert!(layer.is_empty());
    }
}
