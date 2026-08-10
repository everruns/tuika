//! Adaptive charts for [`tuika`].
//!
//! [`Chart`] accepts one renderer-independent chart model. On a terminal with
//! a graphics protocol it rasterizes smooth lines and filled bars into an image;
//! everywhere else it renders the same axes, series, domains, and legend with
//! terminal cells, using dense 2×2 quadrant glyphs for connected geometry and
//! Braille subcells for scatter points. [`tuika::Runner`] supplies terminal
//! graphics automatically; custom hosts can provide
//! [`tuika::term::image::ImageSupport`] and an
//! [`tuika::term::image::ImageLayer`] through [`tuika::RenderCtx`].

use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Style};
use ratatui_core::symbols::pixel::QUADRANTS;
use tuika::term::image::{ImageData, ImageLayer, ImageSupport};
use tuika::{RenderCtx, Size, Surface, View};

/// A numeric point in chart coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Horizontal value.
    pub x: f64,
    /// Vertical value.
    pub y: f64,
}

impl Point {
    /// Construct a point.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// The shared chart grammar supported by both adaptive renderers.
#[derive(Clone, Debug, PartialEq)]
pub enum SeriesKind {
    /// Connect points in ascending input order.
    Line,
    /// Draw vertical bars from the zero baseline (or the visible domain edge).
    Bar,
    /// Connect points and fill the region down to the zero baseline.
    Area,
    /// Draw independent point markers without connecting them.
    Scatter,
    /// Connect points with horizontal-then-vertical segments.
    Step,
}

/// One named data series.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// Name used in the legend.
    pub name: String,
    /// Geometry used by both renderers.
    pub kind: SeriesKind,
    /// Data points. Non-finite points are ignored.
    pub points: Vec<Point>,
    /// Explicit series color. The chart palette supplies one when absent.
    pub color: Option<Color>,
}

impl Series {
    /// Construct a line series.
    pub fn line(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Line, points)
    }

    /// Construct a bar series.
    pub fn bar(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Bar, points)
    }

    /// Construct a filled area series.
    pub fn area(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Area, points)
    }

    /// Construct a scatter series.
    pub fn scatter(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Scatter, points)
    }

    /// Construct a stepped line series.
    pub fn step(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Step, points)
    }

    fn new(
        name: impl Into<String>,
        kind: SeriesKind,
        points: impl IntoIterator<Item = Point>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            points: points.into_iter().collect(),
            color: None,
        }
    }

    /// Override the palette color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

/// A fixed numeric domain. Values outside it are clipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    /// Inclusive lower edge.
    pub min: f64,
    /// Inclusive upper edge.
    pub max: f64,
}

impl Domain {
    /// Construct a valid increasing finite domain.
    pub fn new(min: f64, max: f64) -> Option<Self> {
        (min.is_finite() && max.is_finite() && min < max).then_some(Self { min, max })
    }
}

/// Which rendering path a chart used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// High-resolution image emitted through a graphics protocol.
    Graphics,
    /// Unicode cell renderer, available in every terminal.
    Cells,
}

/// An adaptive line, bar, area, scatter, or step chart view.
///
/// | Portable cells | Terminal graphics |
/// | --- | --- |
/// | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-cells.png" alt="Line chart rendered with terminal cells" width="420"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-graphics.png" alt="Line chart rendered with terminal graphics" width="420"> |
pub struct Chart {
    title: String,
    series: Vec<Series>,
    x_domain: Option<Domain>,
    y_domain: Option<Domain>,
    support: Option<ImageSupport>,
    layer: Option<ImageLayer>,
    legend: bool,
}

impl Chart {
    /// Construct an empty chart.
    pub fn new() -> Self {
        Self {
            title: String::new(),
            series: Vec::new(),
            x_domain: None,
            y_domain: None,
            support: None,
            layer: None,
            legend: true,
        }
    }

    /// Set the title rendered above the plot.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Append a series.
    pub fn series(mut self, series: Series) -> Self {
        self.series.push(series);
        self
    }

    /// Use an explicit horizontal domain.
    pub fn x_domain(mut self, domain: Domain) -> Self {
        self.x_domain = Some(domain);
        self
    }

    /// Use an explicit vertical domain.
    pub fn y_domain(mut self, domain: Domain) -> Self {
        self.y_domain = Some(domain);
        self
    }

    /// Show or hide the legend. It is shown by default.
    pub fn legend(mut self, visible: bool) -> Self {
        self.legend = visible;
        self
    }

    /// Override terminal graphics support supplied by the render context.
    pub fn support(mut self, support: ImageSupport) -> Self {
        self.support = Some(support);
        self
    }

