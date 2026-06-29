
# GPU Pipeline Transition Roadmap

> **Goal:** Move `audio-transcriber` from a CPU-bound async pipeline to a hard real-time
> GPU-accelerated pipeline targeting the NVIDIA RTX 4060.
>
> **Design constraint:** Progress is only counted when `cargo test` confirms it —
> not when the code exists. Every milestone below maps to a concrete test, lint, or
> compile-time guard that cannot be faked.

---

## How to check your current status at any time

```bash
# From repo root
cargo test -p audio-transcriber --all-features 2>&1 | grep -E "PASS|FAIL|error"

# Quick milestone summary
cargo test -p audio-transcriber milestone_ -- --nocapture
```

---

## Milestone Map

```
[M1] Hotpath module boundary established
      └─► [M2] Zero-allocation audio buffer
            └─► [M3] SPSC ring buffer (lock-free)
                  └─► [M4] k-NN stack-only query engine
                        └─► [M5] GPU context pre-allocation
                              └─► [M6] Full pipeline integration
```

Dependencies are strict: each milestone's tests must be green before starting the next.

---

## M1 — Hotpath Module Boundary

**What it means:** The `hotpath/` module exists, compiles, and is structurally isolated
from async plumbing. Clippy pedantic + nursery run clean on it. No `async`, no `tokio`,
no `std::sync::Mutex` imported anywhere inside `hotpath/`.

**Verification:**
```bash
cargo test -p audio-transcriber milestone_m1 -- --nocapture
cargo clippy -p audio-transcriber -- -W clippy::pedantic -W clippy::nursery -D warnings
```

**Test that must pass:** `tests/milestones.rs::milestone_m1_hotpath_boundary`

**Files to create:**
- `src/hotpath/mod.rs`
- `src/hotpath/README.md` (invariant contract, see template below)

**Done when:** CI runs clean, no `tokio` in `hotpath/` dependency tree, structural lint passes.

**ADR:** [ADR-001](docs/adr/001-hotpath-isolation.md)

---

## M2 — Zero-Allocation Real-Time Audio Buffer

**What it means:** `RealTimeAudioBuffer<CAP>` is fully implemented. The custom allocator
harness confirms zero heap allocations occur during `push_frame` and `drain`. Buffer
overflow returns `Err` immediately — no panic, no growth.

**Verification:**
```bash
cargo test -p audio-transcriber milestone_m2 -- --nocapture
```

**Tests that must pass:**
- `milestone_m2_zero_alloc_push` — push_frame triggers zero allocations
- `milestone_m2_zero_alloc_drain` — drain triggers zero allocations
- `milestone_m2_overflow_returns_err` — overflow is `Err`, not panic
- `milestone_m2_capacity_compile_time` — CAP enforced at compile time (this is a build test)

**Files to create/modify:**
- `src/hotpath/buffer.rs` — the implementation
- `tests/alloc_guard.rs` — the custom allocator harness

**Replaces:** `Vec<f32>` audio buffer in `audio.rs`

**ADR:** [ADR-002](docs/adr/002-zero-alloc-buffer.md)

---

## M3 — Lock-Free SPSC Ring Buffer

**What it means:** `SpscRingBuffer<T, N>` is implemented with atomic indices only.
`try_push` / `try_pop` are non-blocking and safe to call from a real-time thread.
No `std::sync::Mutex`, no `tokio::sync`, no `parking_lot`. The SPSC replaces
`tokio::sync::mpsc` at the hotpath boundary in `process_audio_chunk`.

**Verification:**
```bash
cargo test -p audio-transcriber milestone_m3 -- --nocapture
```

**Tests that must pass:**
- `milestone_m3_push_pop_roundtrip` — basic correctness
- `milestone_m3_full_returns_err_immediately` — backpressure, not block
- `milestone_m3_no_alloc_push` — zero allocations on push
- `milestone_m3_no_alloc_pop` — zero allocations on pop
- `milestone_m3_single_producer_single_consumer` — threaded correctness test

**Files to create:**
- `src/hotpath/spsc.rs`

**Replaces:** `mpsc::Sender<TranscriptionJob>` at the `process_audio_chunk` boundary

