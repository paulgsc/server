# conversation_repo

A conversation essay's record, kept on home (paulgsc/server#1243, HOST16-S):
one append-only log of `some-ui.conversation/2` events per conversation, per
subject (`conversation_event`), replayed through the record's step before
each event is kept, so a refused event is never kept. The rules are in
`src/lib.rs` and `src/step.rs`, in `docs/conversations.md`, and in
`docs/identity.md`, invariant 40.
