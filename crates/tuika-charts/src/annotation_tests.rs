//! Bands, reference lines, dashes, grid lines, axis titles, and legend
//! visibility — the annotation layer over the series grammar.

use tuika::ui::Rect;
use tuika::ui::{Buffer, Color};

use crate::tests::{axis_column, render_to};
use crate::{Axis, Chart, Domain, Point, Reference, Series};

fn row_of(buffer: &Buffer, area: Rect, y: u16) -> String {
    (0..area.width).map(|x| buffer[(x, y)].symbol()).collect()
}

fn find_row(buffer: &Buffer, area: Rect, text: &str) -> Option<u16> {
    (0..area.height).find(|&y| row_of(buffer, area, y).contains(text))
}

fn flat(values: &[(f64, f64)]) -> Vec<Point> {
    values.iter().map(|&(x, y)| Point::new(x, y)).collect()
}

const FILL: Color = Color::Rgb(10, 200, 10);

/// The rows a column's fill covers, top to bottom.
fn filled_rows(buffer: &Buffer, area: Rect, x: u16) -> Vec<u16> {
    (0..area.height)
        .filter(|&y| buffer[(x, y)].fg == FILL)
        .collect()
}

#[test]
fn a_band_fills_between_its_edges_and_not_down_to_the_baseline() {
    let chart = Chart::new()
        .legend(false)
        .y_domain(Domain::new(0.0, 10.0).unwrap())
        .x_axis(Axis::hidden())
        .y_axis(Axis::hidden())
        .series(
            Series::band(
                "range",
                flat(&[(0.0, 4.0), (1.0, 4.0)]),
                flat(&[(0.0, 6.0), (1.0, 6.0)]),
            )
            .color(FILL),
        );
    let (buffer, area) = render_to(&chart, 20, 12);
    let rows = filled_rows(&buffer, area, 10);
    assert!(!rows.is_empty(), "the band paints something");
    let (top, bottom) = (rows[0], *rows.last().unwrap());
    // 11 data rows span 0..10, so 6 and 4 sit near rows 4 and 6.
    assert!(top >= 3 && bottom <= 7, "band spans rows {top}..={bottom}");
    assert!(
        (bottom + 2..area.height - 1).all(|y| buffer[(10, y)].fg != FILL),
        "nothing below the lower edge is filled"
    );
}

#[test]
fn a_band_ignores_stacking_and_tolerates_unequal_edges() {
    let chart = Chart::new()
        .stack(crate::Stack::Normal)
        .series(Series::area("under", flat(&[(0.0, 100.0), (1.0, 100.0)])))
        .series(Series::band(
            "range",
            flat(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)]),
            flat(&[(0.0, 5.0), (1.0, 6.0)]),
        ));
    let plan = crate::tests::plan_of(&chart);
    match &plan.marks[1].geometry {
        crate::plan::Geometry::Fill { upper, lower } => {
            assert_eq!(upper.len(), 2, "the shorter edge decides the length");
            assert_eq!(lower[0].y, 1.0, "absolute, not stacked on the area");
            assert_eq!(upper[1].y, 6.0);
        }
        _ => panic!("a band resolves to a fill"),
    }
}

#[test]
fn a_line_over_a_band_keeps_the_band_behind_it() {
    let chart = Chart::new()
        .legend(false)
        .y_domain(Domain::new(0.0, 10.0).unwrap())
        .series(
            Series::band(
                "range",
                flat(&[(0.0, 0.0), (4.0, 0.0)]),
                flat(&[(0.0, 10.0), (4.0, 10.0)]),
            )
            .color(FILL),
        )
        .series(
            Series::line("diag", flat(&[(0.0, 0.0), (4.0, 10.0)])).color(Color::Rgb(250, 0, 0)),
        );
    let (buffer, area) = render_to(&chart, 24, 12);
    let crossed = (0..area.height)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|&pos| buffer[pos].fg == Color::Rgb(250, 0, 0) && buffer[pos].symbol() != "█")
        .collect::<Vec<_>>();
    assert!(!crossed.is_empty(), "the line draws partial glyphs");
    for pos in crossed {
        assert_eq!(
            buffer[pos].bg, FILL,
            "line glyph at {pos:?} punched a hole in the band"
        );
    }
}

