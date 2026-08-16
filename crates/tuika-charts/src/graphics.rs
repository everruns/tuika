use ratatui_core::style::Color;
use tuika::RenderCtx;
use tuika::term::image::ImageData;

use crate::axis::AxisLayout;
use crate::plan::{ChartPlan, Geometry};
use crate::{Chart, Point, draw_line};

const PIXELS_PER_COL: u32 = 8;
const PIXELS_PER_ROW: u32 = 16;

pub(super) fn render_pixels(
    cols: u16,
    rows: u16,
    chart: &Chart,
    plan: &ChartPlan,
    layout: &AxisLayout,
    ctx: &RenderCtx,
) -> Option<ImageData> {
    let _ = chart;
    let model = &plan.model;
    let width = u32::from(cols).checked_mul(PIXELS_PER_COL)?;
    let height = u32::from(rows).checked_mul(PIXELS_PER_ROW)?;
    if width == 0 || height == 0 {
        return None;
    }
    let background = rgb(ctx.theme.background);
    let pixels = usize::try_from(width.checked_mul(height)?).ok()?;
    let mut rgba = [background.0, background.1, background.2, 255].repeat(pixels);
    // The image plot occupies exactly the cells the portable renderer plots
    // into: one column for the axis, one row for its baseline. Tick labels sit
    // outside the image, so any other margin would misalign them.
    let left = PIXELS_PER_COL;
    let plot_width = width.saturating_sub(left);
    let plot_height = height.saturating_sub(PIXELS_PER_ROW);
    if plot_width == 0 || plot_height == 0 {
        return ImageData::from_rgba(width, height, rgba);
    }
    let at = |point: Point| {
        let (x, y) = model.map(point, plot_width, plot_height);
        (x + left as i32, y)
    };

    let axis = rgb(ctx.theme.border);
    let axis_x = left as i32 - 1;
    let baseline_y = plot_height as i32;
    pixel_line(
        &mut rgba,
        width,
        height,
        (axis_x, 0),
        (axis_x, baseline_y),
        axis,
    );
    pixel_line(
        &mut rgba,
        width,
        height,
        (axis_x, baseline_y),
        ((left + plot_width) as i32, baseline_y),
        axis,
    );
    for tick in &layout.y {
        let y = at(model.up_at(tick.value)).1;
        pixel_line(&mut rgba, width, height, (axis_x - 3, y), (axis_x, y), axis);
    }
    for tick in &layout.x {
        let x = at(model.across_at(tick.value)).0;
        pixel_line(
            &mut rgba,
            width,
            height,
            (x, baseline_y),
            (x, baseline_y + 3),
            axis,
        );
    }

    // The zero rule, when the value domain straddles it, so negative geometry
    // is read against the baseline rather than against the frame.
    if model.y.min < 0.0 && model.y.max > 0.0 {
        if model.horizontal {
            let x = at(model.across_at(0.0)).0;
            pixel_line(&mut rgba, width, height, (x, 0), (x, baseline_y), axis);
        } else {
            let y = at(model.up_at(0.0)).1;
            pixel_line(
                &mut rgba,
                width,
                height,
                (left as i32, y),
                ((left + plot_width) as i32, y),
                axis,
            );
        }
    }

    for mark in &plan.marks {
        let color = rgb(mark.color);
        match &mark.geometry {
            Geometry::Path { points, stepped } => {
                let mapped: Vec<_> = points.iter().map(|point| at(*point)).collect();
                pixel_polyline(&mut rgba, width, height, &mapped, color, *stepped);
            }
            Geometry::Points(points) => {
                for point in points {
                    pixel_marker(&mut rgba, width, height, at(*point), color);
                }
            }
            Geometry::Band { upper, lower } => {
                let edge: Vec<_> = upper.iter().map(|point| at(*point)).collect();
                let floor: Vec<_> = lower.iter().map(|point| at(*point)).collect();
                pixel_band(&mut rgba, width, height, &edge, &floor, dim(color));
                pixel_polyline(&mut rgba, width, height, &edge, color, false);
            }
            Geometry::Bars { bars, .. } => {
                for bar in bars {
                    let (x0, y0) = at(Point::new(bar.x0, bar.y0));
                    let (x1, y1) = at(Point::new(bar.x1, bar.y1));
                    for x in x0.min(x1)..=x0.max(x1) {
                        pixel_line(&mut rgba, width, height, (x, y0), (x, y1), color);
                    }
                }
            }
            Geometry::Arcs(_) => {}
        }
        if mark.markers {
            for sample in &mark.samples {
                pixel_marker(&mut rgba, width, height, at(*sample), color);
            }
        }
    }
    ImageData::from_rgba(width, height, rgba)
}

/// Fill between an upper edge and the floor it rests on.
fn pixel_band(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    upper: &[(i32, i32)],
    lower: &[(i32, i32)],
    color: (u8, u8, u8),
) {
    let floor_at = |index: usize| lower.get(index).map_or(height as i32, |&(_, y)| y);
    for (index, pair) in upper.windows(2).enumerate() {
        let base = floor_at(index).max(floor_at(index + 1));
        draw_line(pair[0], pair[1], |x, y| {
            pixel_line(rgba, width, height, (x, y), (x, base), color);
        });
    }
    if let Some(&(x, y)) = upper.first() {
        pixel_line(rgba, width, height, (x, y), (x, floor_at(0)), color);
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
