use criterion::{black_box, criterion_group, criterion_main, Criterion};
use task_queue::trees::splay_tree::SplayTree;
use task_queue::MedianFinder;

fn benchmark_add_num(c: &mut Criterion) {
	c.bench_function("MedianFinder add_num", |b| {
		b.iter(|| {
			let mut mf = MedianFinder::new();
			for i in 1..1000 {
				mf.add_num(black_box(i));
			}
		});
	});
}

fn benchmark_find_median(c: &mut Criterion) {
	let mut mf = MedianFinder::new();
	for i in 1..1000 {
		mf.add_num(i);
	}

	c.bench_function("MedianFinder find_median", |b| {
		b.iter(|| {
			mf.find_median();
		});
	});
}

criterion_group!(benches, benchmark_add_num, benchmark_find_median);
criterion_main!(benches);
