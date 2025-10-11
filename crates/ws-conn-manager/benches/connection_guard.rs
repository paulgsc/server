use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use futures::StreamExt;
use std::sync::Arc;
use tokio::runtime::Runtime;
use ws_conn_manager::ConnectionGuard;

// Benchmark single-threaded acquire/release
fn bench_single_client_sequential(c: &mut Criterion) {
	let mut group = c.benchmark_group("single_client_sequential");

	for connections in [1, 5, 10, 50].iter() {
		group.throughput(Throughput::Elements(*connections as u64));
		group.bench_with_input(BenchmarkId::from_parameter(connections), connections, |b, &num_conns| {
			let rt = Runtime::new().unwrap();
			let guard = ConnectionGuard::new();

			b.to_async(&rt).iter(|| async {
				let mut permits = Vec::new();
				for i in 0..num_conns {
					let permit = guard.acquire(format!("client-{}", i)).await.expect("should acquire");
					permits.push(permit);
				}
				black_box(permits);
			});
		});
	}
	group.finish();
}

// Benchmark concurrent acquires from same client (tests per-client queuing)
fn bench_same_client_concurrent(c: &mut Criterion) {
	let mut group = c.benchmark_group("same_client_concurrent");

	for concurrent in [5, 10, 20, 50].iter() {
		group.throughput(Throughput::Elements(*concurrent as u64));
		group.bench_with_input(BenchmarkId::from_parameter(concurrent), concurrent, |b, &num_concurrent| {
			let rt = Runtime::new().unwrap();
			let guard = Arc::new(ConnectionGuard::new());

			b.to_async(&rt).iter(|| {
				let guard = guard.clone();
				async move {
					let mut handles = Vec::new();
					for _ in 0..num_concurrent {
						let g = guard.clone();
						handles.push(tokio::spawn(async move { g.acquire("same-client".to_string()).await.ok() }));
					}

					let results: Vec<_> = futures::future::join_all(handles).await.into_iter().filter_map(|r| r.ok().flatten()).collect();
					black_box(results);
				}
			});
		});
	}
	group.finish();
}

// Benchmark concurrent acquires from different clients (tests global contention)
fn bench_different_clients_concurrent(c: &mut Criterion) {
	let mut group = c.benchmark_group("different_clients_concurrent");

	for clients in [10, 50, 100, 500].iter() {
		group.throughput(Throughput::Elements(*clients as u64));
		group.bench_with_input(BenchmarkId::from_parameter(clients), clients, |b, &num_clients| {
			let rt = Runtime::new().unwrap();
			let guard = Arc::new(ConnectionGuard::new());

			b.to_async(&rt).iter(|| {
				let guard = guard.clone();
				async move {
					let mut handles = Vec::new();
					for i in 0..num_clients {
						let g = guard.clone();
						handles.push(tokio::spawn(async move { g.acquire(format!("client-{}", i)).await.ok() }));
					}

					let results: Vec<_> = futures::future::join_all(handles).await.into_iter().filter_map(|r| r.ok().flatten()).collect();
					black_box(results);
				}
			});
		});
	}
	group.finish();
}

// Benchmark acquire + hold + release cycle
fn bench_acquire_hold_release(c: &mut Criterion) {
	let mut group = c.benchmark_group("acquire_hold_release");
	group.measurement_time(std::time::Duration::from_secs(10));

	for hold_ms in [0, 1, 10, 50].iter() {
		group.bench_with_input(BenchmarkId::new("hold_ms", hold_ms), hold_ms, |b, &hold_duration| {
			let rt = Runtime::new().unwrap();
			let guard = Arc::new(ConnectionGuard::new());

			b.to_async(&rt).iter(|| {
				let guard = guard.clone();
				async move {
					let permit = guard.acquire("test-client".to_string()).await.expect("should acquire");

					if hold_duration > 0 {
						tokio::time::sleep(tokio::time::Duration::from_millis(hold_duration)).await;
					}

					drop(permit);
				}
			});
		});
	}
	group.finish();
}

// Benchmark churn: continuous acquire/release from multiple clients
fn bench_high_churn(c: &mut Criterion) {
	let mut group = c.benchmark_group("high_churn");
	group.measurement_time(std::time::Duration::from_secs(15));

	for params in [(10, 100), (50, 200), (100, 500)].iter() {
		let (clients, ops) = params;
		group.throughput(Throughput::Elements(*ops as u64));
		group.bench_with_input(BenchmarkId::new("clients_ops", format!("{}_{}", clients, ops)), params, |b, &(num_clients, num_ops)| {
			let rt = Runtime::new().unwrap();
			let guard = Arc::new(ConnectionGuard::new());

			b.to_async(&rt).iter(|| {
				let guard = guard.clone();
				async move {
					let mut handles = Vec::new();

					for client_id in 0..num_clients {
						let g = guard.clone();
						let ops_per_client = num_ops / num_clients;

						handles.push(tokio::spawn(async move {
							for _ in 0..ops_per_client {
								if let Ok(permit) = g.acquire(format!("client-{}", client_id)).await {
									// Simulate tiny bit of work
									tokio::task::yield_now().await;
									drop(permit);
								}
							}
						}));
					}

					futures::future::join_all(handles).await;
				}
			});
		});
	}
	group.finish();
}

// Benchmark worst case: hitting per-client limits with queue full
fn bench_queue_full_rejection(c: &mut Criterion) {
	let mut group = c.benchmark_group("queue_full_rejection");

	for concurrent in [10, 20, 50].iter() {
		group.bench_with_input(BenchmarkId::from_parameter(concurrent), concurrent, |b, &num_concurrent| {
			let rt = Runtime::new().unwrap();
			let guard = Arc::new(ConnectionGuard::new());

			b.to_async(&rt).iter(|| {
				let guard = guard.clone();
				async move {
					let mut handles = Vec::new();

					// All trying to connect as same client to trigger queue limits
					for _ in 0..num_concurrent {
						let g = guard.clone();
						handles.push(tokio::spawn(async move { g.acquire("overloaded-client".to_string()).await }));
					}

					let results: Vec<_> = futures::future::join_all(handles).await.into_iter().filter_map(|r| r.ok()).collect();
					black_box(results);
				}
			});
		});
	}
	group.finish();
}

// Benchmark metadata operations (should be fast)
fn bench_metadata_queries(c: &mut Criterion) {
	let mut group = c.benchmark_group("metadata_queries");

	let rt = Runtime::new().unwrap();
	let guard = ConnectionGuard::new();

	// Pre-populate with some connections
	rt.block_on(async {
		let mut stream = (0..50).map(|i| guard.acquire(format!("client-{}", i))).collect::<futures::stream::FuturesUnordered<_>>();

		let mut _permits = Vec::new();
		while let Some(result) = stream.next().await {
			if let Ok(permit) = result {
				_permits.push(permit);
			}
		}
	});

	group.bench_function("active_global", |b| {
		b.iter(|| {
			black_box(guard.active_global());
		});
	});

	group.bench_function("active_per_client", |b| {
		b.iter(|| {
			black_box(guard.active_per_client("client-25"));
		});
	});

	group.bench_function("try_acquire_permit_hint", |b| {
		b.iter(|| {
			black_box(guard.try_acquire_permit_hint());
		});
	});

	group.finish();
}

criterion_group!(
	benches,
	bench_single_client_sequential,
	bench_same_client_concurrent,
	bench_different_clients_concurrent,
	bench_acquire_hold_release,
	bench_high_churn,
	bench_queue_full_rejection,
	bench_metadata_queries,
);

criterion_main!(benches);
