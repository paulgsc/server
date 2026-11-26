# Conversation records: home's half (HOST16-S)

A conversation essay is settled as a podcast and then signed as a video (paulgsc/some-ui epic
#3521). Its record is one append-only log of `some-ui.conversation/2` events (paulgsc/some-ui
`packages/video-render/src/conversation.ts`, #3573), and its state is the log replayed through
`stepConversation`, a pure function. Home keeps the log (paulgsc/some-ui C1535) and the
previews the owner approves from (#3558). paulgsc/server#1243 is the story.

```text
GET  /api/v1/conversations/:id                                   the record: { schema, id, events }
GET  /api/v1/conversations/:id/previews                          { previews: [{ preview, recommended, variants, approved }], unreadable }
GET  /api/v1/conversations/:id/previews/:preview/:variant/video  a variant's muted loop, with ranges
POST /api/v1/conversations/:id/events                            201 { kept, events } | 200 the same, a resend | 409 { refused, at } | 422
```

- **The log** is `conversation_event` (`conversation_repo`), one row per event, numbered from 1
  per conversation and subject. Its body is the event as sent.
- **The step** is `conversation_repo::step`: home's replay of the record's rules, under the
  record's own names for phases, events and refusals, so the phone reads a `409`'s `refused`
  with `refusalLine`.
- **The previews** are files the renderer leaves at
  `renders/conversation/<id>/<preview>/preview.json` (`some-ui.conversation-preview/1`) beside
  each `<variant>.mp4`, written manifest last (`file_host::conversations`). A preview is
  offered once its manifest and every variant's video are there.

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
2. **An event's place is its number, with no gap.** `seq` is the next in its log (a trigger
   refuses any other), and the replay reads events in that order. *Enforced by*
   `the_table_is_append_only_gapless_and_leaves_with_the_account`.
3. **A resend keeps nothing twice.** An event equal to the log's last is answered `200` at its
   index, and nothing is written. So a pick the phone sends again after a lost answer is not
   refused as a second approval. *Enforced by*
   `a_retried_event_is_kept_once_and_a_second_approval_is_refused`.
4. **Home holds the log's structural rules exactly, and only those.** Home holds which events
   each phase takes, time order, and points once and at most forty. It holds each version
   numbered next and answering every open flag once, and each flag once, on a turn or speaker
   the newest version has. It holds a withdrawal of an open flag only, the measure and green
   light of the newest version, under twenty minutes, notes on the newest version's turns
   within their words and approved rules, and each preview approved once, from what it offered.
   Home does not port the content rules that rest on some-ui's closed vocabularies: the cast
   and its roles, each character's emotions, the flag kinds' voices, scopes and answers,
   Kokoro's voices, and how one version's turns differ from the last's. A second copy of those
   lists would drift from the first. Every writer runs `stepConversation` before it sends, and
   home trusts it for those. So an event home refuses is one `stepConversation` refuses, with
   the same `why`, while an event home keeps may still break a content rule a writer skipped.
   *Enforced by* `conversation_repo::step`'s tests, one per rule. *Not enforced:* parity with
   `stepConversation`, which a rule added on some-ui's side breaks until home ports it or this
   list names it as trusted. some-ui's contract harness (`conversations.contract.ts`) holds the
   refusals both sides share, against a real `file_host`.
5. **An approval names a preview home offers, exactly as home offers it.** Beyond the step,
   home answers `409` `preview-unknown` for a preview it has no ready files for. It answers
   `preview-offer` when the event's `offered` (in home's order), `recommended`, or the chosen
   variant's `rule` is not the manifest's. So no rule is set from a preview the owner never
   watched. *Enforced by* `an_approval_names_a_preview_exactly_as_home_offers_it` and
   `an_approval_is_kept_only_for_a_preview_home_offers_as_it_offers_it`.
6. **Every name that reaches a path is one plain segment.** A conversation's, a preview's and
   a variant's id on home is a `showId` (`^[a-z0-9][a-z0-9-]{0,63}$`). Any other is `404`, or
   is not listed. *Enforced by* `a_video_is_only_ever_one_plain_segment_per_name`.
7. **Nothing of a record reaches a log line.** Each handler records only `otel.kind`. A
   drive error logs its kind, never a path.

## Known hazard: the record's time order compares writers' clocks

`stepConversation` refuses an event dated before the log's newest (`out-of-time`), and home
holds the same rule (invariant 4). The log has more than one writer: the phone, Studio, and
the agent in a cloud session. So a writer whose clock runs behind another's is refused until
its clock passes the other's. That is the same class of wedge docs/conveyor.md invariant 2
records from paulgsc/server#839. Until the schema decides otherwise, a writer dates an event
no earlier than the record's newest `at`, which it reads from `GET /conversations/:id`. Raised
on paulgsc/server#1243 for the coordinator and the owner.
