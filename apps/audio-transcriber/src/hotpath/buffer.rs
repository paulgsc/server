
/// Zero-allocation, fixed-capacity audio sample buffer for the real-time hotpath.
///
/// # Design
///
/// `RealTimeAudioBuffer<CAP>` stores up to `CAP` f32 samples in a stack-allocated
/// array. `push_frame` and `drain` are guaranteed allocation-free. Overflow returns
/// `Err` immediately — the buffer never grows.
///
/// # Sizing
///
/// `CAP = target_sample_rate * buffer_duration_secs`
///
/// Default (16 kHz, 3s): CAP = 48_000 → 192 KB on the stack.
/// This is well within the default Linux thread stack (8 MB).
///
/// If larger buffers are needed, allocate the struct on the heap once at startup:
/// `Box::new(RealTimeAudioBuffer::default())`.
///
/// See `docs/adr/002-zero-alloc-buffer.md` for full rationale.
pub struct RealTimeAudioBuffer<const CAP: usize> {
    samples: [f32; CAP],
    write_ptr: usize,
}

impl<const CAP: usize> RealTimeAudioBuffer<CAP> {
    /// Create a new, empty buffer.
    ///
    /// This is the only place where the backing array is initialized.
    /// Call this once at startup, not in the hotpath.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: [0.0_f32; CAP],
            write_ptr: 0,
        }
    }

    /// Push a frame of samples into the buffer.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the frame fit.
    /// - `Err("overflow")` if `write_ptr + frame.len() > CAP`.
    ///   The buffer state is unchanged on overflow.
    ///
    /// # Real-time guarantees
    ///
    /// - Zero allocations.
    /// - O(frame.len()) time — one `copy_from_slice`.
    /// - No panic. Overflow is a returned `Err`.
    #[inline(always)]
    pub fn push_frame(&mut self, frame: &[f32]) -> Result<(), &'static str> {
        let end = self.write_ptr + frame.len();
        if end > CAP {
            return Err("overflow: RealTimeAudioBuffer capacity exceeded");
        }
        self.samples[self.write_ptr..end].copy_from_slice(frame);
        self.write_ptr = end;
        Ok(())
    }

    /// Returns true if the buffer has accumulated `CAP` samples.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.write_ptr >= CAP
    }

    /// Current fill level in samples.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.write_ptr
    }

    /// True if no samples have been pushed since last drain.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.write_ptr == 0
    }

    /// Drain the buffer into a fixed-size output array.
    ///
    /// Copies `samples[0..write_ptr]` into `out[0..write_ptr]`,
    /// then resets `write_ptr = 0`.
    ///
    /// # Returns
    ///
    /// The number of samples written to `out`.
    ///
    /// # Real-time guarantees
    ///
    /// - Zero allocations.
    /// - One `copy_from_slice` (bounded by CAP).
    /// - No panic.
    #[inline(always)]
    pub fn drain_into(&mut self, out: &mut [f32; CAP]) -> usize {
        let count = self.write_ptr;
        out[..count].copy_from_slice(&self.samples[..count]);
        self.write_ptr = 0;
        count
    }

    /// Reset the buffer without copying. Discards all pending samples.
    #[inline(always)]
    pub fn reset(&mut self) {
        self.write_ptr = 0;
    }

    /// View the currently buffered samples (read-only).
    #[inline(always)]
    pub fn as_slice(&self) -> &[f32] {
        &self.samples[..self.write_ptr]
    }

    /// Compile-time capacity.
    pub const CAPACITY: usize = CAP;
}

impl<const CAP: usize> Default for RealTimeAudioBuffer<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

// MILESTONE M2 TESTS
// These live here (not just in tests/milestones.rs) so they are co-located with
// the implementation and run on every `cargo test`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_len() {
        let mut buf = RealTimeAudioBuffer::<48_000>::new();
        let frame = [0.5_f32; 160];
        buf.push_frame(&frame).unwrap();
        assert_eq!(buf.len(), 160);
    }

    #[test]
    fn overflow_returns_err_not_panic() {
        let mut buf = RealTimeAudioBuffer::<100>::new();
        let frame = [0.0_f32; 101];
        let result = buf.push_frame(&frame);
        assert!(result.is_err(), "overflow must return Err, not panic");
        assert_eq!(buf.len(), 0, "buffer must be unchanged after overflow");
    }

    #[test]
    fn drain_resets_write_ptr() {
        let mut buf = RealTimeAudioBuffer::<8>::new();
        buf.push_frame(&[1.0, 2.0, 3.0]).unwrap();
        let mut out = [0.0_f32; 8];
        let count = buf.drain_into(&mut out);
        assert_eq!(count, 3);
        assert_eq!(&out[..3], &[1.0, 2.0, 3.0]);
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn is_full_after_exact_capacity() {
        let mut buf = RealTimeAudioBuffer::<4>::new();
        buf.push_frame(&[0.0; 4]).unwrap();
        assert!(buf.is_full());
    }

    #[test]
    fn push_multiple_frames() {
        let mut buf = RealTimeAudioBuffer::<10>::new();
        buf.push_frame(&[1.0, 2.0]).unwrap();
        buf.push_frame(&[3.0, 4.0]).unwrap();
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn reset_discards_samples() {
        let mut buf = RealTimeAudioBuffer::<8>::new();
        buf.push_frame(&[1.0, 2.0, 3.0]).unwrap();
        buf.reset();
        assert!(buf.is_empty());
    }

    // CAP = 0 is pathological but should not panic
    #[test]
    fn zero_capacity_overflow_on_any_push() {
        let mut buf = RealTimeAudioBuffer::<0>::new();
        let result = buf.push_frame(&[1.0]);
        assert!(result.is_err());
    }
}