    /// Override the image layer supplied by the render context.
    pub fn in_layer(mut self, layer: &ImageLayer) -> Self {
        self.layer = Some(layer.clone());
        self
    }

    /// Resolve the rendering path from explicit chart configuration only.
    pub fn render_mode(&self) -> RenderMode {
        if self.layer.is_some() && self.support.unwrap_or(ImageSupport::None) != ImageSupport::None
        {
            RenderMode::Graphics
        } else {
            RenderMode::Cells
        }
    }

    /// Resolve the rendering path including graphics supplied by the host.
    pub fn render_mode_in(&self, ctx: &RenderCtx<'_>) -> RenderMode {
        let (support, layer) = self.graphics(ctx);
        if layer.is_some() && support != ImageSupport::None {
            RenderMode::Graphics
        } else {
            RenderMode::Cells
        }
    }

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

impl Default for Chart {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Chart {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(available.width.min(80), available.height.min(24))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let Some(model) = PlotModel::new(self) else {
            render_empty(area, surface, ctx, &self.title);
            return;
        };
        let (support, layer) = self.graphics(ctx);
        if self.render_mode_in(ctx) == RenderMode::Graphics {
            surface.fill(Style::default().bg(ctx.theme.background));
            if !self.title.is_empty() {
                surface.set_string(area.x, area.y, &self.title, ctx.theme.accent_style());
            }
            render_legend(area, surface, self, ctx);
            let plot = plot_rect(area, self);
            let data = render_pixels(plot.width, plot.height, self, &model, ctx);
            if let (Some(data), Some(layer)) = (data, layer) {
                // Image owns capability-gated placement; using it here keeps the
                // same protocol lifecycle and fallback semantics as core images.
                tuika::components::Image::new(data, plot.width, plot.height)
                    .support(support)
                    .in_layer(layer)
                    .render(plot, &mut surface.child(plot), ctx);
                return;
            }
        }
        render_cells(area, surface, self, &model, ctx);
    }
}

#[derive(Clone, Copy)]
struct PlotModel {
    x: Domain,
    y: Domain,
}

impl PlotModel {
    fn new(chart: &Chart) -> Option<Self> {
        let points = chart
            .series
            .iter()
            .flat_map(|series| series.points.iter())
            .filter(|point| point.x.is_finite() && point.y.is_finite());
        let (mut xmin, mut xmax, mut ymin, mut ymax) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for point in points {
            xmin = xmin.min(point.x);
            xmax = xmax.max(point.x);
            ymin = ymin.min(point.y);
            ymax = ymax.max(point.y);
        }
        if !xmin.is_finite() {
            return None;
        }
        let x = chart.x_domain.unwrap_or_else(|| {
            let domain = padded_domain(xmin, xmax);
            pad_automatic_bar_domain(chart, domain)
        });
        let y = chart.y_domain.unwrap_or_else(|| padded_domain(ymin, ymax));
        Some(Self { x, y })
    }

    fn map(&self, point: Point, width: u32, height: u32) -> (i32, i32) {
        // Clamp before converting to integer coordinates. Besides defining the
        // public clipping behavior, this bounds line work for extreme finite
        // input values against a small explicit domain.
        let normalized_x = ((point.x - self.x.min) / (self.x.max - self.x.min)).clamp(0.0, 1.0);
        let normalized_y = ((point.y - self.y.min) / (self.y.max - self.y.min)).clamp(0.0, 1.0);
        let x = (normalized_x * width.saturating_sub(1) as f64).round() as i32;
        let y = (height.saturating_sub(1) as f64 - normalized_y * height.saturating_sub(1) as f64)
            .round() as i32;
        (x, y)
    }
}

fn pad_automatic_bar_domain(chart: &Chart, domain: Domain) -> Domain {
    let mut positions = chart
        .series
        .iter()
        .filter(|series| matches!(series.kind, SeriesKind::Bar))
        .flat_map(|series| series.points.iter())
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| point.x)
        .collect::<Vec<_>>();
    positions.sort_by(f64::total_cmp);
    positions.dedup();
    let spacing = positions
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|spacing| spacing.is_finite() && *spacing > 0.0)
        .min_by(f64::total_cmp);
    let Some(pad) = spacing.map(|spacing| spacing / 2.0) else {
        return domain;
    };
    let (min, max) = (domain.min - pad, domain.max + pad);
    if min.is_finite() && max.is_finite() {
        Domain { min, max }
    } else {
        domain
    }
}

