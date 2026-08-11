use ratatui_core::style::Color;
use tuika::RenderCtx;
use tuika::term::image::ImageData;

use crate::{Chart, PlotModel, Point, SeriesKind, chart_color, draw_line};

const PIXELS_PER_COL: u32 = 8;
const PIXELS_PER_ROW: u32 = 16;

pub(super) fn render_pixels(
    cols: u16,
    rows: u16,
    chart: &Chart,
    model: &PlotModel,
    ctx: &RenderCtx,
) -> Option<ImageData> {
    let width = u32::from(cols).checked_mul(PIXELS_PER_COL)?;
    let height = u32::from(rows).checked_mul(PIXELS_PER_ROW)?;
    if width == 0 || height == 0 {
        return None;
    }
    let background = rgb(ctx.theme.background);
    let pixels = usize::try_from(width.checked_mul(height)?).ok()?;
    let mut rgba = [background.0, background.1, background.2, 255].repeat(pixels);
    let top = 4;
    let left = PIXELS_PER_COL;
    let plot_width = width.saturating_sub(left + 4);
    let plot_height = height.saturating_sub(top + 4);
    if plot_width == 0 || plot_height == 0 {
        return ImageData::from_rgba(width, height, rgba);
    }
    let axis = rgb(ctx.theme.border);
    pixel_line(
        &mut rgba,
        width,
        height,
        (left as i32, top as i32),
        (left as i32, (top + plot_height) as i32),
        axis,
    );
    pixel_line(
        &mut rgba,
        width,
        height,
        (left as i32, (top + plot_height) as i32),
        ((left + plot_width) as i32, (top + plot_height) as i32),
        axis,
    );
    for (index, series) in chart.series.iter().enumerate() {
        let color = rgb(chart_color(series, index, ctx));
        let mapped: Vec<_> = series
            .points
            .iter()
            .copied()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| {
                let (x, y) = model.map(p, plot_width, plot_height);
                (x + left as i32, y + top as i32)
            })
            .collect();
        let baseline = model
            .map(Point::new(model.x.min, 0.0), plot_width, plot_height)
            .1
            .clamp(0, plot_height as i32)
            + top as i32;
        match series.kind {
            SeriesKind::Line => pixel_polyline(&mut rgba, width, height, &mapped, color, false),
            SeriesKind::Step => pixel_polyline(&mut rgba, width, height, &mapped, color, true),
            SeriesKind::Area => {
                pixel_area(&mut rgba, width, height, &mapped, baseline, dim(color));
                pixel_polyline(&mut rgba, width, height, &mapped, color, false);
            }
            SeriesKind::Scatter => {
                for point in mapped {
                    pixel_marker(&mut rgba, width, height, point, color);
                }
            }
            SeriesKind::Bar => {
                let half = (plot_width / mapped.len().max(1) as u32 / 3).clamp(1, 6) as i32;
                for (x, y) in mapped {
                    for px in x - half..=x + half {
                        pixel_line(&mut rgba, width, height, (px, y), (px, baseline), color);
                    }
                }
            }
        }
    }
    ImageData::from_rgba(width, height, rgba)
}

fn pixel_area(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    points: &[(i32, i32)],
    baseline: i32,
    color: (u8, u8, u8),
) {
    for pair in points.windows(2) {
        draw_line(pair[0], pair[1], |x, y| {
            pixel_line(rgba, width, height, (x, y), (x, baseline), color);
        });
    }
    if let Some(&(x, y)) = points.first() {
        pixel_line(rgba, width, height, (x, y), (x, baseline), color);
    }
}

fn pixel_polyline(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    points: &[(i32, i32)],
    color: (u8, u8, u8),
    stepped: bool,
) {
    for pair in points.windows(2) {
        if stepped {
            let corner = (pair[1].0, pair[0].1);
            pixel_line(rgba, width, height, pair[0], corner, color);
            pixel_line(rgba, width, height, corner, pair[1], color);
        } else {
            pixel_line(rgba, width, height, pair[0], pair[1], color);
            pixel_line(
                rgba,
                width,
                height,
                (pair[0].0, pair[0].1 + 1),
                (pair[1].0, pair[1].1 + 1),
                color,
            );
        }
    }
}

fn pixel_marker(rgba: &mut [u8], width: u32, height: u32, point: (i32, i32), color: (u8, u8, u8)) {
    for y in point.1 - 2..=point.1 + 2 {
        for x in point.0 - 2..=point.0 + 2 {
            if (x - point.0).pow(2) + (y - point.1).pow(2) <= 4 {
                set_pixel(rgba, width, height, x, y, color);
            }
        }
    }
}

fn dim((r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
    (r / 2, g / 2, b / 2)
}

fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (205, 49, 49),
        Color::Green => (13, 188, 121),
        Color::Yellow => (229, 229, 16),
        Color::Blue => (36, 114, 200),
        Color::Magenta => (188, 63, 188),
        Color::Cyan => (17, 168, 205),
        Color::Gray | Color::DarkGray => (128, 128, 128),
        Color::White => (255, 255, 255),
        _ => (200, 200, 200),
    }
}

fn pixel_line(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    from: (i32, i32),
    to: (i32, i32),
    color: (u8, u8, u8),
) {
    draw_line(from, to, |x, y| {
        set_pixel(rgba, width, height, x, y, color);
    });
}

fn set_pixel(rgba: &mut [u8], width: u32, height: u32, x: i32, y: i32, color: (u8, u8, u8)) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let offset = ((y as u32 * width + x as u32) * 4) as usize;
    rgba[offset..offset + 4].copy_from_slice(&[color.0, color.1, color.2, 255]);
}
