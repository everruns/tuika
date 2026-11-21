use tuika::RenderCtx;
use tuika::term::image::ImageData;
use tuika::ui::Color;

use crate::axis::AxisLayout;
use crate::cells::reference_visible;
use crate::model::PlotModel;
use crate::plan::{ChartPlan, Geometry};
use crate::reference::ReferenceAt;
use crate::{Chart, Point, draw_dashed, draw_line, fill_polygon};

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
    let model = &plan.model;
    let width = u32::from(cols).checked_mul(PIXELS_PER_COL)?;
    let height = u32::from(rows).checked_mul(PIXELS_PER_ROW)?;
    if width == 0 || height == 0 {
        return None;
    }
    // Transparent, not filled with the theme background: the terminal
    // composites the image over the cells, so anything opaque here hides the
    // text beneath it. Leaving the empty space clear is what lets value labels
    // and the focus readout stay cells in graphics mode, exactly as the tick
    // labels outside the image already do.
    let pixels = usize::try_from(width.checked_mul(height)?).ok()?;
    let mut rgba = vec![0u8; pixels * 4];
    let data_cols = u32::from(cols).saturating_sub(1);
    let data_rows = u32::from(rows).saturating_sub(1);
    if data_cols == 0 || data_rows == 0 {
        return ImageData::from_rgba(width, height, rgba);
    }
    let frame = Frame::new(model, data_cols, data_rows);

    let axis = rgb(ctx.theme.border);
    pixel_line(
        &mut rgba,
        width,
        height,
        (frame.axis_x, 0),
        (frame.axis_x, frame.baseline_y),
        axis,
    );
    pixel_line(
        &mut rgba,
        width,
        height,
        (frame.axis_x, frame.baseline_y),
        (width as i32 - 1, frame.baseline_y),
        axis,
    );
    for tick in &layout.y {
        let y = frame.at(model.up_at(tick.value)).1;
        pixel_line(
            &mut rgba,
            width,
            height,
            (frame.axis_x - 3, y),
            (frame.axis_x, y),
            axis,
        );
    }
    for tick in &layout.x {
        let x = frame.at(model.across_at(tick.value)).0;
        pixel_line(
            &mut rgba,
            width,
            height,
            (x, frame.baseline_y),
            (x, frame.baseline_y + 3),
            axis,
        );
    }

    // Reference geometry first, so every mark lands on top of it.
    let (across_axis, up_axis) = crate::axis::screen_axes(chart);
    let grid = rgb(ctx.theme.dim);
    if up_axis.grid {
        for tick in &layout.y {
            let y = frame.at(model.up_at(tick.value)).1;
            if y < frame.baseline_y {
                let line = [(frame.axis_x + 1, y), (width as i32 - 1, y)];
                draw_dashed(&line, GRID_ON, GRID_OFF, |x, y| {
                    set_pixel(&mut rgba, width, height, x, y, grid);
                });
            }
        }
    }
    if across_axis.grid {
        for tick in &layout.x {
            let x = frame.at(model.across_at(tick.value)).0;
            if x > frame.axis_x {
                let line = [(x, 0), (x, frame.baseline_y - 1)];
                draw_dashed(&line, GRID_ON, GRID_OFF, |x, y| {
                    set_pixel(&mut rgba, width, height, x, y, grid);
                });
            }
        }
    }

    // The zero rule, when the value domain straddles it, so negative geometry
    // is read against the baseline rather than against the frame.
    if model.y.min < 0.0 && model.y.max > 0.0 {
        if model.horizontal {
            let x = frame.at(model.across_at(0.0)).0;
            pixel_line(
                &mut rgba,
                width,
                height,
                (x, 0),
                (x, frame.baseline_y),
                axis,
            );
        } else {
            let y = frame.at(model.up_at(0.0)).1;
            pixel_line(
                &mut rgba,
                width,
                height,
                (frame.axis_x, y),
                (width as i32 - 1, y),
                axis,
            );
        }
    }

    for reference in chart
        .references
        .iter()
        .filter(|reference| reference.finite() && reference_visible(model, reference))
    {
        let color = rgb(reference.color.unwrap_or(ctx.theme.border));
        let point = match reference.at {
            ReferenceAt::Value(v) => model.value_point(v),
            ReferenceAt::Position(p) => model.position_point(p),
        };
        let (x, y) = frame.at(point);
        let across = matches!(reference.at, ReferenceAt::Value(_)) != model.horizontal;
        let line = if across {
            [(frame.axis_x + 1, y), (width as i32 - 1, y)]
        } else {
            [(x, 0), (x, frame.baseline_y - 1)]
        };
        if reference.dashed {
            draw_dashed(&line, DASH_ON, DASH_OFF, |x, y| {
                set_pixel(&mut rgba, width, height, x, y, color);
            });
        } else {
            pixel_line(&mut rgba, width, height, line[0], line[1], color);
        }
    }

    if let Some(focus) = chart.focus {
        let accent = rgb(ctx.theme.accent);
        if model.horizontal {
            let y = frame.at(model.up_at(focus)).1;
            pixel_line(
                &mut rgba,
                width,
                height,
                (frame.axis_x, y),
                (width as i32 - 1, y),
                accent,
            );
        } else {
            let x = frame.at(model.across_at(focus)).0;
            pixel_line(
                &mut rgba,
                width,
                height,
                (x, 0),
                (x, frame.baseline_y),
                accent,
            );
        }
    }

    for mark in &plan.marks {
        let color = rgb(mark.color);
        match &mark.geometry {
            Geometry::Path { points, stepped } => {
                let mapped: Vec<_> = points.iter().map(|point| frame.at(*point)).collect();
                if mark.dashed {
                    let path = if *stepped {
                        crate::cells::stepped_path(&mapped)
                    } else {
                        mapped
                    };
                    let thick: Vec<_> = path.iter().map(|&(x, y)| (x, y + 1)).collect();
                    for path in [&path, &thick] {
                        draw_dashed(path, DASH_ON, DASH_OFF, |x, y| {
                            set_pixel(&mut rgba, width, height, x, y, color);
                        });
                    }
                } else {
                    pixel_polyline(&mut rgba, width, height, &mapped, color, *stepped);
                }
            }
            Geometry::Points(points) => {
                for point in points {
                    pixel_marker(&mut rgba, width, height, frame.at(*point), color);
                }
            }
            Geometry::Band { upper, lower } => {
                let edge: Vec<_> = upper.iter().map(|point| frame.at(*point)).collect();
                let floor: Vec<_> = lower.iter().map(|point| frame.floor(*point)).collect();
                pixel_band(&mut rgba, width, height, &edge, &floor, dim(color));
                pixel_polyline(&mut rgba, width, height, &edge, color, false);
            }
            Geometry::Fill { upper, lower } => {
                let upper: Vec<_> = upper.iter().map(|point| frame.floor(*point)).collect();
                let lower: Vec<_> = lower.iter().map(|point| frame.floor(*point)).collect();
                fill_polygon(&upper, &lower, |x, y| {
                    set_pixel(&mut rgba, width, height, x, y, color);
                });
            }
            Geometry::Bars { bars, .. } => {
                for bar in bars {
                    // The base corner snaps to the axis when it rests on the
                    // domain edge, so a bar reaches the baseline it is read
                    // against instead of stopping half a cell short.
                    let (x0, y0) = frame.floor(Point::new(bar.x0, bar.y0));
                    let (x1, y1) = frame.at(Point::new(bar.x1, bar.y1));
                    for x in x0.min(x1)..=x0.max(x1) {
                        pixel_line(&mut rgba, width, height, (x, y0), (x, y1), color);
                    }
                }
            }
            Geometry::Arcs(_) => {}
        }
        if mark.markers {
            for sample in &mark.samples {
                pixel_marker(&mut rgba, width, height, frame.at(*sample), color);
            }
        }
    }
    ImageData::from_rgba(width, height, rgba)
}

