//! Wrapped flex-line solving benchmark.
//!
//! Run with `cargo bench --bench layout`.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use tuika::ui::Rect;
use tuika::{
    AlignContent, Dimension, FlexItemStyle, FlexWrap, Item, LayoutStyle, Size, solve_layout,
};

fn wrapped_items() -> Vec<Item> {
    (0..48)
        .map(|index| {
            let basis = 4 + index % 13;
            Item::styled(
                FlexItemStyle::default()
                    .basis(Dimension::Fixed(basis))
                    .grow(index % 3)
                    .shrink(1)
                    .min_main(2)
                    .max_main(20),
                Size::new(basis, 1 + index % 3),
            )
        })
        .collect()
}

fn wrapped_lines(c: &mut Criterion) {
    let items = wrapped_items();
    let style = LayoutStyle::row()
        .wrap(FlexWrap::Wrap)
        .column_gap(1)
        .row_gap(1)
        .align_content(AlignContent::SpaceBetween);
    let mut group = c.benchmark_group("layout/wrapped_lines");
    for width in [40u16, 80, 120] {
        group.bench_with_input(BenchmarkId::from_parameter(width), &width, |b, &width| {
            b.iter(|| {
                black_box(solve_layout(
                    Rect::new(0, 0, black_box(width), 40),
                    black_box(&style),
                    black_box(&items),
                ));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, wrapped_lines);
criterion_main!(benches);
