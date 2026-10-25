# voice_repo

The owner's voice notes, kept on home with their audio: every note the phone
sends home, append-only and kept for good, per subject (`voice_note` describes
each note, `voice_note_audio` holds its bytes). The rules are in `src/lib.rs`,
in `migrations/20261025000100_create_voice_note.up.sql`, and in
`docs/identity.md`, invariant 23.