/// Grid lines are dotted, as the cell renderer's `┈`/`┊` glyphs are: a solid
/// line in the same color would outweigh the data it is meant to sit behind.
const GRID_ON: u32 = 1;
const GRID_OFF: u32 = 2;

/// Dash pattern for graphics strokes, in pixels: about one cell on, half off.
const DASH_ON: u32 = 7;
const DASH_OFF: u32 = 4;

/// Data-to-pixel mapping that lands on the same cells the cell renderer uses.
///
/// Tick labels are cells, placed at the row or column the cell renderer maps a
/// value to. Mapping the image's data range onto the *centres* of those same
/// cells, rather than stretching it edge to edge, keeps a label level with the
/// pixel it names: stretched, a value near the top drifted up to a whole row
/// away from its label.
struct Frame {
    model: PlotModel,
    /// Pixel span of the data range: from the first to the last data cell's
    /// centre.
    span_x: u32,
    span_y: u32,
    /// Centre of the first data column and of the top data row.
    origin_x: i32,
    origin_y: i32,
    /// The axis lines, through the centres of the axis column and row, where
    /// the cell renderer draws `│` and `─`.
    axis_x: i32,
    baseline_y: i32,
}

impl Frame {
    fn new(model: &PlotModel, data_cols: u32, data_rows: u32) -> Self {
        let half_col = (PIXELS_PER_COL / 2) as i32;
        let half_row = (PIXELS_PER_ROW / 2) as i32;
        Self {
            model: *model,
            span_x: (data_cols - 1) * PIXELS_PER_COL,
            span_y: (data_rows - 1) * PIXELS_PER_ROW,
            origin_x: PIXELS_PER_COL as i32 + half_col,
            origin_y: half_row,
            axis_x: half_col,
            baseline_y: (data_rows * PIXELS_PER_ROW) as i32 + half_row,
        }
    }

