use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Style};
use ratatui_core::symbols::pixel::QUADRANTS;
use tuika::{RenderCtx, Surface};

use crate::axis::AxisLayout;
use crate::plan::{ChartPlan, Geometry, Mark};
use crate::{chart_color, draw_line, trim_number, Chart, PlotModel, Point};

pub(super) fn render_cells(
    plot: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    layout: &AxisLayout,
    ctx: &RenderCtx,
) {
    if plot.width == 0 || plot.height == 0 {
        return;
    }
    let model = &plan.model;
    let axis = Style::default().fg(ctx.theme.border);
    for y in plot.y..plot.bottom() {
        surface.set(plot.x, y, '│', axis);
    }
    for x in plot.x..plot.right() {
        surface.set(x, plot.bottom() - 1, '─', axis);
    }
    // Tick marks before the corner, so a tick on the origin cannot erase it.
    for tick in &layout.y {
        surface.set(plot.x, tick.at, '┤', axis);
    }
    for tick in &layout.x {
        surface.set(tick.at, plot.bottom() - 1, '┬', axis);
    }
    surface.set(plot.x, plot.bottom() - 1, '└', axis);

    let data_width = plot.width.saturating_sub(1) as u32;
    let data_height = plot.height.saturating_sub(1) as u32;
    if data_width == 0 || data_height == 0 {
        return;
    }
    render_zero_rule(plot, surface, model, data_width, data_height, ctx);

    for mark in &plan.marks {
        let style = Style::default().fg(mark.color);
        match &mark.geometry {
            Geometry::Path { points, stepped } => {
                let mapped = map_all(points, model, data_width * 2, data_height * 2);
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                draw_quadrant_polyline(&mut quadrants, &mapped, *stepped);
                quadrants.render(surface, plot, style);
            }
            Geometry::Points(points) => {
                let mapped = map_all(points, model, data_width * 2, data_height * 4);
                let mut braille = BrailleGrid::new(data_width, data_height);
                for (x, y) in mapped {
                    braille.set(x, y);
                }
                braille.render(surface, plot, style);
            }
            Geometry::Band { upper, lower } => {
                let edge = map_all(upper, model, data_width * 2, data_height * 2);
                let floor = map_all(lower, model, data_width * 2, data_height * 2);
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                draw_quadrant_band(&mut quadrants, &edge, &floor);
                quadrants.render(surface, plot, style);
            }
            Geometry::Bars { bars, .. } => {
                for bar in bars {
                    // A bar is a rectangle in data space; painting its mapped
                    // corners keeps grouped slots and stacked segments exact
                    // without the renderer knowing either rule.
                    let (x0, y0) = model.map(Point::new(bar.x0, bar.y0), data_width, data_height);
                    let (x1, y1) = model.map(Point::new(bar.x1, bar.y1), data_width, data_height);
                    let (lo, hi) = half_open(x0, x1, y0, y1, model.horizontal);
                    for x in lo.0..=hi.0 {
                        for y in lo.1..=hi.1 {
                            set_plot(surface, plot, x, y, '█', style);
                        }
                    }
                }
            }
            Geometry::Radar { values, labels } => {
                render_radar_cells(plot, surface, values, labels, style, ctx);
            }
            Geometry::Arcs(_) => {}
        }
        if mark.markers {
            // A whole glyph, not a Braille dot: a dot beside a quadrant stroke
            // reads as an artifact of the line rather than as a sample.
            for sample in &mark.samples {
                let (x, y) = model.map(*sample, data_width, data_height);
                set_plot(surface, plot, x, y, '●', style);
            }
        }
    }
    render_overlays(plot, surface, chart, plan, ctx, true);
}

