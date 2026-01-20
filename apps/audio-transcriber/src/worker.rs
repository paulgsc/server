mod queue;
mod whisper;

pub use queue::{TranscriptionJob, TranscriptionQueue};
pub use whisper::start_whisper_worker;

/// Queue capacity - scientifically derived from:
/// - Audio duration per job (buffer_duration_secs)
/// - Whisper RTF (typically 0.8-1.2 on CPU)
/// - Maximum acceptable latency (20-25s for near-live use)
///
/// Formula: CAPACITY = floor(L_max / (D × RTF))
/// Example: floor(20s / (5s × 0.8)) = 5
///
/// DO NOT increase arbitrarily - larger queues = higher latency
pub const TRANSCRIPTION_QUEUE_CAPACITY: usize = 4;
