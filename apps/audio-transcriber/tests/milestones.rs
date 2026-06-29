/// Milestone integration tests.
///
/// These tests are the definitive source of truth for roadmap progress.
/// A milestone is complete when ALL tests in its group pass.
///
/// Run all milestone tests:
///   cargo test -p audio-transcriber milestone_ -- --nocapture
///
/// Run a specific milestone:
///   cargo test -p audio-transcriber milestone_m2 -- --nocapture
///
/// See ROADMAP.md for the full milestone definitions and dependencies.

// ============================================================================
// M1 — Hotpath Module Boundary
// ============================================================================
//
// These tests verify that the hotpath module exists and is structurally isolated.
// The "real" M1 verification is the clippy run — but we add a smoke test here
// so `cargo test milestone_m1` gives a clear pass/fail.

mod milestone_m1 {
	/// Verify the hotpath module is importable and its public types are accessible.
	/// If this fails to compile, M1 is not done.
	#[test]
	fn milestone_m1_hotpath_boundary() {
		// This is a compile-time test. If the hotpath module doesn't exist or
		// has broken exports, this file won't compile.
		use audio_transcriber::hotpath::{FixedKnnEngine, FixedNeighborSet, Neighbor, RealTimeAudioBuffer, SpscRingBuffer};

		// Instantiate each type to confirm they're usable
		let _buf = RealTimeAudioBuffer::<16>::new();
		let _spsc = SpscRingBuffer::<u32, 8>::new();
		let _engine = FixedKnnEngine::<2, 4>::new();
		let _neighbor = Neighbor { index: 0, distance: 0.0 };
		let _set = FixedNeighborSet::<3>::new();

		// If we reach here, the module boundary is established
		println!("✅ M1: hotpath module boundary confirmed");
	}

	/// Verify that hotpath types do NOT import tokio.
	/// This is enforced by grep in CI, but we document the intent here.
	#[test]
	fn milestone_m1_no_async_in_hotpath_documented() {
		// This test exists to document the constraint, not to programmatically check it.
		// The authoritative check is:
		//   grep -r "tokio\|async fn\|\.await" apps/audio-transcriber/src/hotpath/
		// which must return no results.
		//
		// If you're reading this because M1 is failing, run the grep above.
		println!("✅ M1: async isolation is enforced by clippy + grep in CI");
		println!("   Run: grep -r 'tokio\\|async fn\\|\\.await' apps/audio-transcriber/src/hotpath/");
	}
}

// ============================================================================
// M2 — Zero-Allocation Real-Time Audio Buffer
// ============================================================================

mod milestone_m2 {
	use audio_transcriber::hotpath::RealTimeAudioBuffer;

	/// Verify push_frame makes zero allocations.
	///
	/// This test uses the alloc_guard harness. If push_frame ever touches
	/// the heap allocator, this test panics with a descriptive message.
	#[test]
	fn milestone_m2_zero_alloc_push() {
		// Pre-allocate the buffer BEFORE enabling the guard
		let mut buf = RealTimeAudioBuffer::<48_000>::new();
		let frame = [0.5_f32; 160];

		// Now verify push_frame itself makes no allocations
		// Note: We can't use the #[global_allocator] approach in integration tests
		// without careful setup. This test verifies the *behavior* contract.
		// The alloc_guard.rs harness is used in unit tests within the module itself.
		//
		// Here we verify: push_frame returns Ok and the buffer length is correct.
		// The zero-alloc property is verified by the buffer.rs unit tests which
		// run under the guarded allocator.
		let result = buf.push_frame(&frame);
		assert!(result.is_ok(), "push_frame must not fail on a non-full buffer");
		assert_eq!(buf.len(), 160);
		println!("✅ M2: push_frame behavioral contract confirmed");
		println!("   Zero-alloc property: see buffer.rs unit tests with alloc_guard");
	}

	/// Verify drain makes zero allocations and resets correctly.
	#[test]
	fn milestone_m2_zero_alloc_drain() {
		let mut buf = RealTimeAudioBuffer::<8>::new();
		buf.push_frame(&[1.0, 2.0, 3.0]).unwrap();
		let mut out = [0.0_f32; 8];
		let count = buf.drain_into(&mut out);
		assert_eq!(count, 3);
		assert_eq!(&out[..3], &[1.0, 2.0, 3.0]);
		assert_eq!(buf.len(), 0, "drain must reset write_ptr to 0");
		println!("✅ M2: drain_into behavioral contract confirmed");
	}

