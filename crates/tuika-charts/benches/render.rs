//! Cell and graphics chart rendering benchmarks.
//!
//! Run with `cargo bench -p tuika-charts --bench render`.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use tuika::term::image::{ImageLayer, ImageSupport};
use tuika::ui::Buffer;
use tuika::ui::Rect;
use tuika::{RenderCtx, Surface, Theme, View};
use tuika_charts::{Chart, Point, Series};

fn chart(support: ImageSupport, layer: Option<&ImageLayer>) -> Chart {
    let points = (0..256).map(|x| {
        let x = f64::from(x);
        Point::new(x, (x / 12.0).sin() * 40.0 + x / 8.0)
    });
    let chart = Chart::new()
        .title("Requests")
        .series(Series::area("baseline", points.clone()))
        .series(Series::line("requests", points))
        .support(support);
    match layer {
        Some(layer) => chart.in_layer(layer),
        None => chart,
    }
}

fn render_chart(chart: &Chart, cols: u16, rows: u16, theme: &Theme) {
    let area = Rect::new(0, 0, cols, rows);
    let mut buffer = Buffer::empty(area);
    let mut surface = Surface::new(&mut buffer, area);
    chart.render(area, &mut surface, &RenderCtx::new(theme));
}

fn render(c: &mut Criterion) {
    let theme = Theme::default();
    let cells = chart(ImageSupport::None, None);
    let kitty_layer = ImageLayer::new();
    let kitty = chart(ImageSupport::Kitty, Some(&kitty_layer));
    let iterm2_layer = ImageLayer::new();
    let iterm2 = chart(ImageSupport::ITerm2, Some(&iterm2_layer));
    let mut group = c.benchmark_group("charts/render");

    for (cols, rows) in [(48, 14), (96, 28), (160, 48)] {
        let size = format!("{cols}x{rows}");
        group.bench_with_input(
            BenchmarkId::new("cells", &size),
            &(cols, rows),
            |b, &size| {
                b.iter(|| render_chart(black_box(&cells), size.0, size.1, black_box(&theme)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("graphics_raster", &size),
            &(cols, rows),
            |b, &size| {
                b.iter(|| {
                    render_chart(black_box(&kitty), size.0, size.1, black_box(&theme));
                    kitty_layer.clear();
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("kitty_frame", &size),
            &(cols, rows),
            |b, &size| {
                let mut encoded = Vec::new();
                b.iter(|| {
                    render_chart(black_box(&kitty), size.0, size.1, black_box(&theme));
                    encoded.clear();
                    kitty_layer.emit(&mut encoded).unwrap();
                    black_box(&encoded);
                    kitty_layer.clear();
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("iterm2_frame", &size),
            &(cols, rows),
            |b, &size| {
                let mut encoded = Vec::new();
                b.iter(|| {
                    render_chart(black_box(&iterm2), size.0, size.1, black_box(&theme));
                    encoded.clear();
                    iterm2_layer.emit(&mut encoded).unwrap();
                    black_box(&encoded);
                    iterm2_layer.clear();
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, render);
criterion_main!(benches);
