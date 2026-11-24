# Snips on home

A snip is one picture of one screen of the app, taken on sample data by
paulgsc/some-ui's `Snips Capture` workflow (epic paulgsc/some-ui#3114). The
bundle's contract is some-ui's `docs/snips.md`. This file covers what home
does with snips, and with public pages and walks, the same fetcher's
second and third families (below): `apps/servers/file_host/src/snips.rs`
fetches them, and `essay_media::snips`, `essay_media::pages` and
`essay_media::walks` hand them to the render.

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
the drive, gets no `.gone` mark: only `<commit>.retry`, and it is asked again
next pass, behind the other commits.

## Public pages: the family's second member (paulgsc/some-ui#3395)

some-ui's dispatch-only `Public Pages Capture` photographs public pages
logged out and uploads `pages-bundle-<run>` for one day: `pages.json`
(`some-ui.pages/1`), `SHA256SUMS` and each `<slug>-<hash12>.png` (some-ui's
`docs/snips.md`, "How public pages reach home", and C1420). A cards essay's
page scene names the run that took its picture, its file, its address and
the moment it was taken. `snips.rs` fetches those bundles as it fetches
snips, with the run in place of the commit (`Family::Pages`,
`snips/pages.rs`), into `renders/pages/<run>/`. `essay_media::pages` hands
`<key>.pages/` beside the essay: one `pages.json` holding only the records
the essay names, one per file, and each picture (the render's `--pages`).

The four invariants above hold for pages as written, with these readings:

- **No library, though a page can't be made again.** The cache lives only
  while an essay not yet given names the run. It is fetched on the first pass
  that sees that essay, even while the essay waits on something else (a take
  not heard yet), because the bundle lives one day on GitHub. The essay's
  `<key>.pages/` copy goes with its video. A page is never a reason to take
  an essay back.
- **Gone is said, never retried.** A run that doesn't exist, is of another
  workflow, didn't succeed, whose artifact expired, or
  whose bundle was refused gets `<run>.gone`, and the essay is refused
  `page-gone`. A newer attempt of the same run clears the mark. A timeout or
  the drive, or a run not finished yet, gets only `<run>.retry`. The recovery is the owner's: a new
  dispatch, and a cut list naming the new run's pictures. Home never
  dispatches.

### Call C1440: one pickup over a closed set of families

Decided by PAGES-S for #3395's server half, as the brief asked ("generalize
snips.rs's fetcher over what names a bundle rather than copying it").

`snips.rs` holds one fetcher over `Family { Snips, Pages }`. A family says
only what names a bundle (a commit, or a run), how its run is found (listed
by commit, or `GET /actions/runs/<run>`, which must have `path`
`.github/workflows/public-pages-capture.yml` and `conclusion` `success`),
its artifact's name, its check and its cache folder. Everything else is
shared: the marks, the ordering, and the bounds. There is **one queue over
both families**, so the pass looks up at most five keys and downloads at
most one bundle in all, within the one five-minute deadline. The pickup adds
no time to the mailbox pass. The queue's fairness rule (failed keys to the
back, gone ones last) keeps either family from holding the other back.
Rejected: a second fetcher (a copy that drifts), or bounds per family (twice
the pass's longest hold on the mailbox).

Two small changes reach the snips side. Of several unexpired artifacts with
the bundle's name, the newest is taken, where it used to be the first
listed. The artifact list is asked by name.

### Call C1443: pages go first in the shared queue

Among keys never looked at, and among keys marked gone, a run's pages come
before a commit's snips. Keys whose fetch failed keep their turn, longest ago
first, whatever their family, so a page that keeps failing never holds a
snip back for good. The pass runs hourly and downloads one
bundle, so a day's worth of snips commits queued ahead of a run would
outlive its one-day artifact. A snips bundle is a re-run away, and a page
can't be taken again. Raised by the coordinator on #1144.

### Call C1441: a scene its run doesn't give as named is `page-gone`

Each page scene is checked against its run's `pages.json`. Its `file` must be
there with the scene's `url`, and with the same moment (`capturedAt` compared
as an instant, to the millisecond, as some-ui's `resolveScreens` reads it with
`Date.parse`). If not, the essay is refused `page-gone`, naming the run and the
file and never the address. A re-attempt of a run may upload other pictures
under the same name, and a scene may name the wrong run. Neither can be put
right by waiting, and a page can't be taken again the same. A run already
cached is not fetched again for a newer attempt: a cut list naming a
re-attempt's pictures while the first attempt's bundle is cached is refused
the same way, and the recovery is the same new dispatch. The same holds
when two runs give one file with another address, moment or picture. When
they give the same bytes, it is the same picture, and it is kept once. A
scene with no run, or a run or file no capture makes, is `page-missing`, as is
every page when `SNIPS_GITHUB_TOKEN` is unset.

### Call C1442: a run not finished is waited on, not gone

A run that hasn't concluded yet (`conclusion` null: queued or in progress) is
the failure `unfinished`. It gets only `<run>.retry`, so its essay waits and
is asked again next pass, behind the other keys. A run that concluded other
than `success`, or doesn't exist, is no run and is marked gone. Studio's
words for `page-gone` send the owner to capture the page again, which is the
wrong advice for a run about to succeed. The routine may name a run while it
is in flight, though the capture's own order names it only after it has
succeeded. Asked by ESSAY-Q2 on #1144. A newer attempt under way also clears a
run's gone mark, once a pass reaches it (gone keys come last, as a snip's do), so an essay waits on the owner's re-run rather than being
refused for the attempt before it. A run that never concludes (one waiting
on an approval no one gives) is asked again every pass until GitHub ends
it, and its essay waits that long.

