# Offline Voice-to-Text for Livestreaming: Project Roadmap with Pseudo-Code Challenges

## 🧱 Overview
This roadmap breaks the system into progressive, modular milestones, each accompanied by pseudo-code challenges that resemble LeetCode-style exercises to reinforce key implementation concepts in Rust.

---

## 1. Input Layer — Mic Audio Capture

### Goal:
Capture short audio buffers from the microphone using `cpal`.

### Subtasks:
- Select default input device
- Stream audio as f32 samples
- Pipe samples into a buffer

### LeetCode-style Challenges:
1. **DeviceEnumerator**
   ```rust
   fn list_input_devices() -> Vec<String>
   ```
   *Input:* none
   *Output:* vector of device names

2. **MicStreamSampler**
   ```rust
   fn stream_audio(device_name: &str, duration_secs: u32) -> Vec<f32>
   ```
   *Input:* device name, stream duration
   *Output:* vector of audio samples

---

## 2. Buffering Layer — Convert and Chunk Audio

### Goal:
Batch and format audio into `.wav` format that Whisper can consume.

### Subtasks:
- Convert f32 samples to i16 PCM mono
- Chunk buffers by time window (e.g., 10 seconds)
- Encode to `.wav`

### LeetCode-style Challenges:
3. **F32ToPCM**
   ```rust
   fn convert_f32_to_i16(input: &[f32]) -> Vec<i16>
   ```
   *Input:* vector of samples
   *Output:* 16-bit PCM samples

4. **ChunkBuffer**
   ```rust
   fn chunk_audio(samples: &[i16], chunk_size: usize) -> Vec<Vec<i16>>
   ```
   *Input:* entire audio buffer, desired chunk size
   *Output:* vector of audio chunks

5. **WriteWav**
   ```rust
   fn write_wav(path: &str, samples: &[i16], sample_rate: u32)
   ```
   *Input:* file path, samples, and sample rate
   *Effect:* writes a valid mono `.wav` file

---

## 3. Transcription Layer — Whisper Integration

### Goal:
Use Whisper to transcribe `.wav` files into text.

### Subtasks:
- Shell out to `whisper.cpp` CLI
- Capture stdout of process
- Return transcribed string

### LeetCode-style Challenges:
6. **RunWhisper**
   ```rust
   fn run_whisper_on_file(wav_path: &str) -> Result<String, Error>
   ```
   *Input:* path to `.wav`
   *Output:* transcript as a string

7. **TranscribeBatch**
   ```rust
   fn transcribe_audio_chunks(paths: &[String]) -> Vec<String>
   ```
   *Input:* list of paths
   *Output:* list of transcripts

---

## 4. Output Layer — Socket-Based Streaming

### Goal:
Transmit text to a UI overlay using local Unix sockets.

### Subtasks:
- Serialize transcript as JSON
- Send JSON via Unix socket
- Support multiple transcript updates

### LeetCode-style Challenges:
8. **SerializeTranscript**
   ```rust
   fn to_json(transcript: &str, timestamp: u64) -> String
   ```
   *Input:* transcript string, timestamp
   *Output:* JSON string

9. **SendToUnixSocket**
   ```rust
   fn send_json(socket_path: &str, json_payload: &str) -> Result<(), Error>
   ```
   *Input:* path to socket, JSON string
   *Effect:* sends data over socket

10. **StreamingServer**
   ```rust
   fn start_unix_socket_server(socket_path: &str)
   ```
   *Effect:* listens on socket and prints received messages (mock UI)

---

## 5. Integration — Full Pipeline Prototype

### Goal:
Hook everything together into a prototype pipeline that:
- Captures from mic
- Buffers and converts
- Transcribes
- Streams result to UI

### LeetCode-style Integration Challenge:
11. **VoiceToTextPipeline**
   ```rust
   fn run_pipeline(config: Config) -> Result<(), Error>
   ```
   *Effect:* drives the full system from mic to overlay

---

## 6. Refactor for Streaming Mode

### Goal:
Refactor pipeline for continuous streaming using async/threads.

### LeetCode-style Concurrency Challenges:
12. **AudioFifoBuffer**
   ```rust
   struct AudioBufferQueue {
       queue: Arc<Mutex<VecDeque<Vec<i16>>>>,
   }

   impl AudioBufferQueue {
       fn push(&self, chunk: Vec<i16>)
       fn pop(&self) -> Option<Vec<i16>>
   }
   ```