fn padded_domain(min: f64, max: f64) -> Domain {
    if min < max {
        Domain { min, max }
    } else {
        let pad = if min == 0.0 { 1.0 } else { min.abs() * 0.1 };
        Domain {
            min: min - pad,
            max: max + pad,
        }
    }
}

fn render_empty(area: Rect, surface: &mut Surface, ctx: &RenderCtx, title: &str) {
    surface.fill(Style::default().bg(ctx.theme.background));
    if !title.is_empty() {
        surface.set_string(area.x, area.y, title, ctx.theme.accent_style());
    }
    let y = area.y + usize::from(!title.is_empty()) as u16;
    if y < area.bottom() {
        surface.set_string(area.x, y, "No chart data", ctx.theme.muted_style());
    }
}

fn chart_color(series: &Series, index: usize, ctx: &RenderCtx) -> Color {
    series.color.unwrap_or(match index % 4 {
        0 => ctx.theme.accent,
        1 => ctx.theme.accent_alt,
        2 => ctx.theme.code.link,
        _ => ctx.theme.code.string,
    })
}

fn plot_rect(area: Rect, chart: &Chart) -> Rect {
    let title = u16::from(!chart.title.is_empty());
    let legend = u16::from(chart.legend && !chart.series.is_empty());
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(title),
        area.width.saturating_sub(2),
        area.height.saturating_sub(title + legend + 1),
    )
}