pub(super) fn render_radar_cells(
    plot: Rect,
    surface: &mut Surface,
    values: &[f64],
    labels: &[String],
    style: Style,
    ctx: &RenderCtx,
) {
    if values.len() < 3 || plot.width < 20 || plot.height < 9 {
        return;
    }
    let center = (
        i32::from(plot.x) + i32::from(plot.width) / 2,
        i32::from(plot.y) + i32::from(plot.height) / 2,
    );
    let label_margin = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0)
        .saturating_add(2)
        .min(usize::from(plot.width / 3)) as u16;
    let radius_x =
        (f64::from(plot.width.saturating_sub(label_margin.saturating_mul(2))) / 2.0).max(2.0);
    let radius_y = (f64::from(plot.height.saturating_sub(4)) / 2.0).max(2.0);
    let radial = |index: usize, scale: f64| {
        let angle = -std::f64::consts::FRAC_PI_2
            + std::f64::consts::TAU * index as f64 / values.len() as f64;
        (
            center.0 + (radius_x * scale * angle.cos()).round() as i32,
            center.1 + (radius_y * scale * angle.sin()).round() as i32,
        )
    };

    // The web is deliberately low contrast so the data silhouette reads first.
    for level in 1..=4 {
        let ring: Vec<_> = (0..values.len())
            .map(|index| radial(index, level as f64 / 4.0))
            .collect();
        draw_text_polyline(surface, &closed(&ring), '·', ctx.theme.border);
    }
    for index in 0..values.len() {
        draw_text_polyline(
            surface,
            &[center, radial(index, 1.0)],
            '·',
            ctx.theme.border,
        );
    }

    let data: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(index, value)| radial(index, (*value / 100.0).clamp(0.0, 1.0)))
        .collect();
    fill_text_polygon(surface, &data, '░', style.fg.unwrap_or(ctx.theme.accent));
    draw_text_polyline(
        surface,
        &closed(&data),
        '●',
        style.fg.unwrap_or(ctx.theme.accent),
    );

    for (index, label) in labels.iter().take(values.len()).enumerate() {
        let (anchor_x, anchor_y) = radial(index, 1.08);
        let width = label.chars().count() as i32;
        let x = if anchor_x < center.0 {
            anchor_x - width
        } else if anchor_x > center.0 {
            anchor_x + 1
        } else {
            anchor_x - width / 2
        };
        let y = if anchor_y < center.1 {
            anchor_y - 1
        } else if anchor_y > center.1 {
            anchor_y + 1
        } else {
            anchor_y
        };
        if x >= i32::from(plot.x)
            && y >= i32::from(plot.y)
            && x + width <= i32::from(plot.right())
            && y < i32::from(plot.bottom())
        {
            surface.set_string(
                x as u16,
                y as u16,
                label,
                Style::default().fg(ctx.theme.text),
            );
        }
    }
}

fn draw_text_polyline(surface: &mut Surface, points: &[(i32, i32)], glyph: char, color: Color) {
    for pair in points.windows(2) {
        draw_text_line(surface, pair[0], pair[1], glyph, color);
    }
}

