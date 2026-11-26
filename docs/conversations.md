# Conversation records: home's half (HOST16-S)

A conversation essay is settled as a podcast and then signed as a video (paulgsc/some-ui epic
#3521). Its record is one append-only log of `some-ui.conversation/2` events (paulgsc/some-ui
`packages/video-render/src/conversation.ts`, #3573), and its state is the log replayed through
`stepConversation`, a pure function. Home keeps the log (paulgsc/some-ui C1535) and the
previews the owner approves from (#3558). paulgsc/server#1243 is the story.

```text
GET  /api/v1/conversations/:id                                   the record: { schema, id, events }
GET  /api/v1/conversations/:id/previews                          { previews: [{ preview, about, recommended, variants, approved }], unreadable }
GET  /api/v1/conversations/:id/previews/:preview/:variant/video  a variant's muted loop, with ranges
POST /api/v1/conversations/:id/events                            201 { kept, events } | 200 the same, a resend | 409 { refused, at } | 422
```

- **The log** is `conversation_event` (`conversation_repo`), one row per event, numbered from 1
  per conversation and subject. Its body is the event as sent.
- **The step** is `conversation_repo::step`: home's replay of the record's rules, under the
  record's own names for phases, events and refusals, so the phone reads a `409`'s `refused`
  with `refusalLine`.
- **The previews** are files the renderer leaves at
  `renders/conversation/<subject>/<id>/<preview>/preview.json` (`some-ui.conversation-preview/1`:
  `{ schema, preview, about, recommended, variants: [{ variant, title, rule, callouts }] }`,
  the card's `PreviewOfferSchema`) beside each `<variant>.mp4`, written manifest last
  (`file_host::conversations`). A preview is offered once its manifest and every variant's
  video are there.

## Why a table of its own (reaching inward first)

No existing log takes it. `studio_sitting_event` and `leetype_session_event` are per-record
logs, but ordered by their events' own ids and times and holding a sitting's or session's own
columns. `app_event` keys an event by id within a family, with no record and no order.
`studio_take_move` is ordered by number, as this is, but its columns are a conveyor move's. A
conversation's order is its log's position, which the step reads, so generalizing any of them
means rebuilding an append-only table to change what its order means. That rebuild would be a
discard site (docs/schema.md). The table takes the conveyor's shape instead: a number per
record, from 1, with no gap.

## Invariants

1. **The log is append-only, and a refused event is never kept.** `POST …/events` replays the
   kept log with the new event in one `BEGIN IMMEDIATE` transaction, and writes only when the
   step takes it. Its trigger aborts every `UPDATE` of a row, and rows leave only with the
   account. *Enforced by* `conversation_repo`'s `a_refused_event_changes_nothing`,
   `the_table_is_append_only_gapless_and_leaves_with_the_account` and
   `the_migration_round_trips_and_refuses_to_drop_a_record`, and by `handlers::conversations`'
   `a_refused_event_answers_why_and_writes_nothing`.
2. **An event's place is its number, never a clock** (paulgsc/some-ui C1580, as
   docs/conveyor.md invariant 2). `seq` is the next in its log (a trigger refuses any other),
   and the replay reads events in that order. An event's `at` is its writer's clock, kept and
   never compared, since the log's writers (the phone, Studio, the agent in the cloud) each
   have their own. A writer that names `basis`, how many events the log held when it wrote,
   is refused `stale` (`{ why, basis, seq }`) when the log has moved since: a pick made on an
   old read is refused, never kept after an event it did not see. *Enforced by*
   `the_table_is_append_only_gapless_and_leaves_with_the_account`,
   `an_event_written_against_an_older_log_is_stale`,
   `order_is_the_logs_and_a_writers_clock_is_never_compared` and
   `a_refused_event_answers_why_and_writes_nothing`.
3. **A resend keeps nothing twice.** An event equal to the log's last is answered `200` at its
   index, and nothing is written. So a pick the phone sends again after a lost answer is not
   refused as a second approval. *Enforced by*
   `a_retried_event_is_kept_once_and_a_second_approval_is_refused`.
4. **Home holds the log's structural rules exactly, and only those** (to C1581). Home holds
   `basis` (`stale`), each kind's author (`EVENT_AUTHORS`: the owner's topic, points, flags,
   withdrawals, green light and approvals, the agent's versions and notes, home's measure),
   which events each phase takes, and points once and at most forty. It holds each version
   numbered next and answering every kind of every open flag once, and each flag once, on the
   version the owner heard and on a turn or speaker it has. It holds a withdrawal of an open
   flag only, the measure and green light of the newest version, under twenty minutes, notes
   only once green-lit, on the newest version's turns whose words the record holds, within
   them and following at least one approved rule, and each preview approved once, from what
   it offered, a rule set again only by naming the approval it moves.
   Home does not port the content rules that rest on some-ui's closed vocabularies: the cast
   and its roles, each character's emotions, the flag kinds' voices, scopes and answers,
   Kokoro's voices, and how one version's turns differ from the last's. Home keeps `/2`
   records only, so a `/1` record's lift (`lifted`) is never one of its rules. A second copy of those
   lists would drift from the first. Every writer runs `stepConversation` before it sends, and
   home trusts it for those. So an event home refuses is one `stepConversation` refuses, with
   the same `why`, while an event home keeps may still break a content rule a writer skipped.
   *Enforced by* `conversation_repo::step`'s tests, one per rule. *Not enforced:* parity with
   `stepConversation`, which a rule added on some-ui's side breaks until home ports it or this
   list names it as trusted. some-ui's contract harness (`conversations.contract.ts`, which
   lands once these routes are in its snapshot) holds refusals both sides share, against a
   real `file_host`.
