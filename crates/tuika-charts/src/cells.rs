use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use ratatui_core::symbols::pixel::QUADRANTS;
use tuika::{RenderCtx, Surface};

use crate::{Chart, PlotModel, Point, Series, SeriesKind, chart_color, draw_line, plot_rect};

pub(super) fn render_cells(
    area: Rect,
    surface: &mut Surface,
    chart: &Chart,
    model: &PlotModel,
    ctx: &RenderCtx,
) {
    surface.fill(Style::default().bg(ctx.theme.background));
    if !chart.title.is_empty() {
        surface.set_string(area.x, area.y, &chart.title, ctx.theme.accent_style());
    }
    let plot = plot_rect(area, chart);
    if plot.width == 0 || plot.height == 0 {
        return;
    }
    let axis = Style::default().fg(ctx.theme.border);
    for y in plot.y..plot.bottom() {
        surface.set(plot.x, y, '│', axis);
    }
    for x in plot.x..plot.right() {
        surface.set(x, plot.bottom() - 1, '─', axis);
    }
    surface.set(plot.x, plot.bottom() - 1, '└', axis);

    let data_width = plot.width.saturating_sub(1) as u32;
    let data_height = plot.height.saturating_sub(1) as u32;
    if data_width == 0 || data_height == 0 {
        return;
    }
    for (index, series) in chart.series.iter().enumerate() {
        let color = chart_color(series, index, ctx);
        let style = Style::default().fg(color);
        match series.kind {
            SeriesKind::Line | SeriesKind::Step => {
                let mapped = map_finite_points(
                    series,
                    model,
                    data_width.saturating_mul(2),
                    data_height.saturating_mul(2),
                );
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                draw_quadrant_polyline(
                    &mut quadrants,
                    &mapped,
                    matches!(series.kind, SeriesKind::Step),
                );
                quadrants.render(surface, plot, style);
            }
            SeriesKind::Scatter => {
                let mapped = map_finite_points(
                    series,
                    model,
                    data_width.saturating_mul(2),
                    data_height.saturating_mul(4),
                );
                let mut braille = BrailleGrid::new(data_width, data_height);
                for (x, y) in mapped {
                    braille.set(x, y);
                }
                braille.render(surface, plot, style);
            }
            SeriesKind::Area => {
                let edge = map_finite_points(
                    series,
                    model,
                    data_width.saturating_mul(2),
                    data_height.saturating_mul(2),
                );
                let baseline = cell_baseline(
                    model,
                    data_width.saturating_mul(2),
                    data_height.saturating_mul(2),
                );
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                draw_quadrant_area(&mut quadrants, &edge, baseline);
                quadrants.render(surface, plot, style);
            }
            SeriesKind::Bar => {
                let mapped = map_finite_points(series, model, data_width, data_height);
                let baseline = cell_baseline(model, data_width, data_height);
                for (x, y) in mapped {
                    for row in y.min(baseline)..=y.max(baseline) {
                        set_plot(surface, plot, x, row, '█', style);
                    }
                }
            }
        }
    }
    render_legend(area, surface, chart, ctx);
}

fn map_finite_points(
    series: &Series,
    model: &PlotModel,
    width: u32,
    height: u32,
) -> Vec<(i32, i32)> {
    series
        .points
        .iter()
        .copied()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| model.map(point, width, height))
        .collect()
}

fn cell_baseline(model: &PlotModel, width: u32, height: u32) -> i32 {
    model
        .map(Point::new(model.x.min, 0.0), width, height)
        .1
        .clamp(0, height as i32)
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

pub(crate) fn draw_quadrant_area(grid: &mut QuadrantGrid, points: &[(i32, i32)], baseline: i32) {
    for pair in points.windows(2) {
        draw_line(pair[0], pair[1], |x, y| {
            for row in y.min(baseline)..=y.max(baseline) {
                grid.set(x, row);
            }
        });
    }
    if let Some(&(x, y)) = points.first() {
        for row in y.min(baseline)..=y.max(baseline) {
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
