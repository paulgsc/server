# Snips on home

A snip is one picture of one screen of the app, taken on sample data by
paulgsc/some-ui's `Snips Capture` workflow (epic paulgsc/some-ui#3114). The
bundle's contract is some-ui's `docs/snips.md`. This file covers what home
does with snips: `apps/servers/file_host/src/snips.rs` fetches them, and
`essay_media::snips` hands them to the render.

## Invariants

1. **Home keeps no picture library.** The owner ruled on 2026-11-21: "If it's
   something a CI run can create at any time, there's no need to duplicate
   it by persisting it on home." A capture is deterministic at a commit, so
   home holds a bundle only while an essay it has not yet given names that
   commit. The bundle sits in `renders/snips/<commit>/`, a cache. The essay's
   copies in `<key>.snips/` go with its video. No table holds a snip.
2. **What home keeps of a snip is the cut list.** An essay names each snip by
   its id and the commit its bundle was captured at (C868). `cards_essay`
   keeps the cut list append-only. That is the record of which snip and
   commit each essay used, and how to make the snip again (SNIPS5).
3. **Fetched on demand, bounded, from the mailbox pass.** There is no timer of
   its own (home-compute invariant 6). It runs last in the pass, after what a
   phone or the routine waits on. A pass looks up at most five commits (those
   never looked at first, those marked gone last) and downloads at most one
   bundle, of at most 256 MiB, streamed to the drive, 512 MiB unpacked. The
   whole pickup ends at five minutes: that is the longest it holds up the
   mailbox pass. A commit whose fetch failed goes to the back of the queue
   (`<commit>.retry`), so none holds another back.
4. **Checked whole, or refused whole.** A bundle is used only once its
   schema, commit, `SHA256SUMS` and every file check out (some-ui's
   `docs/snips.md`).

## Call C875: an expired bundle is refused, never dispatched again

Decided by W17 for the #3311 follow-on, accepted by the coordinator on
2026-11-21.

**When home can't get a bundle.** This covers three cases: a commit with no
successful run, a run whose artifact has expired (a bundle lives one day on
GitHub), and a bundle that does not check out. Home marks the commit
`<commit>.gone`, naming the run, and refuses the essay `snip-gone`. The
failed log on Studio › Renders asks the owner to run Snips Capture again at
that commit.

**Why home never dispatches a run itself.** Dispatching needs Actions: write.
`SNIPS_GITHUB_TOKEN` is Actions: read only, and widening it is the owner's
call. It would be raised only if these refusals turn out to be common.

**How a commit recovers.** A newer run of that commit, or a newer attempt of
the same run (GitHub's Re-run keeps the run id), clears the mark, and the
bundle is fetched. A failure that may clear on its own, such as a timeout or
the drive, is marked nowhere and is asked again next pass.

## Not yet

SNIPS2's fourth step puts each record, never a picture, in the mailbox for
the routine. It waits on the mailbox Worker (`paulgsc/some-ui`'s
`apps/mailbox`) gaining a folder for it.