5. **An approval names a preview home offers, exactly as home offers it.** Beyond the step,
   home answers `409` `preview-unknown` for a preview it has no ready files for. It answers
   `preview-offer` when the event's `offered` (in home's order), `recommended`, or the chosen
   variant's `rule` is not the manifest's. So no rule is set from a preview the owner never
   watched. *Enforced by* `an_approval_names_a_preview_exactly_as_home_offers_it` and
   `an_approval_is_kept_only_for_a_preview_home_offers_as_it_offers_it`.
6. **Previews are the subject's, and every name that reaches a path is one plain segment.** A
   conversation's id is unique only within its subject, so its previews live under the
   subject's folder, and every read (the list, an approval's check, a video) takes the caller's
   subject: another subject's are a `404`. The subject's, a conversation's, a preview's and a
   variant's id on home are each a `showId` (`^[a-z0-9][a-z0-9-]{0,63}$`); any other is `404`,
   or is not listed. *Enforced by* `another_subjects_previews_are_none_of_the_callers` and
   `a_video_is_only_ever_one_plain_segment_per_name`.
7. **Nothing of a record reaches a log line.** Each handler records only `otel.kind`. A
   drive error logs its kind, never a path.
8. **A manifest reads as the card's `PreviewOfferSchema` reads it.** Its bounds are that
   schema's: a heading of at most 80 characters, a title of 60, 1 to 12 callouts of 80, names
   of 64, each trimmed first, 2 or 3 variants each once with the recommended one among them;
   home also holds preview and variant ids to a `showId`, since they name folders. A manifest
   past any of them is `unreadable`, never offered. *Not enforced:* parity with the schema,
   which the follow-up's shared fixtures hold.
9. **A write holds no file open under its lock.** An approval's preview is read before
   `BEGIN IMMEDIATE`, and confirmed inside it by one `stat` of its manifest, read again only
   if it changed.
10. **Home keeps `/2` records only.** `POST` takes one `/2` event at a time, so a `/1` record
    lifted into `/2` (`lifted`) never comes through home, and `GET` never answers one.
