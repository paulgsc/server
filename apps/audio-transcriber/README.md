
# Hotpath Module — Invariant Contract

This module is a real-time execution boundary. Every file in this directory
must uphold all invariants listed below. Violating any of them is a bug,
regardless of whether tests catch it immediately.

## Hard Invariants (never negotiate these)

1. **Zero dynamic allocations in steady state.**
   No `Vec::new()`, `String::new()`, `Box::new()`, or any call that reaches the
   global allocator during normal operation. Startup (`_init` functions) may allocate.
   The steady-state path may not.

2. **No blocking system calls.**
   No `std::sync::Mutex::lock()`, no `std::sync::RwLock`, no file I/O, no network I/O,
   no `thread::sleep`, no `thread::park`. If you need cross-thread communication,
   use the SPSC in `spsc.rs`.

3. **No async.**
   No `async fn`, no `.await`. The hotpath runs on a dedicated OS thread. It must
   never be scheduled by Tokio or any async executor.

4. **No `tracing` macros in the execution loop.**
   `info!`, `warn!`, `debug!` etc. internally acquire a `Mutex`. Use the log ring
   buffer appender (M6) for any observability from the hotpath.

5. **Bounded execution time.**
   All loops must iterate over a fixed, compile-time count. No open-ended `while`
   loops that depend on runtime conditions. No recursive functions.

## Allowed

- `std::sync::atomic::*`
- `const` generics for all sizing
- `#[inline(always)]` on critical path functions
- Raw pointer arithmetic in `spsc.rs` (documented, `unsafe` blocks with invariant comments)
- FFI into CUDA / whisper-rs (these are blocking by nature and belong here)

## Verification

Run this to confirm the module is clean:

```bash
cargo test -p audio-transcriber milestone_ -- --nocapture
cargo clippy -p audio-transcriber -- -W clippy::pedantic -D warnings
```

If any milestone test fails, fix it before committing.