13. **TranscriberThread**
   ```rust
   fn spawn_transcriber(queue: Arc<Mutex<VecDeque<Vec<i16>>>>, socket_path: &str)
   ```
   *Effect:* Continuously pops chunks, transcribes, and sends JSON

---

---

## 🎯 Project Goals (summarized)

- **Voice → Text**, processed locally, fed to a **chatbot UI**
- Built for **livestream overlay**, but latency is **not critical**
- Must run **offline**
- **As lossless as possible** (accuracy over speed)
- Prefer **low-dependency, deeply understood** components
- Want to **enjoy building** it, not just wiring up other people's crates

---

## 🧱 Layered System Architecture

Let’s break the pipeline into discrete, composable layers:

### 1. **Input Layer** – Capture from Mic
- 📌 **What it does:** Collects short audio buffers from your microphone
- 🔧 Minimal deps: Use `cpal` or even direct access to `/dev/audio` (if you’re brave)
- ✨ Alternative: Write a small wrapper around ALSA/Pulse if you want ultra-low-level
- 🧠 Your control level: High — you’ll know exactly when and how data is captured

---

### 2. **Buffering Layer** – Chunk Audio for Inference
- 📌 **What it does:** Batches mic input into usable `.wav`-like buffers
- 🔧 You’ll likely convert float32 samples into PCM format (e.g., 16-bit mono)
- ✨ Could write your own WAV writer or use `hound` (pure Rust, very light)
- 📦 Optional: FIFO buffer to allow for asynchronous transcription without data loss

---

### 3. **Transcription Layer** – Run Whisper Inference
- 📌 **What it does:** Converts buffered audio to text
- 🔧 Minimal path: shell out to `whisper.cpp` via CLI with `.wav` input
- ✨ Future: integrate via FFI or use `whisper-rs` if you're okay with that level of vendor dependency
- 🎯 Ideal path: You understand exactly how the model is being used, and maybe even tweak it later

---

### 4. **Output Layer** – JSON Streaming / UI Integration
- 📌 **What it does:** Feeds the transcribed text to your chatbot overlay
- 🔧 Zero-dep: You can open a Unix socket or pipe if you're keeping it all local
- ⚙️ Else: A lightweight HTTP/WebSocket server (e.g., `hyper`, `axum`, or your own minimal TCP socket)

---

## 🔄 Data Flow Example

```text
Mic (via cpal or raw ALSA) 
   ↓
Chunker (ring buffer or fixed 10s window)
   ↓
Save buffer as .wav (or feed directly to whisper.cpp)
   ↓
whisper.cpp -> transcript.txt
   ↓
Parse & stream result (stdout -> JSON to local client)
   ↓
Chatbot UI renders text live
```

---

## 🔌 Interface Between Layers

You want **explicit, decoupled boundaries** between these layers. Options:

- **File-based**: mic writes `.wav` to temp dir → whisper reads it
- **Pipes/FIFO**: whisper reads from stdin (harder, but clean)
- **Unix sockets**: for Rust ↔ UI integration

These boundaries let you:
- Debug each part independently
- Replace parts gradually (e.g., swap in a custom model later)
- Avoid over-coupling vendor crates

---

## 🔎 Reasoning Summary

| Layer | Options (Low-dep) | Reasoning |
|-------|-------------------|-----------|
| Mic Input | `cpal`, raw ALSA | Enough control, very hackable |
| Buffering | DIY WAV buffer, or `hound` | WAV format is simple to write |
| Inference | CLI whisper.cpp | No vendor lock-in, shell control |
| Output | Unix socket / custom TCP | Integrate with chatbot cleanly |

---

## 🪜 Recommended Next Steps

1. **Design your inter-layer boundaries**: Decide if you're going file-based or streaming between mic + model.
2. **Prototype the whisper integration first**: get a .wav file transcribed, so you know your endpoint.
3. **Backfill mic input and buffering**: mic → .wav writer that conforms to the format whisper expects.
4. **Add a simple JSON emitter**: pipe the results into your chatbot UI or file log.
5. **Refactor to add streaming mode**: after happy path works.

---