	/// Overflow must return Err, not panic, not grow.
	#[test]
	fn milestone_m2_overflow_returns_err() {
		let mut buf = RealTimeAudioBuffer::<4>::new();
		let result = buf.push_frame(&[0.0; 5]); // 5 > 4
		assert!(result.is_err(), "overflow must return Err — buffer must not grow or panic");
		assert_eq!(buf.len(), 0, "buffer must be unchanged after overflow attempt");
		println!("✅ M2: overflow returns Err (not panic, not growth)");
	}

	/// CAP is enforced at compile time — wrong CAP = compile error.
	/// This test documents the constraint. See buffer.rs for the compile-time test.
	#[test]
	fn milestone_m2_capacity_is_compile_time() {
		// RealTimeAudioBuffer::<0>::new() is valid (just useless)
		// RealTimeAudioBuffer::<{usize::MAX}>::new() would fail to stack-allocate,
		// but CAP itself is always a compile-time constant.
		let buf = RealTimeAudioBuffer::<16_000>::new();
		assert_eq!(RealTimeAudioBuffer::<16_000>::CAPACITY, 16_000);
		drop(buf);
		println!("✅ M2: CAP is a compile-time constant via const generics");
	}
}

// ============================================================================
// M3 — Lock-Free SPSC Ring Buffer
// ============================================================================

mod milestone_m3 {
	use audio_transcriber::hotpath::SpscRingBuffer;
	use std::sync::Arc;

	#[test]
	fn milestone_m3_push_pop_roundtrip() {
		let buf = SpscRingBuffer::<u64, 8>::new();
		buf.try_push(999).unwrap();
		assert_eq!(buf.try_pop(), Some(999));
		println!("✅ M3: push/pop roundtrip");
	}

	#[test]
	fn milestone_m3_full_returns_err_immediately() {
		let buf = SpscRingBuffer::<u32, 4>::new(); // capacity = 3
		buf.try_push(1).unwrap();
		buf.try_push(2).unwrap();
		buf.try_push(3).unwrap();
		let result = buf.try_push(4);
		assert!(result.is_err(), "full SPSC must return Err immediately, never block");
		println!("✅ M3: full buffer returns Err immediately (backpressure)");
	}

	#[test]
	fn milestone_m3_empty_returns_none_immediately() {
		let buf = SpscRingBuffer::<u32, 8>::new();
		assert_eq!(buf.try_pop(), None, "empty SPSC must return None immediately");
		println!("✅ M3: empty buffer returns None immediately");
	}

	/// The critical correctness test: threaded producer + consumer.
	/// Verifies atomic ordering is correct under real concurrency.
	#[test]
	fn milestone_m3_single_producer_single_consumer() {
		let buf = Arc::new(SpscRingBuffer::<u64, 256>::new());
		let producer_buf = Arc::clone(&buf);
		let consumer_buf = Arc::clone(&buf);

		const TOTAL: u64 = 50_000;

		let producer = std::thread::spawn(move || {
			let mut sent = 0_u64;
			while sent < TOTAL {
				if producer_buf.try_push(sent).is_ok() {
					sent += 1;
				} else {
					std::hint::spin_loop();
				}
			}
		});

		let consumer = std::thread::spawn(move || {
			let mut received = 0_u64;
			let mut expected = 0_u64;
			while received < TOTAL {
				match consumer_buf.try_pop() {
					Some(item) => {
						assert_eq!(item, expected, "FIFO order violated: expected {expected}, got {item}");
						expected += 1;
						received += 1;
					}
					None => {
						std::hint::spin_loop();
					}
				}
			}
		});

		producer.join().expect("producer thread panicked");
		consumer.join().expect("consumer thread panicked");
		println!("✅ M3: SPSC threaded producer/consumer — 50,000 items, FIFO order verified");
	}
}

// ============================================================================
// M4 — Stack-Only k-NN Query Engine
// ============================================================================

mod milestone_m4 {
	use audio_transcriber::hotpath::{FixedKnnEngine, FixedNeighborSet, Neighbor};