fn render_cells(
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

fn draw_quadrant_area(grid: &mut QuadrantGrid, points: &[(i32, i32)], baseline: i32) {
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

struct BrailleGrid {
    cols: u32,
    rows: u32,
    masks: Vec<u8>,
}

impl BrailleGrid {
    fn new(cols: u32, rows: u32) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; (cols as usize).saturating_mul(rows as usize)],
        }
    }

    fn set(&mut self, x: i32, y: i32) {
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

struct QuadrantGrid {
    cols: u32,
    rows: u32,
    masks: Vec<u8>,
}

impl QuadrantGrid {
    fn new(cols: u32, rows: u32) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; (cols as usize).saturating_mul(rows as usize)],
        }
    }

    fn set(&mut self, x: i32, y: i32) {
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

fn render_legend(area: Rect, surface: &mut Surface, chart: &Chart, ctx: &RenderCtx) {
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

const PIXELS_PER_COL: u32 = 8;
const PIXELS_PER_ROW: u32 = 16;

fn render_pixels(
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

fn draw_line(mut from: (i32, i32), to: (i32, i32), mut draw: impl FnMut(i32, i32)) {
    let dx = (to.0 - from.0).abs();
    let sx = if from.0 < to.0 { 1 } else { -1 };
    let dy = -(to.1 - from.1).abs();
    let sy = if from.1 < to.1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        draw(from.0, from.1);
        if from == to {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            from.0 += sx;
        }
        if twice <= dx {
            error += dx;
            from.1 += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::buffer::Buffer;
    use tuika::Theme;

    fn is_braille(ch: char) -> bool {
        ('\u{2801}'..='\u{28ff}').contains(&ch)
    }

    fn is_quadrant(ch: char) -> bool {
        ratatui_core::symbols::pixel::QUADRANTS[1..].contains(&ch)
    }

    fn sample() -> Chart {
        Chart::new()
            .title("Traffic")
            .series(Series::line(
                "requests",
                [
                    Point::new(0.0, 2.0),
                    Point::new(1.0, 5.0),
                    Point::new(2.0, 3.0),
                ],
            ))
            .series(Series::bar(
                "errors",
                [
                    Point::new(0.0, 1.0),
                    Point::new(1.0, 2.0),
                    Point::new(2.0, 1.0),
                ],
            ))
    }

    #[test]
    fn domain_rejects_degenerate_and_non_finite_ranges() {
        assert_eq!(Domain::new(0.0, 1.0), Some(Domain { min: 0.0, max: 1.0 }));
        assert_eq!(Domain::new(1.0, 1.0), None);
        assert_eq!(Domain::new(f64::NAN, 1.0), None);
    }

    #[test]
    fn graphics_requires_capability_and_layer() {
        let layer = ImageLayer::new();
        assert_eq!(
            sample().support(ImageSupport::Kitty).render_mode(),
            RenderMode::Cells
        );
        assert_eq!(sample().in_layer(&layer).render_mode(), RenderMode::Cells);
        assert_eq!(
            sample()
                .support(ImageSupport::Kitty)
                .in_layer(&layer)
                .render_mode(),
            RenderMode::Graphics
        );
    }

    #[test]
    fn chart_uses_graphics_from_the_render_context() {
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let ctx = RenderCtx::new(&theme).with_image_graphics(ImageSupport::Kitty, &layer);
        let chart = sample();

        assert_eq!(chart.render_mode(), RenderMode::Cells);
        assert_eq!(chart.render_mode_in(&ctx), RenderMode::Graphics);
        tuika::testing::render_with_context(&chart, 40, 12, &ctx);
        assert_eq!(layer.len(), 1);
    }

    #[test]
    fn cell_renderer_draws_title_axes_series_and_legend() {
        let area = Rect::new(0, 0, 32, 10);
        let mut buffer = Buffer::empty(area);
        sample().render(
            area,
            &mut Surface::new(&mut buffer, area),
            &RenderCtx::new(&Theme::default()),
        );
        let grid = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(grid.contains("Traffic"));
        assert!(grid.contains('└'));
        assert!(grid.chars().any(is_quadrant));
        assert!(grid.contains('█'));
        assert!(grid.contains("requests"));
    }

    #[test]
    fn cell_lines_use_dense_quadrant_subcells() {
        let area = Rect::new(0, 0, 10, 6);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .legend(false)
            .x_domain(Domain::new(0.0, 1.0).unwrap())
            .y_domain(Domain::new(0.0, 1.0).unwrap())
            .series(Series::line(
                "line",
                [Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
            ))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );

        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.symbol().chars().next().is_some_and(is_quadrant)),
            "portable lines should use dense quadrant subcells"
        );
        assert!(
            !buffer
                .content
                .iter()
                .any(|cell| cell.symbol().chars().next().is_some_and(is_braille)),
            "connected lines should not expose separated Braille dots"
        );
    }

    #[test]
    fn cell_renderer_uses_documented_theme_slots() {
        let area = Rect::new(0, 0, 32, 10);
        let theme = Theme::default();
        let mut buffer = Buffer::empty(area);
        sample().render(
            area,
            &mut Surface::new(&mut buffer, area),
            &RenderCtx::new(&theme),
        );

        assert_eq!(buffer[(0, 0)].fg, theme.accent, "title uses accent");
        assert_eq!(buffer[(1, 1)].fg, theme.border, "axis uses border");
        assert_eq!(
            buffer[(0, 9)].fg,
            theme.accent,
            "first legend mark uses series color"
        );
        assert_eq!(
            buffer[(2, 9)].fg,
            theme.muted,
            "legend label uses muted text"
        );
    }

    #[test]
    fn additional_cell_series_have_distinct_marks() {
        let cases = [
            (
                Series::area("area", [Point::new(0.0, 1.0), Point::new(1.0, 3.0)]),
                false,
            ),
            (
                Series::scatter("scatter", [Point::new(0.0, 1.0), Point::new(1.0, 3.0)]),
                true,
            ),
        ];
        for (series, expected_braille) in cases {
            let area = Rect::new(0, 0, 24, 8);
            let mut buffer = Buffer::empty(area);
            Chart::new().series(series).render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
            let has_braille = buffer
                .content
                .iter()
                .any(|cell| cell.symbol().chars().next().is_some_and(is_braille));
            assert_eq!(has_braille, expected_braille);
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|cell| cell
                        .symbol()
                        .chars()
                        .next()
                        .is_some_and(if expected_braille {
                            is_braille
                        } else {
                            is_quadrant
                        }))
            );
        }
    }

    #[test]
    fn quadrant_area_stops_at_the_edge_subcell() {
        let mut grid = QuadrantGrid::new(1, 2);
        draw_quadrant_area(&mut grid, &[(0, 1), (1, 1)], 3);
        assert_eq!(
            grid.masks,
            [0b1100, 0b1111],
            "fill should include every subcell below the edge and none above it"
        );
    }

    #[test]
    fn braille_grid_packs_all_eight_subcells() {
        let mut grid = BrailleGrid::new(1, 1);
        for y in 0..4 {
            for x in 0..2 {
                grid.set(x, y);
            }
        }
        assert_eq!(grid.masks, [u8::MAX]);
    }

    #[test]
    fn quadrant_grid_packs_all_four_subcells() {
        let mut grid = QuadrantGrid::new(1, 1);
        for y in 0..2 {
            for x in 0..2 {
                grid.set(x, y);
            }
        }
        assert_eq!(grid.masks, [0b1111]);
    }

    #[test]
    fn area_fills_columns_between_sparse_points() {
        let area = Rect::new(0, 0, 16, 8);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .legend(false)
            .series(Series::area(
                "area",
                [Point::new(0.0, 1.0), Point::new(10.0, 3.0)],
            ))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        assert!(
            (5..10).any(|x| {
                (0..area.height).any(|y| {
                    buffer[(x, y)]
                        .symbol()
                        .chars()
                        .next()
                        .is_some_and(is_quadrant)
                })
            }),
            "area must fill between data points rather than draw isolated columns"
        );
    }

    #[test]
    fn step_series_uses_horizontal_then_vertical_segments() {
        let area = Rect::new(0, 0, 12, 7);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .legend(false)
            .x_domain(Domain::new(0.0, 1.0).unwrap())
            .y_domain(Domain::new(0.0, 1.0).unwrap())
            .series(Series::step(
                "step",
                [Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
            ))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        let grid = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(
            grid.lines()
                .any(|line| line.chars().filter(|&ch| is_quadrant(ch)).count() >= 5)
        );
    }

    #[test]
    fn every_series_kind_records_graphics() {
        let points = [Point::new(0.0, 1.0), Point::new(1.0, 3.0)];
        let series = [
            Series::line("line", points),
            Series::bar("bar", points),
            Series::area("area", points),
            Series::scatter("scatter", points),
            Series::step("step", points),
        ];
        for series in series {
            let area = Rect::new(0, 0, 20, 8);
            let mut buffer = Buffer::empty(area);
            let layer = ImageLayer::new();
            Chart::new()
                .series(series)
                .support(ImageSupport::Kitty)
                .in_layer(&layer)
                .render(
                    area,
                    &mut Surface::new(&mut buffer, area),
                    &RenderCtx::new(&Theme::default()),
                );
            assert_eq!(layer.len(), 1);
        }
    }

    #[test]
    fn graphics_renderer_records_plot_while_title_and_legend_stay_cells() {
        let area = Rect::new(0, 0, 20, 8);
        let mut buffer = Buffer::empty(area);
        let layer = ImageLayer::new();
        sample()
            .support(ImageSupport::Kitty)
            .in_layer(&layer)
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        assert_eq!(layer.len(), 1);
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Traffic"));
        assert!(text.contains("requests"));
    }

    #[test]
    fn empty_and_non_finite_data_render_a_safe_message() {
        let area = Rect::new(0, 0, 20, 3);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .title("Empty")
            .series(Series::line("bad", [Point::new(f64::NAN, 1.0)]))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("No chart data"));
    }

    #[test]
    fn explicit_domains_clip_extreme_values_to_plot_bounds() {
        let chart = Chart::new()
            .x_domain(Domain::new(0.0, 1.0).unwrap())
            .y_domain(Domain::new(0.0, 1.0).unwrap())
            .series(Series::line(
                "extreme",
                [
                    Point::new(-f64::MAX, f64::MAX),
                    Point::new(f64::MAX, -f64::MAX),
                ],
            ));
        let model = PlotModel::new(&chart).unwrap();
        assert_eq!(model.map(chart.series[0].points[0], 20, 10), (0, 0));
        assert_eq!(model.map(chart.series[0].points[1], 20, 10), (19, 9));
    }

    #[test]
    fn tiny_sizes_are_safe_in_both_renderers() {
        for width in 0..=3 {
            for height in 0..=3 {
                let area = Rect::new(0, 0, width, height);
                let mut buffer = Buffer::empty(area);
                sample().render(
                    area,
                    &mut Surface::new(&mut buffer, area),
                    &RenderCtx::new(&Theme::default()),
                );

                let layer = ImageLayer::new();
                sample()
                    .support(ImageSupport::Kitty)
                    .in_layer(&layer)
                    .render(
                        area,
                        &mut Surface::new(&mut buffer, area),
                        &RenderCtx::new(&Theme::default()),
                    );
            }
        }
    }

    #[test]
    fn constant_series_gets_a_nonzero_automatic_domain() {
        let chart = Chart::new().series(Series::line("flat", [Point::new(1.0, 2.0)]));
        let model = PlotModel::new(&chart).unwrap();
        assert!(model.x.min < model.x.max);
        assert!(model.y.min < model.y.max);
    }

    #[test]
    fn automatic_bar_domain_reserves_half_an_interval_at_each_edge() {
        let chart = Chart::new().series(Series::bar(
            "bars",
            [
                Point::new(0.0, 1.0),
                Point::new(1.0, 2.0),
                Point::new(2.0, 3.0),
            ],
        ));
        let model = PlotModel::new(&chart).unwrap();
        assert_eq!(
            model.x,
            Domain {
                min: -0.5,
                max: 2.5
            }
        );

        let explicit = chart.x_domain(Domain::new(0.0, 2.0).unwrap());
        assert_eq!(
            PlotModel::new(&explicit).unwrap().x,
            Domain { min: 0.0, max: 2.0 },
            "explicit bounds remain exact clipping bounds"
        );
    }
}
