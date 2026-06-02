use criterion::{criterion_group, criterion_main, Criterion};

fn link_benchmark(c: &mut Criterion) {
    c.bench_function("placeholder", |b| b.iter(|| {}));
}

criterion_group!(benches, link_benchmark);
criterion_main!(benches);