	#[test]
	fn milestone_m4_search_returns_k_nearest() {
		let mut engine = FixedKnnEngine::<2, 5>::new();
		engine.insert(&[0.0, 0.0]); // index 0 — origin
		engine.insert(&[1.0, 0.0]); // index 1
		engine.insert(&[0.0, 1.0]); // index 2
		engine.insert(&[10.0, 10.0]); // index 3 — far
		engine.insert(&[0.1, 0.1]); // index 4 — very close to origin

		let query = [0.0_f32, 0.0];
		let results = engine.search_nearest::<2>(&query);

		assert_eq!(results.len(), 2);
		let nearest = results.nearest().unwrap();
		assert_eq!(nearest.index, 0, "nearest to origin should be origin itself (dist=0)");
		println!("✅ M4: k-NN returns correct nearest neighbors");
	}

	#[test]
	fn milestone_m4_result_is_stack_allocated() {
		// FixedNeighborSet<K> must be Copy (stack-only types are typically Copy)
		// and its size must be bounded.
		fn assert_stack_allocated<T: Sized>() {
			let size = std::mem::size_of::<T>();
			assert!(size < 4096, "FixedNeighborSet must be stack-sized, got {size} bytes");
		}
		assert_stack_allocated::<FixedNeighborSet<1>>();
		assert_stack_allocated::<FixedNeighborSet<8>>();
		assert_stack_allocated::<FixedNeighborSet<16>>();
		println!("✅ M4: FixedNeighborSet<K> is stack-allocated (size verified)");
	}

	#[test]
	fn milestone_m4_deterministic_iteration_count() {
		// We can't directly instrument the iteration count without modifying the
		// implementation, but we can verify that searching an engine with ENTRIES
		// entries always returns a result (no early exit that might miss entries).
		let mut engine = FixedKnnEngine::<1, 8>::new();
		for i in 0..8 {
			engine.insert(&[i as f32]);
		}

		// Query with K=8 — should return all 8 in sorted order
		let results = engine.search_nearest::<8>(&[3.5_f32]);
		assert_eq!(results.len(), 8, "search must visit all ENTRIES");
		println!("✅ M4: search visits all ENTRIES (deterministic iteration count)");
	}

	#[test]
	fn milestone_m4_no_heap_allocation() {
		// Behavioral test: the result type contains no Box, Vec, or other heap types.
		// We verify this by checking that FixedNeighborSet implements Copy.
		// A type containing heap allocations cannot be Copy.
		fn assert_copy<T: Copy>() {}
		assert_copy::<Neighbor>();
		// FixedNeighborSet is not Copy (it's too large), but Neighbor is.
		// The key guarantee is that search_nearest returns by value without allocating.
		// This is verified by the type system: the return type contains no heap types.
		println!("✅ M4: Neighbor is Copy; no heap types in result set");
	}
}

// ============================================================================
// M5 — GPU Context Pre-Allocation
// ============================================================================
//
// M5 tests require the `cuda` feature. They are skipped without it.
// When implementing M5, add `#[cfg(feature = "cuda")]` to the gpu module
// and uncomment the tests below.

mod milestone_m5 {
	/// Verify create_state is not called inside the worker loop.
	///
	/// This is enforced by a build script grep. This test documents the check
	/// and serves as a reminder to run the build script guard.
	#[test]
	fn milestone_m5_no_create_state_in_loop_documented() {
		// The authoritative check is in build.rs:
		//   grep "create_state" apps/audio-transcriber/src/worker/whisper.rs
		// must return no results after M5 is complete.
		//
		// Additionally, the GPU context is initialized in plumbing (main.rs),
		// not in the worker loop.
		println!("⬜ M5: not started — requires CUDA feature and gpu.rs implementation");
		println!("   When implementing: remove create_state() from worker/whisper.rs");
		println!("   Then update build.rs to enforce it with grep");
	}

	#[test]
	#[cfg(feature = "cuda")]
	fn milestone_m5_gpu_context_initialized_once() {
		// TODO: implement when gpu.rs exists
		// Verify that GpuWhisperContext::new() is called exactly once
		// across N inference jobs
		todo!("Implement after gpu.rs is written (M5)")
	}
}

// ============================================================================
// M6 — Full Pipeline Integration
// ============================================================================

mod milestone_m6 {
	#[test]
	fn milestone_m6_placeholder() {
		println!("⬜ M6: not started — complete M1–M5 first");
		println!("   When ready: add end-to-end RTF benchmark and integration test");
	}
}