fn draw_text_line(
    surface: &mut Surface,
    (mut x0, mut y0): (i32, i32),
    (x1, y1): (i32, i32),
    glyph: char,
    color: Color,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if x0 >= 0 && y0 >= 0 {
            surface.set(x0 as u16, y0 as u16, glyph, Style::default().fg(color));
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice_error = error * 2;
        if twice_error >= dy {
            error += dy;
            x0 += sx;
        }
        if twice_error <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn fill_text_polygon(surface: &mut Surface, points: &[(i32, i32)], glyph: char, color: Color) {
    if points.len() < 3 {
        return;
    }
    let min_y = points.iter().map(|point| point.1).min().unwrap_or(0);
    let max_y = points.iter().map(|point| point.1).max().unwrap_or(-1);
    for y in min_y..=max_y {
        let mut xs = Vec::new();
        for index in 0..points.len() {
            let (x1, y1) = points[index];
            let (x2, y2) = points[(index + 1) % points.len()];
            if (y1 <= y && y < y2) || (y2 <= y && y < y1) {
                xs.push(
                    x1 + ((y - y1) as f64 * (x2 - x1) as f64 / (y2 - y1) as f64).round() as i32,
                );
            }
        }
        xs.sort_unstable();
        for pair in xs.chunks_exact(2) {
            for x in pair[0]..=pair[1] {
                if x >= 0 && y >= 0 {
                    surface.set(x as u16, y as u16, glyph, Style::default().fg(color));
                }
            }
        }
    }
}

fn closed(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    points
        .iter()
        .copied()
        .chain(points.first().copied())
        .collect()
}

/// Draw a rule along zero when the value domain straddles it, so negative
/// geometry is read against the baseline rather than against the frame.
///
/// Values always live on the chart's y axis; only which way the rule runs
/// depends on orientation.
fn render_zero_rule(
    plot: Rect,
    surface: &mut Surface,
    model: &PlotModel,
    data_width: u32,
    data_height: u32,
    ctx: &RenderCtx,
) {
    if !(model.y.min < 0.0 && model.y.max > 0.0) {
        return;
    }
    let style = Style::default().fg(ctx.theme.border);
    if model.horizontal {
        let column = model.map(model.across_at(0.0), data_width, data_height).0;
        for row in 0..data_height as i32 {
            set_plot_dim(surface, plot, column, row, '│', style);
        }
    } else {
        let row = model.map(model.up_at(0.0), data_width, data_height).1;
        for column in 0..data_width as i32 {
            set_plot_dim(surface, plot, column, row, '─', style);
        }
    }
}

/// Value labels and the focus rule. Text over an image for the same reason the
/// tick labels are: cells stay crisp and identical in both render modes.
pub(super) fn render_overlays(
    plot: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    ctx: &RenderCtx,
    focus_rule: bool,
) {
    let data_width = plot.width.saturating_sub(1) as u32;
    let data_height = plot.height.saturating_sub(1) as u32;
    if data_width == 0 || data_height == 0 {
        return;
    }
    let model = &plan.model;

    if let Some(focus) = chart.focus.filter(|_| focus_rule) {
        let style = Style::default().fg(ctx.theme.accent);
        if model.horizontal {
            let row = model.map(model.up_at(focus), data_width, data_height).1;
            for column in 0..data_width as i32 {
                set_plot_dim(surface, plot, column, row, '─', style);
            }
        } else {
            let column = model.map(model.across_at(focus), data_width, data_height).0;
            for row in 0..data_height as i32 {
                set_plot_dim(surface, plot, column, row, '│', style);
            }
        }
    }

    for mark in plan.marks.iter().filter(|mark| mark.labelled) {
        render_value_labels(plot, surface, model, mark, data_width, data_height, ctx);
    }
}

/// Print each sample's value beside its mark.
///
/// A label annotates a reading, so it may never cover one: each is tried in a
/// few positions around its mark and dropped if none is clear. Dropping is the
/// right failure — a label sitting on the geometry it describes is worse than
/// no label at all.
fn render_value_labels(
    plot: Rect,
    surface: &mut Surface,
    model: &PlotModel,
    mark: &Mark,
    data_width: u32,
    data_height: u32,
    ctx: &RenderCtx,
) {
    let style = ctx.theme.muted_style();
    let mut taken: Vec<(u16, u16, u16)> = Vec::new();
    for sample in &mark.samples {
        let text = trim_number(sample.y);
        let Ok(len) = u16::try_from(text.chars().count()) else {
            continue;
        };
        let (x, y) = model.map(*sample, data_width, data_height);
        if x < 0 || y < 0 {
            continue;
        }
        let (col, row) = (plot.x + 1 + x as u16, plot.y + y as u16);
        let centered = col.saturating_sub(len / 2);
        let candidates: [(u16, u16); 4] = if model.horizontal {
            // Past the bar's end first, then just inside it.
            [
                (col + 1, row),
                (col.saturating_sub(len + 1), row),
                (col + 1, row.saturating_sub(1)),
                (col + 1, row + 1),
            ]
        } else {
            [
                (centered, row.saturating_sub(1)),
                (centered, row + 1),
                (col + 1, row),
                (col.saturating_sub(len + 1), row),
            ]
        };
        for (col, row) in candidates {
            if col < plot.x + 1
                || col + len > plot.right()
                || row < plot.y
                || row >= plot.bottom() - 1
            {
                continue;
            }
            if taken
                .iter()
                .any(|&(r, start, end)| r == row && col < end && start < col + len)
            {
                continue;
            }
            if (col..col + len).any(|x| !is_blank(surface, x, row)) {
                continue;
            }
            taken.push((row, col, col + len));
            surface.set_string(col, row, &text, style);
            break;
        }
    }
}

/// A donut: one ring of arcs with optional centre text.
pub(super) fn render_donut(
    body: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    ctx: &RenderCtx,
) {
    if body.width < 3 || body.height < 3 {
        return;
    }
    // Cells are about twice as tall as they are wide, so a ring drawn on a
    // square grid of cells would read as an ellipse. Scaling x by two restores
    // the circle a reader expects.
    let radius = f64::from(body.height.min(body.width / 2)) / 2.0 - 0.5;
    if radius < 1.0 {
        return;
    }
    let cx = f64::from(body.x) + f64::from(body.width) / 2.0;
    let cy = f64::from(body.y) + f64::from(body.height) / 2.0;
    let thickness = (radius * 0.42).max(1.0);

    for mark in &plan.marks {
        let Geometry::Arcs(arcs) = &mark.geometry else {
            continue;
        };
        let style = Style::default().fg(mark.color);
        for arc in arcs {
            // Sample densely enough that the thinnest ring has no gaps.
            let steps = ((arc.sweep * radius * 24.0).ceil() as usize).max(2);
            for step in 0..=steps {
                let turn = arc.start + arc.sweep * step as f64 / steps as f64;
                let angle = turn * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
                let (sin, cos) = angle.sin_cos();
                let mut inward = 0.0;
                while inward <= thickness {
                    let r = radius - inward;
                    let x = cx + cos * r * 2.0;
                    let y = cy + sin * r;
                    if x >= f64::from(body.x)
                        && y >= f64::from(body.y)
                        && x < f64::from(body.right())
                        && y < f64::from(body.bottom())
                    {
                        surface.set(x as u16, y as u16, '█', style);
                    }
                    inward += 0.5;
                }
            }
        }
    }

    render_center(body, surface, chart, ctx);
}

/// The donut's centre text. Cells in both modes, like every other label.
pub(super) fn render_center(body: Rect, surface: &mut Surface, chart: &Chart, ctx: &RenderCtx) {
    if chart.center.is_empty() || body.width == 0 || body.height == 0 {
        return;
    }
    let Ok(len) = u16::try_from(chart.center.chars().count()) else {
        return;
    };
    let x = (body.x + body.width / 2).saturating_sub(len / 2);
    let y = body.y + body.height / 2;
    if x + len <= body.right() && y < body.bottom() {
        surface.set_string(x, y, &chart.center, ctx.theme.accent_style());
    }
}

/// A bar's cell span, half-open across the category axis.
///
/// A band's gutter can be under a cell wide, and two bars rounded to touching
/// columns read as one wide bar. Dropping the far column keeps the gap the
/// grouping intended, at the cost of a cell of width no reader can miss.
fn half_open(x0: i32, x1: i32, y0: i32, y1: i32, horizontal: bool) -> ((i32, i32), (i32, i32)) {
    let (lox, mut hix) = (x0.min(x1), x0.max(x1));
    let (loy, mut hiy) = (y0.min(y1), y0.max(y1));
    // Shrink across the category axis only: the value direction carries the
    // reading, and taking a cell off it would understate the bar.
    if horizontal {
        hiy = hiy.max(loy + 1) - 1;
    } else {
        hix = hix.max(lox + 1) - 1;
    }
    ((lox, loy), (hix, hiy))
}

fn map_all(points: &[Point], model: &PlotModel, width: u32, height: u32) -> Vec<(i32, i32)> {
    points
        .iter()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| model.map(*point, width, height))
        .collect()
}

fn set_plot(surface: &mut Surface, plot: Rect, x: i32, y: i32, ch: char, style: Style) {
    if x >= 0
        && y >= 0
        && x < plot.width.saturating_sub(1) as i32
        && y < plot.height.saturating_sub(1) as i32
    {
        surface.set(plot.x + 1 + x as u16, plot.y + y as u16, ch, style);
    }
}

/// Like [`set_plot`] but never paints over data already drawn there.
fn set_plot_dim(surface: &mut Surface, plot: Rect, x: i32, y: i32, ch: char, style: Style) {
    if x < 0 || y < 0 {
        return;
    }
    let (col, row) = (plot.x + 1 + x as u16, plot.y + y as u16);
    if x >= plot.width.saturating_sub(1) as i32 || y >= plot.height.saturating_sub(1) as i32 {
        return;
    }
    if !is_blank(surface, col, row) {
        return;
    }
    surface.set(col, row, ch, style);
}

/// Whether a cell still holds background, so a rule can be drawn there.
///
/// Rules are reference lines, not data: they must never paint over a mark the
/// reader is trying to measure against them.
fn is_blank(surface: &mut Surface, x: u16, y: u16) -> bool {
    let clip = surface.area();
    if x < clip.x || y < clip.y || x >= clip.right() || y >= clip.bottom() {
        return false;
    }
    let buffer = surface.buffer_mut();
    let area = buffer.area;
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return false;
    }
    buffer[(x, y)].symbol().trim().is_empty()
}

