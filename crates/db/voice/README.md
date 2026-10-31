# voice_repo

The owner's voice notes, kept on home with their audio: every note the phone
sends home, append-only and kept for good, per subject (`voice_note` describes
each note, `voice_note_audio` holds its bytes), and what home heard in each
(`voice_transcript`, one row per attempt, written by `file_host`'s
transcriber), with each heard word, when it was said, how sure the model
was of it and the language it was heard in beside a transcript that heard
something (`voice_transcript_word`, written with it;
`migrations/20261030000300_create_voice_transcript_word.up.sql`,
`20261031000200` for how sure and `20261031000300` for the language). A game bite is a voice note said for `game`, and what it was
said about and what the screen showed is `game_bite`'s row beside it, written
with the note
(`migrations/20261028000100_add_game_bite_to_voice_note.up.sql`; a bite said
first may keep the scenario picked after it since
`migrations/20261028000300_let_a_said_first_bite_keep_its_scenario.up.sql`). A
Studio take is a voice note said for `studio`, its `ref` the take
(`<project>:<shot>:<take>`), with nothing beside it; a redo is a new note
(`migrations/20261030000100_add_studio_to_voice_note.up.sql`). The rules are in `src/lib.rs` and `src/transcript.rs`, in
`migrations/20261025000100_create_voice_note.up.sql` and
`migrations/20261025000200_create_voice_transcript.up.sql` (and the word
table's migration above), and in
`docs/identity.md`, invariant 23.