    fn at(&self, point: Point) -> (i32, i32) {
        let (x, y) = self.model.map(point, self.span_x + 1, self.span_y + 1);
        (x + self.origin_x, y + self.origin_y)
    }

    /// Like [`Frame::at`], but a coordinate on the value domain's low edge is
    /// pulled onto the axis: fills and bars rest on the baseline rather than
    /// on the centre of the last data cell.
    fn floor(&self, point: Point) -> (i32, i32) {
        let (x, y) = self.at(point);
        if self.model.horizontal {
            let low = x <= self.origin_x;
            (if low { self.axis_x + 1 } else { x }, y)
        } else {
            let low = y >= self.origin_y + self.span_y as i32;
            (x, if low { self.baseline_y - 1 } else { y })
        }
    }
}

/// A smooth ring. The cell renderer paints the same arcs in block glyphs; here
/// the boundary between segments lands on a pixel instead of a cell.
pub(super) fn render_donut_pixels(cols: u16, rows: u16, plan: &ChartPlan) -> Option<ImageData> {
    let width = u32::from(cols).checked_mul(PIXELS_PER_COL)?;
    let height = u32::from(rows).checked_mul(PIXELS_PER_ROW)?;
    if width == 0 || height == 0 {
        return None;
    }
    // Transparent everywhere but the ring, so the hole shows the centre text
    // the cell renderer drew beneath it.
    let pixels = usize::try_from(width.checked_mul(height)?).ok()?;
    let mut rgba = vec![0u8; pixels * 4];

    let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
    let radius = cx.min(cy) - 2.0;
    if radius <= 2.0 {
        return ImageData::from_rgba(width, height, rgba);
    }
    let inner = radius * 0.58;

    // Walk the disc once and colour each pixel by the arc its angle falls in,
    // which keeps segment boundaries exact without stroking arcs one by one.
    let arcs: Vec<(f64, f64, (u8, u8, u8))> = plan
        .marks
        .iter()
        .filter_map(|mark| match &mark.geometry {
            Geometry::Arcs(arcs) => arcs
                .first()
                .map(|arc| (arc.start, arc.start + arc.sweep, rgb(mark.color))),
            _ => None,
        })
        .collect();

    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            let distance = dx.hypot(dy);
            if distance > radius || distance < inner {
                continue;
            }
            // Turns clockwise from twelve o'clock, matching the arc order.
            let mut turn = (dy.atan2(dx) + std::f64::consts::FRAC_PI_2) / std::f64::consts::TAU;
            turn -= turn.floor();
            if let Some(&(_, _, color)) = arcs
                .iter()
                .find(|&&(start, end, _)| turn >= start && turn < end)
            {
                set_pixel(&mut rgba, width, height, x as i32, y as i32, color);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Domain;

    #[test]
    fn every_value_lands_in_the_cell_row_the_cell_renderer_uses() {
        // The label for a value is printed on the row `PlotModel::map` gives
        // it at cell resolution; the pixel for the same value has to fall
        // inside that row, or the label points somewhere else.
        let model = PlotModel {
            x: Domain::new(0.0, 32.0).unwrap(),
            y: Domain::new(0.0, 300.0).unwrap(),
            horizontal: false,
        };
        for data_rows in [3u32, 7, 13, 28, 41] {
            let data_cols = 40;
            let frame = Frame::new(&model, data_cols, data_rows);
            for step in 0..=60 {
                let value = f64::from(step) * 5.0;
                let row = model.map(model.value_point(value), data_cols, data_rows).1;
                let y = frame.at(model.value_point(value)).1;
                let top = row * PIXELS_PER_ROW as i32;
                assert!(
                    (top..top + PIXELS_PER_ROW as i32).contains(&y),
                    "{value} at pixel {y}, label row {row} ({data_rows} rows)"
                );
            }
        }
    }

    #[test]
    fn fills_rest_on_the_axis_line() {
        let model = PlotModel {
            x: Domain::new(0.0, 1.0).unwrap(),
            y: Domain::new(0.0, 1.0).unwrap(),
            horizontal: false,
        };
        let frame = Frame::new(&model, 10, 10);
        assert_eq!(frame.floor(Point::new(0.5, 0.0)).1, frame.baseline_y - 1);
        assert!(frame.at(Point::new(0.5, 0.0)).1 < frame.baseline_y - 1);
    }
}
