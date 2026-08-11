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
use tuika::term::image::{ImageLayer, ImageSupport};
use tuika::{RenderCtx, Size, Surface, View};

mod cells;
mod graphics;
mod model;

use cells::{render_cells, render_legend};
use graphics::render_pixels;
use model::PlotModel;
pub use model::{Domain, Point, Series, SeriesKind};

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
    use crate::cells::{BrailleGrid, QuadrantGrid, draw_quadrant_area};
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
