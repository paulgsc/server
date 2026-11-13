# Studio takes on a conveyor

An epiphany is ephemeral: a Studio take is recorded at once, asks nothing, and lands as a
draft. Later, on its own Studio page, the owner moves each take along a conveyor of stations
the owner named on 2026-11-11 (paulgsc/some-ui#2876), and only a take ready for pickup is
picked up by the routine and filed as an issue (paulgsc/some-ui epic #2872, CONVEYOR).

| Station | Meaning |
|---|---|
| `in-the-oven` | new: every take is here when recorded |
| `cooking` | being sorted |
| `ready-for-pickup` | the only station the routine files from |
| `picked-up` | filed, with its issue |
| `canned` | set aside and kept |

Home's half is `voice_repo::conveyor` and the table `studio_take_move`
(`migrations/20261113000600_create_studio_take_move.up.sql`), served by `file_host`'s
`handlers::voice`: `GET`/`POST /voice-notes/:id/station` and `GET /voice-notes/conveyor`.
What it holds about a person is docs/identity.md's, invariant 23.

## Invariants

1. **A move is a row, and nothing rewrites one.** Each move is its own row of
   `studio_take_move`; the newest row says where the take is, and a take with no row is in
   the oven. The table is append-only (its trigger aborts an `UPDATE`); its rows go only with
   the account, or with their take when My data deletes it for good.
2. **Moves are ordered by their number, never by a clock.** A move names the take's newest
   move number as the mover saw it (`seq`, 0 for a take in the oven) and the station the take
   was at; home appends it as the next number only if both are still true, and otherwise
   answers `409` with the take's station and newest number. That is the only way a stale
   phone, a second device, or a move resent late is told apart from a current one. A phone's
   clock is never compared: a move's `at` is shown, and bounded to at most
   `voice_repo::conveyor::CLOCK_ALLOWANCE` ahead of home's clock only so a wrong clock cannot
   pin a take in a list. A station lists its takes by when home stored their newest move
   (home's clock), never by `at`. (Coordinator's call under the owner's standing "go with the
   recommended", 2026-11-13, after a guard that compared phone clocks was found to wedge a
   take for good on paulgsc/server#839.)
3. **One rule says which takes are on the conveyor, and every read holds to it.** A `studio`
   note the subject holds that answers no idea (its `ref` does not start `idea:` and no
   `idea_answer` names it) and that the owner has not let go of (`voice_note_set_aside`). A
   take that stops meeting it leaves every station, every read and the take library's
   `station` at once; its moves are kept.
4. **Which moves there are is fixed.** Nothing goes back to the oven; a take reaches
   `picked-up` only from `ready-for-pickup` and always with its issue, and stays there;
   `canned` can come back. The table's `CHECK`s hold the same rules as
   `voice_repo::conveyor::ask`.

*Enforced by* `voice_repo::conveyor`'s tests (`moves_are_ordered_by_number_never_by_a_clock`,
`a_take_moves_along_from_where_it_is_and_keeps_every_move`,
`a_take_that_becomes_an_answer_or_is_let_go_leaves_the_conveyor`, `a_move_is_one_the_conveyor_has`,
`no_move_is_ever_updated_and_moves_leave_with_the_account`) and `handlers::voice`'s
`the_owner_moves_a_take_along_the_conveyor`.
