//! Bulk paste scales with inserted bytes; scalar insertion is a comparison for
//! detecting accidental per-character rebuilding. Run `cargo bench --bench input`.
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use tuika::components::TextInputState;

fn paste(c: &mut Criterion) {
    let mut group = c.benchmark_group("input/paste");
    for count in [100, 10_000, 100_000] {
        let text = "x".repeat(count);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::new("bulk", count), &text, |b, text| {
            b.iter_batched(
                || TextInputState::from_text("prefix suffix"),
                |mut input| {
                    input.set_cursor(0, 7);
                    input.insert_str(black_box(text));
                    black_box(input);
                },
                BatchSize::SmallInput,
            );
        });
        if count <= 10_000 {
            group.bench_with_input(BenchmarkId::new("scalar", count), &text, |b, text| {
                b.iter_batched(
                    || TextInputState::from_text("prefix suffix"),
                    |mut input| {
                        input.set_cursor(0, 7);
                        for ch in black_box(text).chars() {
                            input.insert_char(ch);
                        }
                        black_box(input);
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    }
    group.finish();
}
criterion_group!(benches, paste);
criterion_main!(benches);
