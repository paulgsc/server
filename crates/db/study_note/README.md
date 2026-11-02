# study_note_repo

The owner's study notes, kept on home: the text the phone writes beside a
soundbite (when it was recorded, how long it ran, what the phone noted around
it) and beside a LeetType spoken answer (the prompt, the rubric, the points
ticked, the answer typed), one closed list of kinds, append-only, per subject,
never parsed (`study_note`). The audio of each is a voice note (`voice_repo`).
The rules are in `src/lib.rs`, in
`migrations/20261101000600_create_study_note.up.sql`, and in
`docs/identity.md`, invariant 30.
