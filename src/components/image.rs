//! The [`Image`] view: cell reservation and the text fallback.
//!
//! An image is the one component whose pixels tuika cannot paint into the cell
//! buffer, so this view owns only the half that *is* cells — it reserves a
//! `cols × rows` footprint, keeps those cells blank when the picture will cover
//! them, and paints alt text — or, when asked, a [`Blitter`] mosaic of the
//! pixels — when it won't. The pixels themselves are recorded
//! into an [`ImageLayer`] and emitted after the frame by
//! [`term::image`](crate::term::image), which owns the graphics protocols.

use crate::geometry::Rect;
use crate::style::{Modifier, Style};

use crate::framebuffer::{Blitter, paint_mosaic};
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
    mosaic: Option<Blitter>,
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
            mosaic: None,
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

    /// Without a graphics protocol, draw the picture from cells with
    /// `blitter` instead of showing the alt text.
    ///
    /// Each sub-cell averages the pixels it covers, and finer blitters trade
    /// color detail for shape: see [`Blitter`] for the trade-off and font
    /// support. The alt text is not shown while a mosaic is set.
    ///
    /// ```
    /// use tuika::framebuffer::Blitter;
    /// use tuika::prelude::*;
    /// use tuika::term::image::ImageData;
    ///
    /// let red = ImageData::from_rgba(1, 1, vec![255, 0, 0, 255]).unwrap();
    /// let image = Image::new(red, 2, 1).mosaic(Blitter::Octant);
    /// let frame = tuika::testing::render(&image, 2, 1, &Theme::default());
    /// assert_eq!(frame[(0, 0)].bg, Color::Rgb(255, 0, 0));
    /// ```
    pub fn mosaic(mut self, blitter: Blitter) -> Self {
        self.mosaic = Some(blitter);
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
            // A layout-only pass discards its cells; recording here would emit
            // the image twice once the real paint records it again.
            if let Some(layer) = layer
                && !ctx.is_layout_pass()
            {
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
        if let Some(blitter) = self.mosaic {
            self.render_mosaic(area, surface, ctx, blitter);
            return;
        }
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

impl Image {
    /// Paint the pixels as a cell mosaic over the reserved `cols × rows`.
    fn render_mosaic(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx, blitter: Blitter) {
        let cols = self.cols.min(area.width);
        let rows = self.rows.min(area.height);
        if cols == 0 || rows == 0 {
            return;
        }
        let (w, h) = (self.data.pixel_width(), self.data.pixel_height());
        let (sw, sh) = blitter.subcells();
        let (grid_w, grid_h) = (
            u64::from(cols) * u64::from(sw),
            u64::from(rows) * u64::from(sh),
        );
        let bg = match ctx.theme.background {
            crate::style::Color::Rgb(r, g, b) => [r, g, b],
            _ => [0, 0, 0],
        };
        // Average a few evenly spaced source pixels per sub-cell (up to 4×4):
        // a box filter good enough to keep thin features, at a cost bounded
        // by the cell count rather than the image size.
        const TAPS: u64 = 4;
        let sample = |px: u32, py: u32| {
            let span = |p: u32, size: u32, grid: u64| {
                let lo = u64::from(p) * u64::from(size) / grid;
                let hi = ((u64::from(p) + 1) * u64::from(size) / grid).max(lo + 1);
                (lo, hi)
            };
            let (x0, x1) = span(px, w, grid_w);
            let (y0, y1) = span(py, h, grid_h);
            let (nx, ny) = ((x1 - x0).min(TAPS), (y1 - y0).min(TAPS));
            let mut sum = [0u64; 3];
            for j in 0..ny {
                let y = y0 + (2 * j + 1) * (y1 - y0) / (2 * ny);
                for i in 0..nx {
                    let x = x0 + (2 * i + 1) * (x1 - x0) / (2 * nx);
                    let p = self
                        .data
                        .pixel((x as u32).min(w - 1), (y as u32).min(h - 1));
                    // Composite over the theme background, so transparency
                    // reads as the surrounding UI.
                    let a = u64::from(p[3]);
                    for c in 0..3 {
                        sum[c] += (u64::from(p[c]) * a + u64::from(bg[c]) * (255 - a)) / 255;
                    }
                }
            }
            let n = nx * ny;
            sum.map(|c| ((c + n / 2) / n) as u8)
        };
        paint_mosaic(surface, (area.x, area.y), cols, rows, blitter, sample);
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

    #[test]
    fn mosaic_draws_pixels_instead_of_alt_text_without_graphics() {
        use crate::framebuffer::{Blitter, OCTANTS};
        // 4×4 pixels: left half white, right half black.
        let mut rgba = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                let v = if x < 2 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let data = ImageData::from_rgba(4, 4, rgba).unwrap();
        let image = Image::new(data.clone(), 1, 1)
            .alt("cat")
            .mosaic(Blitter::Octant);
        let buf = render(&image, 1, 1, &Theme::default());
        assert_eq!(buf[(0, 0)].symbol(), "▌");
        assert_eq!(buf[(0, 0)].fg, crate::style::Color::Rgb(255, 255, 255));
        assert!(OCTANTS.contains(&'▌'));

        // Every blitter at every size: no panic, never the alt text.
        for blitter in Blitter::ALL {
            for (w, h) in [(0, 0), (1, 1), (3, 2), (30, 9)] {
                let image = Image::new(data.clone(), 9, 4).alt("cat").mosaic(blitter);
                let buf = render(&image, w, h, &Theme::default());
                assert!(!crate::testing::grid(&buf).contains("cat"));
            }
        }
    }
}