#[test]
fn hidden_series_leave_the_legend_and_the_readout() {
    let chart = Chart::new()
        .focus(1.0)
        .series(Series::line(
            "shown",
            flat(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)]),
        ))
        .series(Series::line("helper", flat(&[(0.0, 3.0), (1.0, 2.0), (2.0, 1.0)])).legend(false));
    let (buffer, area) = render_to(&chart, 50, 14);
    let text: String = (0..area.height)
        .map(|y| row_of(&buffer, area, y) + "\n")
        .collect();
    assert!(text.contains("shown"));
    assert!(!text.contains("helper"), "hidden series appears:\n{text}");
}

#[test]
fn every_series_hidden_drops_the_legend_row() {
    let visible = Chart::new().series(Series::line("a", flat(&[(0.0, 1.0), (1.0, 2.0)])));
    let hidden =
        Chart::new().series(Series::line("a", flat(&[(0.0, 1.0), (1.0, 2.0)])).legend(false));
    let height = |chart: &Chart| {
        let (buffer, area) = render_to(chart, 30, 12);
        let gutter = axis_column(&buffer, area).unwrap();
        (0..area.height)
            .filter(|&y| matches!(buffer[(gutter, y)].symbol(), "│" | "┤" | "└"))
            .count()
    };
    assert_eq!(
        height(&hidden),
        height(&visible) + 1,
        "the legend row goes back to the plot"
    );
}

fn referenced() -> Chart {
    Chart::new()
        .legend(false)
        .series(Series::line("v", flat(&[(0.0, 1.0), (10.0, 2.0)])))
}

#[test]
fn a_value_reference_draws_across_with_its_label_and_widens_the_domain() {
    let auto = referenced().reference(Reference::value(5.0));
    let plan = crate::tests::plan_of(&auto);
    assert!(
        plan.model.y.max >= 5.0,
        "automatic domain keeps the line visible"
    );

    let chart = referenced()
        .y_domain(Domain::new(0.0, 10.0).unwrap())
        .reference(Reference::value(5.0).label("SLO").dashed());

    let (buffer, area) = render_to(&chart, 40, 14);
    let line = (0..area.height)
        .find(|&y| row_of(&buffer, area, y).matches('╌').count() > 20)
        .expect("a dashed line across the plot");
    let label = find_row(&buffer, area, "SLO").expect("label");
    assert_eq!(label + 1, line, "label sits just above the line's end");
}

#[test]
fn an_explicit_domain_clips_a_reference_instead_of_pinning_it() {
    let chart = referenced()
        .y_domain(Domain::new(0.0, 3.0).unwrap())
        .reference(Reference::value(50.0).label("far"));
    let (buffer, area) = render_to(&chart, 40, 14);
    assert!(find_row(&buffer, area, "far").is_none());
    let axis = find_row(&buffer, area, "└").unwrap();
    assert!(
        (0..area.height)
            .filter(|&y| y != axis)
            .all(|y| row_of(&buffer, area, y).matches('─').count() < 5),
        "no rule apart from the axis"
    );
}

#[test]
fn a_position_reference_runs_up_a_vertical_chart_and_across_a_horizontal_one() {
    let vertical = referenced().reference(Reference::position(5.0));
    let (buffer, area) = render_to(&vertical, 40, 14);
    let columns: Vec<u16> = (0..area.width)
        .filter(|&x| {
            (0..area.height)
                .filter(|&y| buffer[(x, y)].symbol() == "│")
                .count()
                > 3
        })
        .collect();
    assert_eq!(columns.len(), 2, "the axis and the reference: {columns:?}");

    let horizontal = Chart::new()
        .legend(false)
        .horizontal()
        .series(Series::bar(
            "b",
            flat(&[(0.0, 3.0), (1.0, 5.0), (2.0, 4.0)]),
        ))
        .reference(Reference::value(4.5).dashed());
    let (buffer, area) = render_to(&horizontal, 40, 14);
    let dashed = (0..area.width)
        .filter(|&x| (0..area.height).any(|y| buffer[(x, y)].symbol() == "╎"))
        .count();
    assert_eq!(
        dashed, 1,
        "a value line in a horizontal chart runs up the screen"
    );
}