/// Fill between an upper edge and the floor it rests on.
pub(crate) fn draw_quadrant_band(
    grid: &mut QuadrantGrid,
    upper: &[(i32, i32)],
    lower: &[(i32, i32)],
) {
    let floor_at = |index: usize| lower.get(index).map_or(0, |&(_, y)| y);
    for (index, pair) in upper.windows(2).enumerate() {
        let base = floor_at(index).max(floor_at(index + 1));
        draw_line(pair[0], pair[1], |x, y| {
            for row in y.min(base)..=y.max(base) {
                grid.set(x, row);
            }
        });
    }
    if let Some(&(x, y)) = upper.first() {
        let base = floor_at(0);
        for row in y.min(base)..=y.max(base) {
            grid.set(x, row);
        }
    }
}

pub(crate) struct BrailleGrid {
    cols: u32,
    rows: u32,
    pub(crate) masks: Vec<u8>,
}

impl BrailleGrid {
    pub(crate) fn new(cols: u32, rows: u32) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; (cols as usize).saturating_mul(rows as usize)],
        }
    }

    pub(crate) fn set(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= (self.cols * 2) as i32 || y >= (self.rows * 4) as i32 {
            return;
        }
        const DOTS: [[u8; 2]; 4] = [
            [0b0000_0001, 0b0000_1000],
            [0b0000_0010, 0b0001_0000],
            [0b0000_0100, 0b0010_0000],
            [0b0100_0000, 0b1000_0000],
        ];
        let cell_x = x as u32 / 2;
        let cell_y = y as u32 / 4;
        let index = (cell_y * self.cols + cell_x) as usize;
        self.masks[index] |= DOTS[y as usize % 4][x as usize % 2];
    }

    fn render(self, surface: &mut Surface, plot: Rect, style: Style) {
        for (index, mask) in self.masks.into_iter().enumerate() {
            if mask == 0 {
                continue;
            }
            let x = index as u32 % self.cols;
            let y = index as u32 / self.cols;
            let glyph = char::from_u32(0x2800 + u32::from(mask)).expect("valid Braille mask");
            set_plot(surface, plot, x as i32, y as i32, glyph, style);
        }
    }
}