**ADR:** [ADR-003](docs/adr/003-spsc-ring-buffer.md)

---

## M4 — Stack-Only k-NN Query Engine

**What it means:** `FixedKnnEngine<DIM, ENTRIES>` and `FixedNeighborSet<K>` are
implemented. Search results live entirely on the stack. No `Vec`, no `Box`, no heap
anywhere in the query path. Worst-case iteration count is `ENTRIES` — a compile-time
constant.

**Verification:**
```bash
cargo test -p audio-transcriber milestone_m4 -- --nocapture
```

**Tests that must pass:**
- `milestone_m4_search_returns_k_nearest` — correctness against known vectors
- `milestone_m4_no_heap_allocation` — zero allocations during search
- `milestone_m4_result_is_stack_allocated` — `size_of::<FixedNeighborSet<K>>()` is bounded
- `milestone_m4_deterministic_iteration` — iteration count == ENTRIES (instrumented)

**Files to create:**
- `src/hotpath/knn.rs`

**ADR:** [ADR-004](docs/adr/004-stack-knn.md)

---

## M5 — GPU Context Pre-Allocation (RTX 4060)

**What it means:** Whisper/TensorRT GPU context is initialized once at startup — not
inside the worker loop. `create_state()` is removed from `process_transcription_job`.
VRAM allocations happen in plumbing init, never in the hotpath. CUDA stream handles are
pre-created and reused.

**Verification:**
```bash
cargo test -p audio-transcriber milestone_m5 -- --nocapture
```

**Tests that must pass:**
- `milestone_m5_no_create_state_in_loop` — static analysis test: grep/AST confirms
  `create_state` is not called inside the worker loop (enforced via a build script)
- `milestone_m5_gpu_context_initialized_once` — init counter is 1 after N jobs
- `milestone_m5_context_reused_across_jobs` — same pointer across 10 sequential jobs

**Files to create/modify:**
- `src/hotpath/gpu.rs` — GPU context wrapper
- `build.rs` — static analysis guard for `create_state` call-site

**ADR:** [ADR-005](docs/adr/005-gpu-prealloc.md)

---

## M6 — Full Pipeline Integration

**What it means:** The entire path from NATS ingestion to GPU inference uses the new
hotpath components end-to-end. The old `tokio::sync::mpsc` at the audio boundary is
gone. Logging is routed through the atomic ring-buffer appender (no `info!` in the
hotpath loop). All M1–M5 tests still pass.

**Verification:**
```bash
cargo test -p audio-transcriber -- --nocapture
cargo bench -p audio-transcriber
```

**Tests that must pass:** All M1–M5 tests, plus:
- `milestone_m6_no_tracing_macros_in_hotpath` — build-script AST check
- `milestone_m6_end_to_end_rtf_below_threshold` — bench: RTF < 0.15 under load
- `milestone_m6_zero_alloc_end_to_end` — alloc guard wraps full audio→transcript path

**ADR:** [ADR-006](docs/adr/006-integration.md)

---

## Status Board

Update this manually when a milestone goes green. Do not mark green until `cargo test` confirms it.

| Milestone | Status | Date | Notes |
|-----------|--------|------|-------|
| M1 — Hotpath boundary | ⬜ Not started | — | — |
| M2 — Zero-alloc buffer | ⬜ Not started | — | — |
| M3 — SPSC ring buffer | ⬜ Not started | — | — |
| M4 — Stack k-NN | ⬜ Not started | — | — |
| M5 — GPU pre-alloc | ⬜ Not started | — | — |
| M6 — Integration | ⬜ Not started | — | — |

Legend: ⬜ Not started · 🔧 In progress · ✅ Green · ❌ Blocked

---

## What "done" is NOT

- Writing the code without a passing test → not done
- The test passing locally but not in CI → not done  
- Commenting out the test → not done
- Marking the status board green before `cargo test` confirms it → not done

---

## Picking up after a break

1. Run `cargo test -p audio-transcriber milestone_ -- --nocapture`
2. Find the lowest-numbered milestone that isn't all-green
3. Read its ADR in `docs/adr/`
4. Open the corresponding `src/hotpath/*.rs` file
5. Resume

That's the full re-orientation. No other context needed.