#[test]
fn crossing_references_join_instead_of_cutting() {
    let chart = referenced()
        .reference(Reference::position(5.0))
        .reference(Reference::value(1.5));
    let (buffer, area) = render_to(&chart, 40, 14);
    let text: String = (0..area.height).map(|y| row_of(&buffer, area, y)).collect();
    assert!(text.contains('┼'));
}

#[test]
fn a_dashed_line_leaves_gaps_a_solid_one_does_not() {
    let painted = |series: Series| {
        let chart = Chart::new()
            .legend(false)
            .x_axis(Axis::hidden())
            .y_axis(Axis::hidden())
            .series(series);
        let (buffer, area) = render_to(&chart, 30, 8);
        (0..area.height)
            .flat_map(|y| (1..area.width).map(move |x| (x, y)))
            .filter(|&pos| {
                tuika::framebuffer::QUADRANTS[1..]
                    .iter()
                    .any(|q| buffer[pos].symbol() == q.to_string())
            })
            .count()
    };
    let points = flat(&[(0.0, 1.0), (10.0, 1.0)]);
    let solid = painted(Series::line("s", points.clone()));
    let dashed = painted(Series::line("d", points).dashed());
    assert!(
        dashed > 0 && dashed < solid,
        "dashed {dashed} vs solid {solid}"
    );
}

#[test]
fn grid_lines_follow_the_labelled_ticks() {
    let chart = referenced()
        .y_axis(Axis::new().ticks(2).grid())
        .x_axis(Axis::new().grid());
    let (buffer, area) = render_to(&chart, 40, 14);
    let gutter = axis_column(&buffer, area).unwrap();
    for y in 0..area.height {
        let tick = buffer[(gutter, y)].symbol() == "┤";
        let grid = row_of(&buffer, area, y).contains('┈');
        // The bottom tick sits on the x axis itself, which needs no grid.
        if tick && !row_of(&buffer, area, y).contains('└') {
            assert!(grid, "tick row {y} has no grid line");
        }
        if grid {
            assert!(tick, "grid line on row {y} has no tick label");
        }
    }
    let text: String = (0..area.height).map(|y| row_of(&buffer, area, y)).collect();
    assert!(text.contains('┊'), "vertical grid lines");
}

#[test]
fn axis_titles_take_one_row_each_and_land_beside_their_axes() {
    let plain = referenced();
    let titled = referenced()
        .y_axis(Axis::new().title("ms"))
        .x_axis(Axis::new().title("day"));
    let (plain_buffer, area) = render_to(&plain, 40, 14);
    let (buffer, _) = render_to(&titled, 40, 14);

    let ms = find_row(&buffer, area, "ms").expect("y title");
    assert!(
        row_of(&buffer, area, ms).starts_with("ms"),
        "y title is left-aligned"
    );
    let gutter = axis_column(&buffer, area).unwrap();
    assert_eq!(
        buffer[(gutter, ms + 1)].symbol(),
        "┤",
        "plot starts right under it"
    );

    let day = find_row(&buffer, area, "day").expect("x title");
    let corner = find_row(&buffer, area, "└").unwrap();
    assert_eq!(day, corner + 2, "x title sits under the tick labels");

    let plot_rows = |b: &Buffer| {
        (0..area.height)
            .filter(|&y| matches!(b[(axis_column(b, area).unwrap(), y)].symbol(), "│" | "┤"))
            .count()
    };
    assert_eq!(plot_rows(&plain_buffer), plot_rows(&buffer) + 2);
}

#[test]
fn annotations_survive_tiny_sizes_in_both_renderers() {
    use tuika::term::image::{ImageLayer, ImageSupport};
    let chart = || {
        referenced()
            .x_axis(Axis::new().grid().title("x"))
            .y_axis(Axis::new().grid().title("y"))
            .reference(Reference::value(1.5).label("limit").dashed())
            .reference(Reference::position(3.0).label("event"))
            .series(Series::band("b", flat(&[(0.0, 0.5)]), flat(&[(0.0, 1.0)])))
            .series(Series::step("s", flat(&[(0.0, 1.0), (5.0, 2.0)])).dashed())
    };
    for width in 0..14 {
        for height in 0..10 {
            render_to(&chart(), width, height);
            let layer = ImageLayer::new();
            let graphics = chart().support(ImageSupport::Kitty).in_layer(&layer);
            render_to(&graphics, width, height);
        }
    }
}
