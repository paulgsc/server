# app_event_repo

What the app saw happen as it ran, kept on home: one event per obligation the
phone ran to an ending (a take today; a listening, a spoken line and a note sent
home next), saying how it ended, how long it took, which screen it came from and
which build (paulgsc/some-ui CK2, #2381). Append-only and per subject, ids only,
one table for every family of event. The rules are in `src/model.rs`; the design
is in `migrations/20261103001700_create_app_event.up.sql`.