pub(crate) struct QuadrantGrid {
    cols: u32,
    rows: u32,
    pub(crate) masks: Vec<u8>,
}

impl QuadrantGrid {
    pub(crate) fn new(cols: u32, rows: u32) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; (cols as usize).saturating_mul(rows as usize)],
        }
    }

    pub(crate) fn set(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= (self.cols * 2) as i32 || y >= (self.rows * 2) as i32 {
            return;
        }
        let cell_x = x as u32 / 2;
        let cell_y = y as u32 / 2;
        let index = (cell_y * self.cols + cell_x) as usize;
        self.masks[index] |= 1 << ((x as usize % 2) + 2 * (y as usize % 2));
    }

    fn render(self, surface: &mut Surface, plot: Rect, style: Style) {
        for (index, mask) in self.masks.into_iter().enumerate() {
            if mask == 0 {
                continue;
            }
            let x = index as u32 % self.cols;
            let y = index as u32 / self.cols;
            set_plot(
                surface,
                plot,
                x as i32,
                y as i32,
                QUADRANTS[mask as usize],
                style,
            );
        }
    }
}

fn draw_quadrant_polyline(grid: &mut QuadrantGrid, points: &[(i32, i32)], stepped: bool) {
    for pair in points.windows(2) {
        if stepped {
            let corner = (pair[1].0, pair[0].1);
            draw_line(pair[0], corner, |x, y| grid.set(x, y));
            draw_line(corner, pair[1], |x, y| grid.set(x, y));
        } else {
            draw_line(pair[0], pair[1], |x, y| grid.set(x, y));
        }
    }
    for &(x, y) in points {
        grid.set(x, y);
    }
}

pub(super) fn render_legend(area: Rect, surface: &mut Surface, chart: &Chart, ctx: &RenderCtx) {
    if !chart.legend || chart.series.is_empty() || area.height == 0 {
        return;
    }
    let mut x = area.x;
    let y = area.bottom() - 1;
    for (index, series) in chart.series.iter().enumerate() {
        let color = chart_color(series, index, ctx);
        surface.set(x, y, '■', Style::default().fg(color));
        x = x.saturating_add(2);
        x = surface.set_string(x, y, &series.name, ctx.theme.muted_style());
        x = x.saturating_add(2);
        if x >= area.right() {
            break;
        }
    }
}
