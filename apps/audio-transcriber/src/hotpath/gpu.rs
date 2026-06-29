
/// GPU context wrapper — pre-allocated at startup, reused across all inference jobs.
///
/// This module is gated on the `cuda` feature flag.
/// Enable it with: `cargo build --features cuda`
///
/// # Contract
///
/// - `GpuWhisperContext::new()` is called ONCE at service startup (in main/plumbing).
/// - `run_inference()` is called in the hotpath worker loop.
/// - `run_inference()` makes ZERO CUDA allocations (all buffers are pre-allocated in new()).
/// - `Drop` frees VRAM — only runs at process shutdown.
///
/// See `docs/adr/005-gpu-prealloc.md` for full rationale and VRAM budget.

/// Number of CUDA streams to pre-create.
/// 2 allows overlapping H2D transfer for job N+1 with compute for job N.
pub const CUDA_STREAM_COUNT: usize = 2;

/// Audio samples per inference job.
/// Must match RealTimeAudioBuffer::CAPACITY.
/// 16000 Hz × 3s = 48000 samples.
pub const INFERENCE_SAMPLES: usize = 48_000;

/// Result of a single Whisper inference pass.
/// Stack-allocated — no Vec, no String.
/// Text is written into a fixed-size byte buffer.
pub struct InferenceResult {
    /// UTF-8 encoded transcript bytes. Not null-terminated.
    pub text: [u8; 1024],
    /// Number of valid bytes in `text`.
    pub text_len: usize,
    /// Confidence score (log probability), if available.
    pub confidence: Option<f32>,
}

impl InferenceResult {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.text[..self.text_len])
            .unwrap_or("[invalid utf8]")
    }
}

/// Pre-allocated GPU Whisper context.
///
/// # TODO (M5 implementation checklist)
///
/// - [ ] Load model weights from disk → GPU VRAM in `new()`
/// - [ ] Allocate KV cache, mel spectrogram buffer, output token buffer in VRAM
/// - [ ] Create `CUDA_STREAM_COUNT` CUDA streams
/// - [ ] Allocate one pinned host buffer (`cudaMallocHost`) for H2D transfers
/// - [ ] Remove `whisper_ctx.create_state()` from `worker/whisper.rs`
/// - [ ] Uncomment `check_no_create_state_in_worker()` in `build.rs`
/// - [ ] Enable `cuda` feature in `Cargo.toml`
/// - [ ] Run milestone_m5 tests
pub struct GpuWhisperContext {
    // TODO: add CUDA context, stream handles, VRAM buffer pointers
    // Example fields (fill in with actual CUDA types):
    //
    // whisper_ctx: *mut WhisperCudaContext,  // opaque FFI pointer
    // cuda_streams: [CudaStream; CUDA_STREAM_COUNT],
    // vram_kv_cache: CudaBuffer,
    // vram_spectrogram: CudaBuffer,
    // pinned_audio_host: *mut f32,  // page-locked host memory
    //
    // For now, a unit struct placeholder:
    _private: (),
}

impl GpuWhisperContext {
    /// Initialize the GPU context. Call this ONCE at startup.
    ///
    /// # Arguments
    ///
    /// * `model_path` — Path to the Whisper GGML model file
    /// * `threads` — Number of CPU threads for preprocessing (mel spectrogram)
    ///
    /// # Errors
    ///
    /// Returns `Err` if the GPU is unavailable, VRAM is insufficient, or the
    /// model file cannot be loaded.
    pub fn new(_model_path: &str, _threads: i32) -> Result<Self, &'static str> {
        // TODO: implement CUDA initialization
        // 1. Load model via whisper_rs with CUDA enabled
        // 2. create_state() here, once
        // 3. Allocate CUDA streams
        // 4. Allocate pinned host buffer
        Err("GpuWhisperContext: not yet implemented (M5 TODO)")
    }

    /// Run inference on a pre-filled audio buffer.
    ///
    /// # Arguments
    ///
    /// * `audio` — Fixed-size audio buffer (INFERENCE_SAMPLES f32 samples at 16kHz)
    ///
    /// # Returns
    ///
    /// Stack-allocated `InferenceResult`. No heap allocations.
    ///
    /// # Real-time guarantees
    ///
    /// This function is NOT async and does NOT allocate. It blocks the calling
    /// thread until inference completes. The caller (hotpath worker) must be
    /// a dedicated blocking thread.
    pub fn run_inference(&self, _audio: &[f32; INFERENCE_SAMPLES]) -> InferenceResult {
        // TODO: implement
        // 1. Copy audio to pinned host buffer
        // 2. cudaMemcpyAsync(device_buf, pinned_buf, ..., stream[0])
        // 3. Launch whisper kernel on stream[1]
        // 4. cudaStreamSynchronize(stream[1])
        // 5. Copy result tokens to InferenceResult.text
        InferenceResult {
            text: [0u8; 1024],
            text_len: 0,
            confidence: None,
        }
    }
}

impl Drop for GpuWhisperContext {
    fn drop(&mut self) {
        // TODO: free CUDA resources
        // cudaFree(vram buffers)
        // cudaFreeHost(pinned_audio_host)
        // Destroy streams
    }
}

// Safety: GpuWhisperContext contains raw CUDA pointers which are Send
// once initialized (they are not thread-local). The hotpath worker owns
// the context exclusively — no concurrent access.
// TODO: uncomment when actual CUDA fields are added
// unsafe impl Send for GpuWhisperContext {}