### Gap: an essay taken back loses its pages

`cards_essays::pass::tidy` takes an essay back before its video is made when
an item it was given is no longer on home, or no one's takes are. Taking it
back removes its media folders, `<key>.pages/` included. The essay waits
again, but its run's cache was dropped when it was given. If the day has
passed by the time it is given again, it is refused `page-gone`. A snip is a
re-run away. A page is a new dispatch and a new cut list. This is unchanged
from the snips' shape and left as it is: taking an essay back is rare (an
account gone, or a clip frame gone), and keeping the pictures past their
essay's withdrawal would be a library.

## Walks: the family's third member (paulgsc/some-ui#3476)

Video essays show our own apps in motion. A cards essay's walk scene is
`{"kind": "screen", "of": {"from": "walks", "run": "<digits>", "app":
"<slug>", "steps": [first, last]?}}`. some-ui's dispatch-only `Snips Capture`
(`.github/workflows/snips-capture.yml`) uploads `walks-bundle-<run>` for one
day: `walks.json` (`some-ui.walks/1`: the run's commit and id, and per app
its `some-ui.walk/1` record and its pictures), `SHA256SUMS`, and each
`<app>/snips/<NN>.png`. A walk that failed in that run is left out, never
listed half. `snips.rs` fetches it as `Family::Walks` (`snips/walks.rs`) into
`renders/walks/<run>/`. `essay_media::walks` hands `<key>.walks/`: one
`walks.json` (`some-ui.walks-given/1`, `{"runs": [{"run", "commit", "walks"}]}`)
and each picture at `<run>/<app>/snips/<NN>.png` (the render's `--walks`,
which checks every picture's sha256 again).

The four invariants and the pages' readings hold for walks as written: no
library, the copy goes with the video, gone is said and never retried, and
home never dispatches a run.

### Call C1470: walks are a member of the one family, after pages

Decided by WALKS-S, as the brief asked ("add walks as a member of that
family, with its shared queue, limits and deadline").

A walk's run is found as a page's is, by `GET /actions/runs/<run>`, here
with `path` `.github/workflows/snips-capture.yml` and `conclusion` `success`.
A run not finished is `unfinished` and waited on (C1442). Of several
unexpired `walks-bundle-<run>` artifacts, the newest is taken, so a re-run
attempt's bundle wins when it is fetched. As for a page's run (C1441), a run
already cached is not fetched again for a newer attempt: a scene naming an
app only the re-attempt walked stays `walk-gone`, and the recovery is a new
dispatch named in the cut list. A newest attempt that failed is no run, even
while an earlier attempt's bundle is live, as for pages. The check is whole or nothing. It covers the
schema, the run's own commit (the run's `head_sha`) and id (as a string),
and each app a slug listed once. Each walk must be a `some-ui.walk/1` of
that app with 1 to 99 steps, whose step files are `snips/01.png` on in
order, and whose `pictures` are exactly `<app>/snips/<NN>.png` in the same
order. `SHA256SUMS` must name exactly `walks.json` and every picture, the
bundle hold nothing else, and every file be its sum and its listed bytes.
One walk that doesn't check out refuses the whole bundle, the walks that do
included: a half bundle is never cached.

The one queue now holds three families. Among keys of the same standing,
pages come first, then walks, then snips. A walks bundle lives one day, and
a new run is a new cut list, so a walk goes before a snips commit (a re-run
away under the same key). It goes after a page because a walk can be walked
again on sample data, and a page can't be taken again. Failed keys keep
their turn, longest ago first, whatever their family, so a walk that keeps
failing never holds a page or a snip back for good. The pass still looks up
at most five keys and downloads at most one bundle in all, within the one
five-minute deadline. Rejected: a fetcher of its own, or a bound per family.

### Call C1471: what a walk scene is refused for

- **`walk-gone`**, naming `<run> <app>`. The run is no successful `Snips
  Capture` run, its bundle expired or was refused, the run's `walks.json`
  doesn't list the app (its walk failed in that run), or the scene's last
  step is past the walk's. None of these is put right by waiting. The
  recovery is the owner's: a new dispatch, and a cut list naming the new run.
- **`walk-missing`**: no `SNIPS_GITHUB_TOKEN`, or a scene that names nothing
  a capture makes. That covers a run that isn't a run id string, an app that
  isn't a slug (`^[a-z0-9][a-z0-9-]{0,63}$`), or `steps` that aren't two
  whole numbers with `1 <= first <= last`. Steps count from 1, as the
  pictures' `<NN>` does. No `steps` is the whole walk.
- **A wait**, not a refusal, while the run's bundle is not cached yet and not
  marked gone, as a page's run waits. Every refusal is also looked at again
  on the next pass, so a token added later clears `walk-missing`.

A refusal outranks a wait within the kind, as for every kind.

### Call C1472: a walk is handed whole

`walks.json` given beside the essay holds, per run in the order first named,
the commit and the record of each app the essay names, once, as the bundle
lists it: `{app, walk, pictures}`. Every picture of that walk is handed, not
only the steps a scene names. The render takes the steps from the scene, and
re-checks every picture the record lists, so a record handed with fewer
pictures than it lists would fail there. A walk is at most 99 pictures.

## Not yet

SNIPS2's fourth step puts each record, never a picture, in the mailbox for
the routine. It waits on the mailbox Worker (`paulgsc/some-ui`'s
`apps/mailbox`) gaining a folder for it.
