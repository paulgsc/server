# call_week_repo

Home's record of the weekly call's weeks (paulgsc/some-ui#2635, ST36):
`call_week`, which `file_host`'s `call_weeks` worker fills. The rules are in
`migrations/20261106000100_create_call_week.up.sql`. A week the routine
re-stamps while no part of it has a video is appended as its next revision
in `call_week_revision` (paulgsc/server#879), never rewritten; the newest
revision is the one home serves. Its rules are in
`migrations/20261115000100_create_call_week_revision.up.sql`.

Its `video` module keeps each video's `YouTube` link and the receipts of what
the owner let go, a part at a time (paulgsc/some-ui#2918): `call_part_link`
and `call_part_deletion`, keyed (week, part), whose rules are in
`migrations/20261114000200_create_call_part.up.sql`. A week of one video is
part 1, keyed by its day as before; the tables before parts
(`call_video_link`, `call_video_deletion`) are kept whole, frozen, as the
copy part 1 was made from.
