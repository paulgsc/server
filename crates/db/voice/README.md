# voice_repo

The owner's voice notes, kept on home with their audio: every note the phone
sends home, append-only and kept for good, per subject (`voice_note` describes
each note, `voice_note_audio` holds its bytes), and what home heard in each
(`voice_transcript`, one row per attempt, written by `file_host`'s
transcriber). The rules are in `src/lib.rs` and `src/transcript.rs`, in
`migrations/20261025000100_create_voice_note.up.sql` and
`migrations/20261025000200_create_voice_transcript.up.sql`, and in
`docs/identity.md`, invariant 23.
