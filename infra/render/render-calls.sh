#!/usr/bin/env bash
# Home's render worker (GWB10, paulgsc/some-ui#2084), with eight jobs, one
# loop: the weekly call's (paulgsc/some-ui#2635, ST36), Playoff Delta's
# (FEED4, paulgsc/some-ui#2647), a template's preview (SFEED12,
# paulgsc/some-ui#2757), a Rust Rounds essay's (RR16, paulgsc/some-ui#2691),
# a clip's walk (paulgsc/some-ui#2670's clips, C125), a cards essay's
# (CARDS7, #854, paulgsc/some-ui#2979, C327), a week of mock interview's
# essay (IW6, paulgsc/some-ui#2937, #899) and a week's engagement report
# (ENG12, paulgsc/some-ui#2700, #821).
# infra/compose/call-render.yml runs it.
#
# Each pass, for each job, for each episode file_host has written for it with
# no video in the job's renders folder, it records the stage (the slice of
# the app its image carries at /opt/stage, which the recorder serves itself,
# #718) with each scene said by home's speaker (`kokoro`), renders it, and
# keeps the result: the video, its takes and stills, its cut list and what it
# was made from. The same episode gives the same files.
#
# | job           | file_host writes (read-only here)      | kept in                     | video       |
# |---------------|----------------------------------------|-----------------------------|-------------|
# | call          | /weeks/<from>.json (call_weeks)        | /renders/call/<part key>/   | call.mp4    |
# | playoff-delta | /episodes/<key>/{fill,season}.json     | /renders/playoff-delta/<key>/ | episode.mp4 |
# | preview       | /previews/<id>.json (previews)         | /renders/preview/<id>/      | preview.mp4 |
# | rust-essay    | /essays/<week>/essay.json (essays)     | /renders/rust-essay/<week>/ | essay.mp4   |
# | clip-walk     | /clip-walks/<clip>-<side>/{sample.json,app} (clips) | /renders/clip-walk/<clip>-<side>/ | walk.json |
# | cards-essay   | /cards-essays/<key>.json (cards_essays) | /renders/cards-essay/<key>/ | cards.mp4   |
# | interview-essay | /interview-essays/<week>/essay.json (essays) | /renders/interview-essay/<week>/ | essay.mp4 |
# | report        | /report-weeks/<week>.json (report_weeks) | /renders/report/<week>/   | report.mp4  |
#
# A week of the call is one video or a series of parts (template version 5,
# paulgsc/some-ui#2918): each part a video of at most ten minutes, cut at a
# chapter by the recorder's own code. The week's first job asks the
# recorder's `render call-parts` how many, inside the job the lane admitted
# (no work starts outside the lane), and keeps the answer in
# /renders/call/<from>.parts, which file_host reads too
# (`call_weeks::render::parts`); a week with none is one part, as every
# week rendered before parts is. Each part is then its own job, keyed
# (week, part): part 1 by the week's day, as a week of one video always
# was, and part k by <from>-p<k>, each recorded with `--part k`, kept in
# /renders/call/<key>/, resumed from its own step cache, failed and retried
# on its own, and shown on the Renders board as part k of n with the
# recorder's scene count within the part. A part the owner let go while
# another still has its video is marked /weeks/<key>.let-go by file_host,
# and is never rendered again; the week's file goes with its last part. A
# week of one part records exactly as before, with no --part.
# A Playoff Delta key is <season>-<team>-<NN> (2026-SF-08, NN 00 the legend);
# file_host writes only fills of a voiced version (pd_fills::render), so a
# fill written for the owner's own voice is never tried here, never failed.
# A preview's id is <template>-<UTC yyyymmddThhmmssZ> (conference-call-
# 20261108T074500Z): the owner asked home for one of the template's sample
# (`POST /previews/:template`). It records the template's sample week and
# renders its preview's cut. One that fails is never tried again: the owner
# asks for another. file_host removes each preview a day after it is made,
# and an older one of its template once a newer one is ready (`previews`);
# nothing here deletes one.
# A Rust Rounds essay's key is its week (2026-w46). file_host gives the
# worker an essay only once its week is final, and only one that plays
# exactly the takes the week's final record names (`rust_essays`): the
# worker reads no record and no draft. The tutor's lines are said by home's
# speaker as every job's; each answer scene is the owner's take, played
# whole by the recorder.
# A week of mock interview's essay is the same, by the same file_host
# export over its kind (`rust_essays`): keyed by its week, given once the
# interview week is final and only when it plays exactly the answers the
# final names, each take <question>.<webm|ogg|m4a> in its takes/ folder and
# each card read read.<card>.<ext> beside them. The same recorder records
# both kinds (`record:essay` reads the essay's schema).
# A clip's walk is keyed <clip>-<side> (ledger-wake-after): file_host gives
# it the walk's sample and app (`clips`), and the motion replays the app's
# scripted walk over that sample (`motion walk`), its snips and walk.json
# into the finished folder; nothing is said and no video is made. file_host
# checks the walk against the clip index before it keeps a frame of it, so a
# walk this image draws differently is never kept. It is drawn by this
# image's some-ui, not the commit of the change the clip shows: nothing here
# reaches the internet to fetch another.
# A cards essay's key is its slug and its stamp's day
# (nixos-upgrade-2026-11-14): file_host gives the worker each essay
# paulgsc/testing's cards index lists, as first served (`cards_essays`), and
# the recorder draws each card on the stage's burn-ins page and renders it,
# refusing one that isn't a cards essay before anything is drawn
# (`record:cards`). A spoken essay (the cheat-sheet series' v2) has each
# scene said by home's speaker, as the call's are, its chapter's intro
# included (EXPLAIN6, #878, paulgsc/some-ui#3021): a podcast with cards on
# screen, its words mattering more than its picture. An older, silent one
# (v1) still renders, silent. Essays go by their day, oldest first, not by
# name.
# A week's engagement report is keyed by its ISO week (2026-W46): file_host
# gives the worker home's own engagement counts for the week and the three it
# is compared with, once the week has ended (`report_weeks`), and the
# recorder writes the report from them by code under the owner's
# public-safety rule (`record:report`: `reportWeek`, no model), records the
# report's stage with each scene said by home's speaker and renders it. It
# refuses, before anything is drawn, a week still under way or edges other
# than the owner's week. The same file gives the same video.
#
# One episode at a time, the call's first, each job's oldest first. An
# episode that fails is left as it was and tried again an hour later; a
# finished one is moved into place whole, so a folder in a job's renders folder is
# always a finished video. Nothing here deletes a finished video: the owner
# lets a week go from My data, once its YouTube link is recorded, and
# file_host deletes it (`call_weeks::videos`), first taking the week's file
# out of /weeks so it is never rendered again (paulgsc/some-ui#2661).
#
# What it reports, per job, for file_host to read (`call_weeks::render`,
# `pd_fills::render`; the CALL dashboard and the Call* and PlayoffDelta*
# alerts): <status>/last-pass, the Unix time each pass started (and, while a
# render runs or waits for room, at least every EVERY seconds), and
# <status>/<key>.failed.log, the whole output of an episode's last failed
# attempt, removed once it renders. The same output is on stdout too, for
# `docker logs render`.
#
# A waiting episode's work, its step cache included, is kept between attempts
# in <work>/<key>, so a recorder that keeps its steps (paulgsc/some-ui#2658)
# reruns only what changed, and leaves with the finished episode. What a
# render may take is policy, shared by the jobs; nothing past it is silent: a
# step not cached (a `cache warning:` line from the recorder) is kept in
# <status>/<key>.cache.log, and an episode's work removed past the total cap
# in <status>/work.cache.log, both read by file_host (CallRenderCacheFull).
#
# **The lane decides when a render runs** (L4c, paulgsc/server#708;
# docs/home-compute.md, "The lane"). No clock starts a render: file_host
# writes the lane's decision to <lane>/lane every five seconds (`verb` run,
# pause or stop; `at`, Unix seconds), and the worker obeys it, each job alike:
#
# - **A job** is one episode's attempt, from its first try until it is
#   rendered, fails or is let go. It gets an id, written to <lane>/job and
#   never reused, and keeps it through every pause and stop, so the lane
#   counts its stops against it alone (LA4) and a resumed job is not
#   admitted as new. Whichever job runs writes the one <lane>/job: the lane
#   admits one batch job at a time. <lane>/job.state says which it is and
#   where it is, for the phone's Renders page (SFEED13, paulgsc/some-ui#2761).
# - **It starts** only on a run decision written after its id was, so the
#   lane has seen it and found room for its peak (LA2). A missing decision,
#   or one older than STALE_AFTER, is stop.
# - **Pause** freezes the recorder and everything it started (SIGSTOP, then
#   SIGCONT on run): it keeps its memory and takes no CPU.
# - **Stop** kills them, letting go of their memory; the job waits for room
#   and runs again over the same work, whose step cache keeps every step
#   that had finished, so nothing is paid for twice. An attempt that fails
#   soon after a pause (a page's timeout runs on while frozen) is treated
#   the same way, PAUSE_FAILS times at most. A yield is never a failure: no
#   failed.log. A job that let go of its memory without the lane's stop is
#   taken away and named again, so the lane checks room before it runs.
# - **Its peak**, after it renders: the render container's memory.peak plus
#   what the speaker and the stage take beside it, written to <lane>/peak,
#   so the lane admits the next job against a measurement, not the declared
#   ceiling.
#
# A container restart keeps the job: <lane>/job.state names it, it runs
# first, and its work is never removed for WORK_MAX_MB while it waits. The
# first render after a boot waits for the lane's first run, which is never
# sooner than its hold-down: home's own services start first. A restart the
# worker was not asked for (no TERM: the container died under the job, an
# OOM inside it, say) is counted in job.state's `restarts`; at RESTARTS the
# job is taken as failed, as an exit is, so a render that kills its own
# container is not resumed first on every restart for good (#740).
set -uo pipefail

# Policy (paulgsc/some-ui#2635, the owner's caps). The recorder refuses a
# video longer than its job's minutes (`max_minutes`: MAX_VIDEO_MINUTES, or
# ESSAY_MAX_VIDEO_MINUTES for an essay, Rust Rounds or interview; from its words, before
# anything is said) or larger than MAX_VIDEO_MB (removed once made), a
# failed attempt that names the cap; it stops keeping an episode's steps
# past CACHE_MB, with a `cache warning:` line. Each job's waiting work
# together is held to WORK_MAX_MB: past it, the oldest other episode's work
# is removed (its retry starts over).
MAX_VIDEO_MINUTES=15
# The owner, 2026-11-17, asked whether a Rust Rounds essay may run past 15
# minutes: "yes raise the cap to 30 mins" (#927). On 2026-11-19 the owner
# ruled that a length cap is a suggestion an agent moves to meet a moved
# story ("just update the duration limit or retire it altogether"), when
# interview takes went from 2 to 3 minutes (some-ui#3289): a full interview
# week is 10 takes, up to 30 minutes of takes plus its own scenes, about 45.
# So both essays share one cap, 60 minutes (some-ui C780, which moves C500).
# A 60-minute essay fits the other caps: a synthetic minute of busy, moving
# text, encoded as the recorder encodes (2026-11-17), is about 4.7 MB
# finished (CRF 18, AAC 160k: about 280 MB for 60 minutes, under
# MAX_VIDEO_MB) and 16.5 MB as segments (CRF 8: about 990 MB, under CACHE_MB
# and WORK_MAX_MB); its joins are JOIN_BATCH scenes at a time, so its memory
# is the batch's, not the length's.
ESSAY_MAX_VIDEO_MINUTES=60
MAX_VIDEO_MB=500
CACHE_MB=2048
WORK_MAX_MB=4096

# How often the worker looks for an episode with no video. Looking is a
# listing of three folders; what starts a render is the lane, not this.
EVERY=60
# A failed episode is tried again this long after its failure, not on every
# look: a week that fails for a reason that stays would fail every minute.
RETRY_AFTER=3600
# A refusal by the policy (#928): the recorder's own words for a video
# past its length or size (paulgsc/some-ui packages/video-render/src/policy.ts,
# `overLength`, `overSize`). The same input, recorded with the same flags by
# the same recorders and the same speaker over the same brand files, is
# refused the same way, so such a failure is not tried again on
# RETRY_AFTER's clock: <status>/<key>.refused keeps what it was refused for
# (`held_by`), and it is tried again once that changes. Every other failure
# keeps RETRY_AFTER.
# The recorder says minutes to one decimal (`toFixed(1)`) and MB whole
# (`toFixed(0)`); .refused keeps exactly those, for file_host's board (#929).
REFUSED_LONG="the video would run ([0-9]+(\.[0-9])?) min, past the policy's ([0-9]+(\.[0-9])?) min"
REFUSED_LARGE="the video is ([0-9]+) MB, past the policy's ([0-9]+) MB"
# The recorder's refusal of scenes that overflow the frame (#975; some-ui
# apps/www/tests/record/voiced.ts, `recordStills`): this line, then each
# scene's id on a line of its own. Whether a scene fits is the essay's bytes
# and the image's stage and fonts alone, so it is refused the same way on
# every retry: held as the two above are. .refused names at most
# REFUSED_SCENES of them (the failed.log has every one) and how many there
# were. The ids are read up to the first line not of SCENE_ID's shape (the
# end of the recorder's list, whatever follows it), and a refusal naming
# none is no refusal, so it keeps the hourly retry.
REFUSED_FIT="These scenes don't fit the frame, so the video would cut them off:"
SCENE_ID='^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
REFUSED_SCENES=12
# The image the recorders are (deploy/home/release.json's render pin, which
# call-render.yml hands in): a new image may say another length or size.
RENDER_IMAGE=${RENDER_IMAGE:-}
# The speaker's image (its pin, likewise): a take's length, so a video's, is
# the speaker's.
KOKORO_IMAGE=${KOKORO_IMAGE:-}
# home's brand files, the music bed a series names (COMMON's --brand): a
# video's sound, so its size.
BRAND_DIR=/brand
# This worker's own text: it is mounted, not in the image, and how it calls
# the recorders (`record`) decides a video as the image does. Taken once: a
# changed script runs only once the container restarts.
SCRIPT_SUM=$(sha256sum < "${BASH_SOURCE[0]}" | cut -d' ' -f1)
# How often the worker reads the lane's decision while a job waits or runs.
# The lane decides every 5 s (`call_weeks::lane::TICK`).
LANE_POLL=2
# A decision older than this is stop (`call_weeks::lane::STALE_AFTER`).
STALE_AFTER=30
# How often, at most, the worker stamps <lane>/job.alive while a job renders
# or is paused, its sign of life (#739). The Renders read takes a stamp older
# than `feeds::renders::ALIVE_STALE` (60 s) as a worker gone.
ALIVE_EVERY=10
# How long a killed recorder is given to go before the worker goes on.
KILL_WAIT=10
# A recorder that fails this soon after a pause or a resume is taken to have
# failed of the pause (a page's timeout runs on while it is frozen), and runs
# again, at most PAUSE_FAILS times a job; later, or more often, it failed.
PAUSE_GRACE=300
PAUSE_FAILS=3
# A job left by this many restarts it did not ask for is taken as failed.
# A stop asked for (a deploy, a reboot: TERM) is no such restart.
RESTARTS=3
# The render container's own cgroup, whose memory.peak is the render's.
CGROUP=/sys/fs/cgroup
# What the render takes beyond its own container, which its cgroup doesn't
# see: the speaker's room above what it holds between renders (its 3 GiB cap
# less the 1.25 GB it keeps, #691), as file_host's declared peak counts it
# (`call_weeks::lane::DECLARED_PEAK_BYTES`). The stage the recorder serves
# itself is inside this container, so its cgroup sees it (#718).
BESIDE_BYTES=$(( 3 * 1024 * 1024 * 1024 - 1250 * 1000 * 1000 ))
# Where the folders below are mounted: the container's root. Only
# render-calls.test.sh moves it, to a temporary folder.
ROOT=
# tsx (a root devDependency of some-ui), not through pnpm: Corepack would fetch
# pnpm at run time, and nothing here reaches the internet.
APP_DIR=/opt/some-ui/apps/www
TSX=/opt/some-ui/node_modules/.bin/tsx
# The renderer's own command line, for how many parts a week of the call is
# (`render call-parts`, paulgsc/some-ui#2918), run with the root's tsx as the
# recorders are: in the published render image beside them, at their commit.
RENDER_DIR=/opt/some-ui/packages/video-render
# The motion, for a clip's walk, with its own tsx: in the published render
# image beside the recorders, built at their commit (#718, some-ui#2866).
MOTION_DIR=/opt/some-ui/packages/video-motion
MOTION_TSX=$MOTION_DIR/node_modules/.bin/tsx
# What both recorders take. /opt/stage is the stage the image carries, built
# at the recorders' own commit, which each recorder serves to itself (#718).
# /brand is home's folder of the owner's files a series names (its music
# bed), read-only (call-render.yml); a series that names none never reads it.
# --max-video-minutes is the job's (`max_minutes`), set by `use_job`.
COMMON=()

# The longest video `job` may make, in minutes.
max_minutes() {
  case $1 in
    rust-essay | interview-essay) echo "$ESSAY_MAX_VIDEO_MINUTES" ;;
    *) echo "$MAX_VIDEO_MINUTES" ;;
  esac
}

JOBS=(call playoff-delta preview rust-essay clip-walk cards-essay interview-essay report)

# The templates a preview may be of (paulgsc/some-ui
# packages/video-render/src/templates.ts, TEMPLATE_IDS; file_host's
# `previews::TEMPLATES`).
PREVIEW_ID='^(conference-call|engagement-report|playoff-delta-legend|playoff-delta-week|rust-essay)-[0-9]{8}T[0-9]{6}Z$'

# A clip's walk: <clip>-<side>, the clip's id plain (file_host's
# `clips::pass`), and the app it walks, plain too.
WALK_KEY='^[a-z0-9]+(-[a-z0-9]+)*-(before|after)$'
WALK_APP='^[a-z0-9]+(-[a-z0-9]+)*$'

# A cards essay: a slug of plain words, then its stamp's day (file_host's
# `cards_essays::pass::is_key`, some-ui's CARDS_KEY).
CARDS_KEY='^[a-z0-9]+(-[a-z0-9]+)*-[0-9]{4}-[0-9]{2}-[0-9]{2}$'

# A week's engagement report: its ISO week (file_host's `IsoWeek`).
REPORT_KEY='^[0-9]{4}-W[0-9]{2}$'

# The job's folders and video's name, as globals: INPUT, RENDERS, STATUS,
# WORK, VIDEO. The work folder is on the same drive as the videos, not the
# RAM disk: what a render writes on /tmp counts against the container's
# memory cap, beside ffmpeg, and moving a finished episode into RENDERS is
# one rename on the same filesystem.
use_job() {
  case $1 in
    call)
      INPUT=$ROOT/weeks RENDERS=$ROOT/renders/call STATUS=$ROOT/renders/call-worker
      WORK=$ROOT/renders/call-work VIDEO=call.mp4 ;;
    playoff-delta)
      INPUT=$ROOT/episodes RENDERS=$ROOT/renders/playoff-delta STATUS=$ROOT/renders/playoff-delta-worker
      WORK=$ROOT/renders/playoff-delta-work VIDEO=episode.mp4 ;;
    preview)
      INPUT=$ROOT/previews RENDERS=$ROOT/renders/preview STATUS=$ROOT/renders/preview-worker
      WORK=$ROOT/renders/preview-work VIDEO=preview.mp4 ;;
    rust-essay)
      INPUT=$ROOT/essays RENDERS=$ROOT/renders/rust-essay STATUS=$ROOT/renders/rust-essay-worker
      WORK=$ROOT/renders/rust-essay-work VIDEO=essay.mp4 ;;
    clip-walk)
      INPUT=$ROOT/clip-walks RENDERS=$ROOT/renders/clip-walk STATUS=$ROOT/renders/clip-walk-worker
      WORK=$ROOT/renders/clip-walk-work VIDEO=walk.json ;;
    cards-essay)
      INPUT=$ROOT/cards-essays RENDERS=$ROOT/renders/cards-essay STATUS=$ROOT/renders/cards-essay-worker
      WORK=$ROOT/renders/cards-essay-work VIDEO=cards.mp4 ;;
    interview-essay)
      INPUT=$ROOT/interview-essays RENDERS=$ROOT/renders/interview-essay STATUS=$ROOT/renders/interview-essay-worker
      WORK=$ROOT/renders/interview-essay-work VIDEO=essay.mp4 ;;
    report)
      INPUT=$ROOT/report-weeks RENDERS=$ROOT/renders/report STATUS=$ROOT/renders/report-worker
      WORK=$ROOT/renders/report-work VIDEO=report.mp4 ;;
  esac
  # The lane's folder, whichever job runs: file_host reads the job and its
  # peak there, and writes its decision there.
  LANE=$ROOT/renders/call-worker
  COMMON=(--dist /opt/stage --voices kokoro --speaker http://kokoro:8880 --brand /brand
    --max-video-minutes "$(max_minutes "$1")" --max-video-mb "$MAX_VIDEO_MB" --cache-mb "$CACHE_MB")
}

# Each episode the job has been given, one key a line, oldest first: a key
# that is not the job's shape is never read, so it is one plain path segment.
episodes() {
  local entry key
  case $1 in
    call)
      # Each week, then its later parts once its first job has said how
      # many. Within a pass each week's parts are listed in order; a week's
      # later parts first appear in the pass after its first job ran.
      local part of
      for entry in "$INPUT"/*.json; do
        [ -e "$entry" ] || continue
        key=$(basename "$entry" .json)
        [[ "$key" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || continue
        echo "$key"
        of=$(parts_known "$key")
        for (( part = 2; part <= of; part++ )); do
          echo "$key-p$part"
        done
      done ;;
    playoff-delta)
      for entry in "$INPUT"/*/; do
        [ -d "$entry" ] || continue
        key=$(basename "$entry")
        [[ "$key" =~ ^[0-9]{4}-[A-Z]{2,3}-[0-9]{2}$ ]] || continue
        [ -e "$entry/fill.json" ] && [ -e "$entry/season.json" ] && echo "$key"
      done ;;
    preview)
      for entry in "$INPUT"/*.json; do
        [ -e "$entry" ] || continue
        key=$(basename "$entry" .json)
        [[ "$key" =~ $PREVIEW_ID ]] && echo "$key"
      done ;;
    rust-essay | interview-essay)
      for entry in "$INPUT"/*/; do
        [ -d "$entry" ] || continue
        key=$(basename "$entry")
        [[ "$key" =~ ^[0-9]{4}-w[0-9]{2}$ ]] || continue
        [ -e "$entry/essay.json" ] && echo "$key"
      done ;;
    clip-walk)
      for entry in "$INPUT"/*/; do
        [ -d "$entry" ] || continue
        key=$(basename "$entry")
        [[ "$key" =~ $WALK_KEY ]] || continue
        [ -e "$entry/sample.json" ] && [ -e "$entry/app" ] && echo "$key"
      done ;;
    cards-essay)
      # By its day, not its name: a slug-first key's name order isn't its
      # day's (nixos-upgrade-2026-11-14 before zfs-2026-11-02).
      for entry in "$INPUT"/*.json; do
        [ -e "$entry" ] || continue
        key=$(basename "$entry" .json)
        [[ "$key" =~ $CARDS_KEY ]] && echo "${key: -10} $key"
      done | LC_ALL=C sort | cut -d' ' -f2 ;;
    report)
      # A week's name is its order: oldest first.
      for entry in "$INPUT"/*.json; do
        [ -e "$entry" ] || continue
        key=$(basename "$entry" .json)
        [[ "$key" =~ $REPORT_KEY ]] && echo "$key"
      done | LC_ALL=C sort ;;
  esac
}

# A call's part keys (paulgsc/some-ui#2918): <from> is the week's first
# part, as a week of one video always was, and <from>-p<k> its k-th.
week_of() {
  echo "${1%-p*}"
}

part_of() {
  case $1 in
    *-p*) echo "${1##*-p}" ;;
    *) echo 1 ;;
  esac
}

# How many parts the week of `from` is, as its first job found them
# (<renders>/<from>.parts); 1 while none is kept, as for a week rendered
# before parts or not looked at yet.
parts_known() {
  local of=
  [ -r "$RENDERS/$1.parts" ] && read -r of < "$RENDERS/$1.parts"
  if [[ "$of" =~ ^[1-9][0-9]?$ ]]; then echo "$of"; else echo 1; fi
}

# Whether any part of the week of `from` has its video, or had it: a part
# let go (file_host's <key>.let-go beside the week's file) was made.
week_made() {
  local made
  for made in "$RENDERS/$1/$VIDEO" "$RENDERS/$1"-p*/"$VIDEO" "$INPUT/$1.let-go" "$INPUT/$1"-p*.let-go; do
    [ -e "$made" ] && return 0
  done
  return 1
}

# The week file the call of `from` is recorded from: the revision this
# worker took in (`promote`), or the week as file_host first gave it. A week
# that took one in and has lost its copy names none: falling back to the
# first would record its parts from two revisions. What the worker took in
# of a week stays, a few bytes, as long as anything of the week might.
week_file() {
  if [ -e "$RENDERS/.taken/$1.json" ]; then
    echo "$RENDERS/.taken/$1.json"
  elif [ -e "$RENDERS/$1.served" ]; then
    echo "the week of $1 took in a revision ($RENDERS/$1.served) whose copy is gone; it is not recorded from another" >&2
    return 1
  else
    echo "$INPUT/$1.json"
  fi
}

# A newer revision of the week of `from` (paulgsc/server#879, C380). file_host
# keeps the week's first revision as <from>.json and gives the newest as
# <from>.next beside it, never rewriting a file it gave; this worker alone
# takes it in, at the start of a job of that week, one job at a time, so no
# revision ever changes under a count or a render. It takes it only while no
# part of the week is made: it copies it to <renders>/.taken/<from>.json,
# which the week is recorded from from then on, drops the week's count of
# parts and its work, and records the revision's SHA-256 in
# <renders>/<from>.served. Once a part is made, the week is frozen at the
# revision it was made from (#874: a count is never dropped once a part is
# made), and that is said once, in <status>/<from>.frozen.log.
promote() {
  local from=$1 next="$INPUT/$1.next" sha served=
  [ -e "$next" ] || return 0
  sha=$(sha256sum < "$next" | cut -d' ' -f1) || return 1
  [ -r "$RENDERS/$from.served" ] && read -r served < "$RENDERS/$from.served"
  [ "$sha" != "$served" ] || return 0
  if week_made "$from"; then
    if [ ! -e "$STATUS/$from.frozen.log" ]; then
      mkdir -p "$STATUS" &&
        echo "a newer revision of the week of $from was given after a part of it was made; the week stays at the revision its parts were made from" |
        tee -a "$STATUS/$from.frozen.log"
    fi
    return 0
  fi
  mkdir -p "$RENDERS/.taken" &&
    cp "$next" "$RENDERS/.taken/.$from.json" &&
    [ "$(sha256sum < "$RENDERS/.taken/.$from.json" | cut -d' ' -f1)" = "$sha" ] &&
    rm -f "${RENDERS:?}/$from.parts" &&
    rm -rf "${WORK:?}/$from" &&
    mv -f "$RENDERS/.taken/.$from.json" "$RENDERS/.taken/$from.json" &&
    put_in "$RENDERS" "$from.served" "$sha" || return 1
  # An attempt that failed was of the old revision: the new one is tried now.
  rm -f "${STATUS:?}/$from.failed.log" "${STATUS:?}/$from"-p*.failed.log \
    "${STATUS:?}/$from.refused" "${STATUS:?}/$from"-p*.refused
  echo "the week of $from takes the newer revision file_host gave; its parts are asked again"
}

# How many parts the week of `from` is: kept, or asked of the recorder's
# `render call-parts` (its JSON, `{week, parts: [{part, of, ...}]}`) and
# kept, so a week is asked once. Run inside the week's first job, which the
# lane admitted. Refused (a week that can't make a call) or unreadable, the
# job fails, as a recorder that refuses the week does.
call_parts() {
  local from=$1 said text parts ofs n=0 part of=
  if [ -r "$RENDERS/$from.parts" ]; then
    parts_known "$from"
    return 0
  fi
  local file
  file=$(week_file "$from") || return 1
  if ! said=$(cd "$RENDER_DIR" && "$TSX" src/cli.ts call-parts "$file"); then
    printf '%s\n' "$said" >&2
    echo "render call-parts refused the week of $from" >&2
    return 1
  fi
  # Whatever its spacing: every part's number, and every `of`.
  text=$(printf '%s' "$said" | tr -d ' \t\r\n')
  parts=$(grep -o '"part":[0-9]*' <<< "$text" | cut -d: -f2)
  ofs=$(grep -o '"of":[0-9]*' <<< "$text" | cut -d: -f2 | sort -u)
  for part in $parts; do
    n=$(( n + 1 ))
    [ "$part" = "$n" ] || n=-1000
  done
  [[ "$ofs" =~ ^[1-9][0-9]?$ ]] && of=$ofs
  if [ -z "$of" ] || [ "$n" != "$of" ]; then
    printf '%s\n' "$said" >&2
    echo "render call-parts said no parts this worker reads for the week of $from" >&2
    return 1
  fi
  mkdir -p "$RENDERS" && put_in "$RENDERS" "$from.parts" "$of" || return 1
  echo "the week of $from is $of part(s)" >&2
  echo "$of"
}

# Whether the job still holds `key` (file_host takes an episode out of its
# input once the owner lets it go; a part of a call's week let go while
# another still has its video is marked <key>.let-go).
holds() {
  case $1 in
    call) [ -e "$INPUT/$(week_of "$2").json" ] && [ ! -e "$INPUT/$2.let-go" ] ;;
    playoff-delta) [ -d "$INPUT/$2" ] ;;
    preview) [ -e "$INPUT/$2.json" ] ;;
    rust-essay | interview-essay) [ -e "$INPUT/$2/essay.json" ] ;;
    clip-walk) [ -e "$INPUT/$2/sample.json" ] ;;
    cards-essay | report) [ -e "$INPUT/$2.json" ] ;;
  esac
}

# Records and renders `key` into $WORK.
record() {
  local job=$1 key=$2
  case $job in
    call)
      # How many parts first, then this one. A week of one part is
      # recorded as it always was, with no --part.
      local from part of file
      from=$(week_of "$key") part=$(part_of "$key")
      file=$(week_file "$from") || return 1
      of=$(call_parts "$from") || return 2
      if [ "$part" -gt "$of" ]; then
        echo "the week of $from is $of part(s): it has no part $part"
        return 2
      fi
      if [ "$of" -eq 1 ]; then
        (cd "$APP_DIR" && "$TSX" tests/record/call.ts "${COMMON[@]}" \
          --week-file "$file" --out "$WORK")
      else
        (cd "$APP_DIR" && "$TSX" tests/record/call.ts "${COMMON[@]}" \
          --week-file "$file" --part "$part" --out "$WORK")
      fi ;;
    playoff-delta)
      (cd "$APP_DIR" && "$TSX" tests/record/playoff-delta.ts "${COMMON[@]}" \
        --season-file "$INPUT/$key/season.json" --narration "$INPUT/$key/fill.json" \
        --week "$((10#${key##*-}))" --out "$WORK/$key") ;;
    preview)
      # The template is the id less its stamp.
      (cd "$APP_DIR" && "$TSX" tests/record/preview.ts "${COMMON[@]}" \
        --template "${key%-*}" --out "$WORK/$key") ;;
    rust-essay)
      # The recorder names the finished folder for the essay's week in
      # --out. Each take it plays is <ticket>.<webm|ogg|m4a> in --takes.
      (cd "$APP_DIR" && "$TSX" tests/record/essay.ts "${COMMON[@]}" \
        --essay-file "$INPUT/$key/essay.json" --takes "$INPUT/$key/takes" --out "$WORK") ;;
    interview-essay)
      # The Rust Rounds essay's recorder, which reads the essay's kind from
      # its schema; its own --out, so its work and renders are its own. Each
      # take it plays is <question>.<ext> or read.<card>.<ext> in --takes.
      (cd "$APP_DIR" && "$TSX" tests/record/essay.ts "${COMMON[@]}" \
        --essay-file "$INPUT/$key/essay.json" --takes "$INPUT/$key/takes" --out "$WORK") ;;
    clip-walk)
      # The app is file_host's word, read here as one plain name or refused.
      local app=
      read -r app < "$INPUT/$key/app"
      if ! [[ "$app" =~ $WALK_APP ]]; then
        echo "the walk of $key names no app this worker runs"
        return 2
      fi
      (cd "$MOTION_DIR" && "$MOTION_TSX" src/cli.ts walk "$app" \
        --sample "$INPUT/$key/sample.json" --out "$WORK/$key") ;;
    cards-essay)
      # Said by home's speaker, from COMMON's --voices and --speaker, as the
      # call's recorder is (EXPLAIN6, #878); a silent (v1) essay says
      # nothing. It names the finished folder for the essay's key in --out,
      # with description.txt (its chapters) beside the video when it has
      # enough of them (`record:cards`).
      (cd "$APP_DIR" && "$TSX" tests/record/cards.ts "${COMMON[@]}" \
        --cut-list-file "$INPUT/$key.json" --out "$WORK") ;;
    report)
      # The report is written from home's counts by the recorder itself
      # (`reportWeek`); it names the finished folder for the week in --out,
      # with report.json (the report as drawn) and description.txt (its
      # chapters) beside the video (`record:report`, paulgsc/some-ui#2915).
      (cd "$APP_DIR" && "$TSX" tests/record/report.ts "${COMMON[@]}" \
        --report-file "$INPUT/$key.json" --out "$WORK") ;;
  esac
}

# Which revision of `key` the job was given: an essay's `revision` file
# (file_host's `rust_essays`, C72), empty for every other job.
revision() {
  [ "$1" = rust-essay ] || [ "$1" = interview-essay ] || return 0
  cat "$INPUT/$2/revision" 2> /dev/null
}

# Where `record` leaves `key`'s finished folder: the call's recorder names it
# for the week's first day in --out, and a part of a series part-<k> in
# that, Playoff Delta's <season>-week-NN in it, the essay's for its week in
# --out.
finished() {
  case $1 in
    call)
      if [ "$(parts_known "$(week_of "$2")")" -gt 1 ]; then
        echo "$WORK/$(week_of "$2")/part-$(part_of "$2")"
      else
        echo "$WORK/$2"
      fi ;;
    playoff-delta) echo "$WORK/$2/${2%%-*}-week-${2##*-}" ;;
    preview) echo "$WORK/$2" ;;
    rust-essay) echo "$WORK/$2" ;;
    clip-walk) echo "$WORK/$2" ;;
    cards-essay) echo "$WORK/$2" ;;
    interview-essay) echo "$WORK/$2" ;;
    report) echo "$WORK/$2" ;;
  esac
}

# Written whole, so file_host never reads half a file: `put_in <dir> <name>
# <text>`, and `put <name> <text>` in the job's status folder.
put_in() {
  printf '%s\n' "$3" > "$1/.$2" && mv "$1/.$2" "$1/$2"
}

put() {
  put_in "$STATUS" "$1" "$2"
}

# Stamps last-pass, the job's and the lane folder's, at most every EVERY
# seconds while a job waits for room or is paused, so a worker the lane holds
# back for hours is not a worker that stopped (CallRenderWorkerStale): that
# is LaneStuck's. Never while it renders: a render that hangs still goes
# stale.
heartbeat() {
  local now
  now=$(date +%s)
  [ $(( now - ${BEAT:-0} )) -ge "$EVERY" ] || return 0
  BEAT=$now
  put last-pass "$now"
  [ "$STATUS" = "$LANE" ] || put_in "$LANE" last-pass "$now"
}

# The worker's sign of life for Docker's healthcheck (#794): the time, in
# <TMP_BASE>/worker.beat (/tmp/worker.beat, the path call-render.yml's
# healthcheck reads: the container's tmpfs, so a restart starts with no
# beat), at most every ALIVE_EVERY seconds, from every
# loop the worker turns (a pass, a wait for room, a render), so it is never
# stale while the worker runs, whatever the lane decides or however long a
# render takes. Not last-pass, which a render leaves to go stale on purpose,
# and not job.alive, which only a job writes. It says only that the loop
# turns: a recorder that hangs while the worker lives is still
# CallRenderOverdue's. call-render.yml's healthcheck takes a beat older than
# 300 s as a worker gone: the longest silence is a pass's EVERY sleep and the
# listing and tidying around it, since a wait or a render beats every
# LANE_POLL.
beat() {
  local now
  now=$(date +%s)
  [ $(( now - ${BEAT_AT:-0} )) -ge "$ALIVE_EVERY" ] || return 0
  BEAT_AT=$now
  put_in "$TMP_BASE" worker.beat "$now" || true
}

# Now, in Unix seconds, against which a decision's age is read.
# render-calls.test.sh puts its fake lane's own clock here, so a fake lane
# a busy machine runs late is late, not stale.
lane_now() {
  date +%s
}

# The lane's decision now, in LANE_VERB (run, pause or stop), LANE_WHY and
# LANE_AT (Unix seconds; 0 when there is none). Missing, unreadable, stale
# either way, or a verb it doesn't know: stop.
read_lane() {
  local line verb='' why='' at='' now age
  LANE_VERB=stop LANE_WHY=unknown LANE_AT=0
  if [ ! -r "$LANE/lane" ]; then
    LANE_WHY=missing
    return 0
  fi
  while IFS= read -r line; do
    case $line in
      verb=*) verb=${line#verb=} ;;
      why=*) why=${line#why=} ;;
      at=*) at=${line#at=} ;;
    esac
  done < "$LANE/lane"
  [[ "$at" =~ ^[0-9]{1,12}$ ]] || return 0
  now=$(lane_now)
  age=$(( now - at ))
  if [ "${age#-}" -gt "$STALE_AFTER" ]; then
    LANE_WHY=stale
    return 0
  fi
  LANE_AT=$at
  LANE_WHY=$why
  case $verb in
    run | pause) LANE_VERB=$verb ;;
  esac
}

# The job job.state names, left by a restart, in LEFT_ID, LEFT_KIND,
# LEFT_KEY, LEFT_SINCE, LEFT_YIELDS, LEFT_RAN and LEFT_RESTARTS; false when
# there is none, or it can't be read, or <lane>/job names another.
left_job() {
  local line id=
  LEFT_ID='' LEFT_KIND='' LEFT_KEY='' LEFT_SINCE='' LEFT_YIELDS='' LEFT_RAN=0 LEFT_RESTARTS=0
  [ -r "$LANE/job.state" ] && [ -r "$LANE/job" ] || return 1
  # A last line with no newline is read too.
  while IFS= read -r line || [ -n "$line" ]; do
    case $line in
      id=*) LEFT_ID=${line#id=} ;;
      kind=*) LEFT_KIND=${line#kind=} ;;
      key=*) LEFT_KEY=${line#key=} ;;
      since=*) LEFT_SINCE=${line#since=} ;;
      yields=*) LEFT_YIELDS=${line#yields=} ;;
      ran=*) LEFT_RAN=${line#ran=} ;;
      restarts=*) LEFT_RESTARTS=${line#restarts=} ;;
    esac
  done < "$LANE/job.state"
  read -r id < "$LANE/job"
  fits "$LEFT_ID" && [ "$id" = "$LEFT_ID" ] || return 1
  [[ "$LEFT_SINCE" =~ ^[0-9]{1,12}$ ]] && [[ "$LEFT_YIELDS" =~ ^[0-9]{1,9}$ ]] || return 1
  [[ "$LEFT_RAN" =~ ^[0-9]{1,9}$ ]] && [[ "$LEFT_RESTARTS" =~ ^[0-9]{1,9}$ ]] || return 1
  [[ "$LEFT_KEY" =~ ^[0-9A-Za-z-]+$ ]] || return 1
  case $LEFT_KIND in
    call | playoff-delta | preview | rust-essay | clip-walk | cards-essay | interview-essay | report) return 0 ;;
    *) return 1 ;;
  esac
}

# Whether `n` is a whole number bash's arithmetic holds (under 2^63).
fits() {
  # Nineteen digits compare as text, which is their order as numbers.
  # shellcheck disable=SC2071
  [[ "$1" =~ ^[0-9]{1,19}$ ]] && { [ "${#1}" -lt 19 ] || [[ "$1" < 9223372036854775807 ]]; }
}

# A job id never given before, in JOB_ID: the time in nanoseconds, or one
# past the last id given when the clock is behind it. job-seq keeps the last.
new_job_id() {
  local last=0 id
  [ -r "$LANE/job-seq" ] && read -r last < "$LANE/job-seq"
  fits "$last" || last=0
  id=$(date +%s%N)
  [ "$id" -gt "$last" ] || id=$(( last + 1 ))
  put_in "$LANE" job-seq "$id" || return 1
  JOB_ID=$id
}

# Where the job is, for the phone's Renders page (SFEED13), written whole:
# `waiting` for room (before its first run, or after it gave way),
# `rendering`, or `paused` (frozen while home's interactive work runs);
# since when; and `ran`, the seconds it has rendered before that, which with
# `took` (below) makes the page's rough bar. `restarts` counts the restarts
# it was left by that the worker was not asked for (RESTARTS).
job_state() {
  local now
  now=$(date +%s)
  if [ "${JOB_STATE:-}" = rendering ]; then
    JOB_RAN=$(( JOB_RAN + now - JOB_STATE_SINCE ))
  fi
  JOB_STATE=$1 JOB_STATE_SINCE=$now
  put_in "$LANE" job.state "$(printf 'id=%s\nkind=%s\nkey=%s\nsince=%s\nstate=%s\nstate_since=%s\nyields=%s\nran=%s\nrestarts=%s' \
    "$JOB_ID" "$JOB_KIND" "$JOB_KEY" "$JOB_SINCE" "$1" "$now" "$JOB_YIELDS" "$JOB_RAN" "$JOB_RESTARTS")"
}

# The job for `job`'s `key`: the one a restart left for it, with its id, or
# a new one. A restart left it without a TERM (no job.termed) is one more of
# its `restarts`.
begin_job() {
  JOB_KIND=$1 JOB_KEY=$2 JOB_YIELDS=0 JOB_RAN=0 JOB_RESTARTS=0 JOB_STATE=''
  if left_job && [ "$LEFT_KIND" = "$1" ] && [ "$LEFT_KEY" = "$2" ]; then
    JOB_ID=$LEFT_ID JOB_SINCE=$LEFT_SINCE JOB_YIELDS=$LEFT_YIELDS JOB_RAN=$LEFT_RAN JOB_RESTARTS=$LEFT_RESTARTS
    [ -e "$LANE/job.termed" ] || JOB_RESTARTS=$(( JOB_RESTARTS + 1 ))
    # Left by a restart: whatever it held went with the container.
    LEFT_AFRESH=1
  else
    LEFT_AFRESH=0
    new_job_id || return 1
    JOB_SINCE=$(date +%s)
  fi
  rm -f "${LANE:?}/job.termed"
  put_in "$LANE" job "$JOB_ID" && job_state waiting
}

# The job is over: rendered, failed or let go.
end_job() {
  rm -f "${LANE:?}/job" "${LANE:?}/job.state" "${LANE:?}/job.progress" "${LANE:?}/job.termed" "${LANE:?}/job.alive"
}

# The worker's sign of life while a job renders or is paused (#739): its id
# and the time, in <lane>/job.alive, at most every ALIVE_EVERY seconds. It
# is the worker's own loop that stamps it, never the recorder's output: a
# scene or the assembly may say nothing for minutes, but the loop looks at
# the lane every LANE_POLL seconds, so a stamp gone stale is a worker gone
# (its container killed or stopped mid-render), and the Renders read stops
# showing that job as rendering. A recorder that hangs while the worker
# lives is still CallRenderOverdue's.
alive() {
  local now
  now=$(date +%s)
  [ $(( now - ${ALIVE:-0} )) -ge "$ALIVE_EVERY" ] || return 0
  ALIVE=$now
  put_in "$LANE" job.alive "$(printf 'id=%s\nat=%s' "$JOB_ID" "$now")" || true
}

# How far the recorder is, for the phone's Renders page (C138), kept whole
# in <lane>/job.progress as it says so: its station (`recording`, then
# `assembling` once it puts the video together), the scenes it has recorded,
# how many there are once it says, and since when it is at its station. It
# reads only the recorder's own count lines (`scenes <n>`, `recorded <file>`,
# `assembling`) and keeps a count, never a file's name or a line's words;
# every line goes on to the worker's output as before. A recorder that says
# none of them (a clip's walk) leaves no progress.
progress() {
  local line scene=0 scenes='' station=recording since
  since=$(date +%s)
  while IFS= read -r line; do
    printf '%s\n' "$line"
    case $line in
      'scenes '*)
        [[ "${line#scenes }" =~ ^[0-9]{1,6}$ ]] || continue
        scenes=${line#scenes } ;;
      'recorded '*) scene=$(( scene + 1 )) ;;
      assembling)
        [ "$station" = assembling ] && continue
        station=assembling since=$(date +%s) ;;
      *) continue ;;
    esac
    put_in "$LANE" job.progress "$(printf 'id=%s\nstation=%s\nscene=%s\nscenes=%s\nsince=%s' \
      "$JOB_ID" "$station" "$scene" "$scenes" "$since")" || true
  done
}

# The render's measured peak, for the lane's next admission: the container's
# memory.peak, its most since it started, so never less than this render's,
# plus what the speaker and the stage take beside it. Unreadable (a kernel
# before 5.19), the lane keeps the declared peak.
keep_peak() {
  local peak=
  if read -r peak 2> /dev/null < "$CGROUP/memory.peak" && [[ "$peak" =~ ^[0-9]{1,18}$ ]] && [ "$peak" -gt 0 ]; then
    put_in "$LANE" peak "$(( peak + BESIDE_BYTES ))"
  else
    echo "the render's memory.peak could not be read; the lane keeps its declared peak"
  fi
}

# `root` and every process under it, one pid a line, from /proc. Not its
# process group: Playwright starts Chromium in a session of its own.
tree_of() {
  local stat rest pid ppid kid
  local -a queue
  local -A kids=()
  for stat in /proc/[0-9]*/stat; do
    read -r rest 2> /dev/null < "$stat" || continue
    pid=${rest%% *}
    # The name, in parentheses, may hold spaces: the fields after it.
    rest=${rest##*) }
    read -r _ ppid _ <<< "$rest"
    kids[$ppid]+=" $pid"
  done
  queue=("$1")
  while [ ${#queue[@]} -gt 0 ]; do
    pid=${queue[0]}
    queue=("${queue[@]:1}")
    echo "$pid"
    for kid in ${kids[$pid]:-}; do
      queue+=("$kid")
    done
  done
}

# Freezes `root` and everything under it. A frozen process starts nothing,
# so a look after the freeze finds only what started during it; it looks
# until it finds nothing new.
freeze_tree() {
  local frozen=" " pid new _round
  for _round in 1 2 3 4 5; do
    new=0
    for pid in $(tree_of "$1"); do
      case $frozen in *" $pid "*) continue ;; esac
      kill -STOP "$pid" 2> /dev/null
      frozen+="$pid "
      new=1
    done
    [ "$new" -eq 1 ] || break
  done
}

thaw_tree() {
  local pid
  for pid in $(tree_of "$1"); do
    kill -CONT "$pid" 2> /dev/null
  done
}

# Kills `root` and everything under it, frozen first so none escapes by
# outliving its parent, and gives `root` KILL_WAIT seconds to go.
kill_tree() {
  local pids waited=0
  freeze_tree "$1"
  pids=$(tree_of "$1")
  # shellcheck disable=SC2086 # one pid a word
  kill -KILL $pids 2> /dev/null
  while kill -0 "$1" 2> /dev/null && [ "$waited" -lt "$KILL_WAIT" ]; do
    sleep 1
    waited=$(( waited + 1 ))
  done
}

# Says `line` on stdout and in the attempt's log.
say() {
  echo "$1" | tee -a "$2"
}

# Waits until the lane admits the job: a run decision written after `asked`,
# so the lane has seen the job and found room for its peak (LA2). False when
# the episode is let go meanwhile (1), or its job can't be named again (3).
#
# With `afresh`, the job holds no memory though the lane may still count it
# as running (it was killed by a restart, or failed of a pause, or its
# decision went stale): the lane checks room only for a job it has not seen
# run, so the job is first taken away until a decision without it is
# written, then named again under the same id. Its count of stops starts
# again (LA4); a stop the lane decided never takes this way.
wait_for_room() {
  local job=$1 key=$2 asked=$3 log=$4 afresh=${5:-0} said=''
  if [ "$afresh" -eq 1 ]; then
    rm -f "${LANE:?}/job"
    asked=$(date +%s)
  fi
  while true; do
    holds "$job" "$key" || return 1
    read_lane
    if [ "$afresh" -eq 1 ] && [ "$LANE_AT" -gt "$asked" ]; then
      put_in "$LANE" job "$JOB_ID" || return 3
      afresh=0
      asked=$(date +%s)
      continue
    fi
    [ "$afresh" -eq 0 ] && [ "$LANE_VERB" = run ] && [ "$LANE_AT" -gt "$asked" ] && return 0
    if [ "$said" != "$LANE_VERB/$LANE_WHY" ]; then
      said="$LANE_VERB/$LANE_WHY"
      say "the $job of $key waits for room on home (the lane: $LANE_VERB, $LANE_WHY)" "$log"
    fi
    heartbeat
    beat
    sleep "$LANE_POLL"
  done
}

# One run of the recorder over the job's work, in the background, obeying
# the lane until it ends. ATTEMPT is then `done`, `failed` (with
# ATTEMPT_EXIT), or `yielded`: the lane said stop and it was killed, or it
# failed after a pause.
attempt() {
  local job=$1 key=$2 log=$3 paused=0 moved=0 status
  local tmp="$TMP_BASE/render.$JOB_ID"
  ATTEMPT='' ATTEMPT_EXIT=0 AFRESH=0
  # Its own temporary folder, gone after it, killed or not: a killed
  # Chromium leaves its profile behind, and /tmp is held in memory.
  rm -rf "${tmp:?}" && mkdir -p "$tmp"
  # Each run counts afresh: one resumed from its kept steps says each again.
  rm -f "${LANE:?}/job.progress"
  (
    # shellcheck disable=SC2030 # the recorder's alone
    export TMPDIR=$tmp
    record "$job" "$key" 2>&1 | tee -a "$log" | progress
    exit "${PIPESTATUS[0]}"
  ) &
  RECORDER=$!
  # The sign of life first: a stamp left by this job's last run (it waited
  # for room since) must not be read beside `rendering` as a worker gone.
  ALIVE=0 && alive
  job_state rendering
  while kill -0 "$RECORDER" 2> /dev/null; do
    sleep "$LANE_POLL"
    kill -0 "$RECORDER" 2> /dev/null || break
    alive
    beat
    [ "$paused" -eq 0 ] || heartbeat
    read_lane
    case $LANE_VERB in
      run)
        if [ "$paused" -eq 1 ]; then
          thaw_tree "$RECORDER"
          paused=0 moved=$(date +%s)
          job_state rendering
          say "resumed the $job of $key" "$log"
        fi ;;
      pause)
        if [ "$paused" -eq 0 ]; then
          freeze_tree "$RECORDER"
          paused=1 moved=$(date +%s)
          job_state paused
          say "paused the $job of $key: home's interactive work goes first (the lane: $LANE_WHY)" "$log"
        fi ;;
      *)
        kill_tree "$RECORDER"
        ATTEMPT=yielded
        # A decision gone stale is no stop the lane decided.
        [ "$LANE_AT" -gt 0 ] || AFRESH=1
        say "the $job of $key gave way (the lane: stop, $LANE_WHY); it resumes from its kept steps once there is room" "$log"
        break ;;
    esac
  done
  wait "$RECORDER"
  status=$?
  RECORDER=
  rm -rf "${tmp:?}"
  # It finished between the last look and the kill: done, not a yield.
  if [ "$status" -eq 0 ]; then
    ATTEMPT='done'
    return 0
  fi
  [ -n "$ATTEMPT" ] && return 0
  # One that died frozen (killed from outside, its memory gone) is taken the
  # same way, however long it was frozen.
  if [ "$moved" -gt 0 ] && { [ "$paused" -eq 1 ] || [ $(( $(date +%s) - moved )) -le "$PAUSE_GRACE" ]; } &&
    [ "$JOB_PAUSE_FAILS" -lt "$PAUSE_FAILS" ]; then
    ATTEMPT=yielded AFRESH=1
    JOB_PAUSE_FAILS=$(( JOB_PAUSE_FAILS + 1 ))
    say "the recorder exited $status soon after a pause; it runs again from its kept steps, not counted as a failure" "$log"
  else
    ATTEMPT=failed ATTEMPT_EXIT=$status
  fi
}

# Renders `key` of `job`: waits for room, runs, gives way and resumes as
# the lane says, until it renders (0), fails (1), or is let go (2).
render_episode() {
  local job=$1 key=$2
  local log="$STATUS/.$key.log" made asked rev
  : > "$log" || return 1
  if ! mkdir -p "$WORK" 2>> "$log"; then
    echo "the work folder $WORK could not be made" | tee -a "$log"
    return 1
  fi
  if ! begin_job "$job" "$key" 2>> "$log"; then
    echo "the job could not be written to $LANE" | tee -a "$log"
    return 1
  fi
  if [ "$JOB_RESTARTS" -ge "$RESTARTS" ]; then
    echo "the $job of $key was left by $JOB_RESTARTS restarts the worker was not asked for (its container died under it); taken as failed" | tee -a "$log"
    end_job
    return 1
  fi
  asked=$(date +%s)
  JOB_PAUSE_FAILS=0
  AFRESH=$LEFT_AFRESH
  while true; do
    wait_for_room "$job" "$key" "$asked" "$log" "$AFRESH"
    case $? in
      0) ;;
      3)
        echo "the job could not be written to $LANE" | tee -a "$log"
        end_job
        return 1 ;;
      *)
        end_job
        return 2 ;;
    esac
    rev=$(revision "$job" "$key")
    HELD=$(held_by "$job" "$key")
    # Where this attempt's lines start in the job's log (`refusal`).
    ATTEMPT_FROM=$(stat -c %s "$log" 2> /dev/null || echo 0)
    attempt "$job" "$key" "$log"
    # An essay corrected while it rendered (file_host gave a newer
    # revision, C72): what was made is of the old one, so it is never kept.
    # It runs again over the new essay, its unchanged steps from the cache.
    if [ "$(revision "$job" "$key")" != "$rev" ] && [ "$ATTEMPT" != yielded ]; then
      rm -rf "$(finished "$job" "$key")"
      say "the $job of $key was corrected while it rendered; it runs again over the newer revision" "$log"
      ATTEMPT=yielded
    fi
    case $ATTEMPT in
      done) break ;;
      failed)
        echo "the recorder exited $ATTEMPT_EXIT" | tee -a "$log"
        end_job
        return 1 ;;
    esac
    JOB_YIELDS=$(( JOB_YIELDS + 1 ))
    job_state waiting
    asked=$(date +%s)
  done
  keep_peak
  # How long it rendered, pauses and waits left out: the next job of this
  # kind's rough bar measures against it.
  job_state 'done'
  put took "$JOB_RAN"
  made=$(finished "$job" "$key")
  # The renderer's intermediates can be made again from what is kept.
  rm -rf "${made:?}/cache"
  if ! { mkdir -p "$RENDERS" && mv "$made" "$RENDERS/$key"; } 2>> "$log"; then
    echo "the finished $job of $key could not be moved into $RENDERS" | tee -a "$log"
    end_job
    return 1
  fi
  # A preview's day is counted from its video's time (file_host's
  # `previews`): now, not when an earlier attempt first wrote it.
  [ "$job" != preview ] || touch "$RENDERS/$key/$VIDEO"
  # What is left of Playoff Delta's --out folder is empty. A part of a
  # call's week leaves its week's folder to the parts still waiting in it,
  # and the last takes the empty folder with it.
  if [ "$job" = call ]; then
    rmdir "${WORK:?}/$(week_of "$key")" 2> /dev/null
  else
    rm -rf "${WORK:?}/$key"
  fi
  end_job
}

# The `cache warning:` lines of `key`'s last attempt, kept where file_host
# reads them; nothing is written when there were none.
keep_cache_warnings() {
  local key=$1 log="$STATUS/.$1.log" warned="$STATUS/.$1.cache.log"
  [ -e "$log" ] || return 0
  if grep '^cache warning: ' "$log" > "$warned"; then
    mv "$warned" "$STATUS/$key.cache.log"
  else
    rm -f "$warned"
  fi
}

# Before `current` renders: the work of an episode that is no longer waiting
# (rendered, or no longer held) goes, and past WORK_MAX_MB the oldest other
# waiting episode's work goes too, each said as a cache warning in
# work.cache.log (an episode's own warnings are rewritten after each attempt).
# A job a restart left (job.state) keeps its work too: the lane holds it,
# and it runs next.
prune_work() {
  local job=$1 current=$2 dir key held=
  if left_job && [ "$LEFT_KIND" = "$job" ]; then held=$LEFT_KEY; fi
  # A call's parts share their week's work folder, each in part-<k>.
  if [ "$job" = call ]; then
    current=$(week_of "$current")
    [ -z "$held" ] || held=$(week_of "$held")
  fi
  for dir in "$WORK"/*/; do
    [ -d "$dir" ] || continue
    key=$(basename "$dir")
    if [ "$job" = call ]; then
      prune_week "$key"
    elif [ -e "$RENDERS/$key/$VIDEO" ] || ! holds "$job" "$key"; then
      rm -rf "$dir"
    fi
  done
  while [ "$(du -sm "$WORK" 2>/dev/null | cut -f1)" -gt "$WORK_MAX_MB" ]; do
    key=$(ls -1 "$WORK" | grep -vx -e "$current" -e "${held:-$current}" | sort | head -n1)
    [ -n "$key" ] || break
    rm -rf "${WORK:?}/$key"
    echo "cache warning: removed the waiting work of $key: all of the $job's work passed the policy's $WORK_MAX_MB MB" |
      tee -a "$STATUS/work.cache.log"
    # Its last hundred lines only: a warning is news, not a record.
    tail -n 100 "$STATUS/work.cache.log" > "$STATUS/.work.cache.log" &&
      mv "$STATUS/.work.cache.log" "$STATUS/work.cache.log"
  done
}

# The call's work for the week of `from`: a part's work goes once the part
# has its video or is let go, and the week's folder once the week is no
# longer held, or once nothing of a part is left in it and its video
# (part 1's, or the week's own) is made.
prune_week() {
  local from=$1 part key
  if [ ! -e "$INPUT/$from.json" ]; then
    rm -rf "${WORK:?}/$from"
    return 0
  fi
  for part in "$WORK/$from"/part-*/; do
    [ -d "$part" ] || continue
    key=${part%/}
    key=${key##*/part-}
    [[ "$key" =~ ^[1-9][0-9]?$ ]] || continue
    if [ "$key" -eq 1 ]; then key=$from; else key=$from-p$key; fi
    if [ -e "$RENDERS/$key/$VIDEO" ] || ! holds call "$key"; then
      rm -rf "$part"
    fi
  done
  # A week first worked as one part (by a worker from before parts) and
  # found since to be more: what it left beside the parts is no part's.
  if [ "$(parts_known "$from")" -gt 1 ] && [ -d "$WORK/$from" ]; then
    find "$WORK/$from" -mindepth 1 -maxdepth 1 ! -name 'part-*' -exec rm -rf {} +
  fi
  if ! compgen -G "$WORK/$from/part-*" > /dev/null &&
    { [ -e "$RENDERS/$from/$VIDEO" ] || ! holds call "$from"; } &&
    [ "$(parts_known "$from")" -eq 1 ]; then
    rm -rf "${WORK:?}/$from"
  elif [ -d "$WORK/$from" ]; then
    rmdir "$WORK/$from" 2> /dev/null
  fi
  return 0
}

# The files `job`'s `key` is made from, one path a line (a folder stands for
# everything in it), whether or not each is there.
inputs_of() {
  local from
  case $1 in
    call)
      from=$(week_of "$2")
      printf '%s\n' "$INPUT/$from.json" "$INPUT/$2.let-go" "$RENDERS/.taken/$from.json"
      # A newer revision is taken in only while no part is made (`promote`):
      # one given for a frozen week changes nothing it is made from.
      week_made "$from" || echo "$INPUT/$from.next" ;;
    playoff-delta | rust-essay | interview-essay | clip-walk) echo "$INPUT/$2" ;;
    preview | cards-essay | report) echo "$INPUT/$2.json" ;;
  esac
}

# What a refusal of `job`'s `key` holds for, as one sum: every flag its
# recorder is handed (COMMON, the job's caps among them), the recorders' and
# the speaker's images, this worker's own text, and each of its inputs' and the brand files' name,
# size and time. Never their bytes: a held essay's takes are not read every
# pass. file_host writes an input whole and new (a revision is a new file),
# so a changed one has a new time. Called after `use_job`.
held_by() {
  local path
  {
    printf '%s\n' "${COMMON[@]}"
    printf 'image=%s\nspeaker=%s\nscript=%s\n' "$RENDER_IMAGE" "$KOKORO_IMAGE" "$SCRIPT_SUM"
    { inputs_of "$1" "$2"; echo "$BRAND_DIR"; } | while IFS= read -r path; do
      [ -e "$path" ] && find "$path" -printf '%p %s %T@\n'
    done | LC_ALL=C sort
  } | sha256sum | cut -d' ' -f1
}

# The policy's refusal in `log`, as <key>.refused's lines: its reason
# (`too-long` with the minutes and the cap, `too-large` with the MB and the
# cap, `overflow` with the scenes' ids and how many; the recorder's numbers
# and ids, never its words) and `held`, what it holds for.
# Only the last attempt's lines are read, from byte `from` of the job's log:
# an earlier attempt of the same job (one that gave way to a newer revision,
# or after a pause) refused may not be what the last one failed of. False
# when the last attempt was not refused by the policy.
refusal() {
  local log=$1 held=$2 from=${3:-0} line scenes
  line=$(tail -c +"$(( from + 1 ))" "$log" | grep -oE "$REFUSED_LONG|$REFUSED_LARGE|$REFUSED_FIT" | tail -n 1)
  if [ "$line" = "$REFUSED_FIT" ]; then
    # The ids on the lines after the attempt's last line holding it, up to
    # the first that is not one. Matched by bash, as the other refusals are,
    # never by the image's awk.
    local text listing=0
    scenes=
    while IFS= read -r text; do
      if [[ "$text" == *"$REFUSED_FIT"* ]]; then
        listing=1 scenes=
      elif [ "$listing" = 1 ] && [[ "$text" =~ $SCENE_ID ]]; then
        scenes+=$text$'\n'
      else
        listing=0
      fi
    done < <(tail -c +"$(( from + 1 ))" "$log")
    scenes=${scenes%$'\n'}
    [ -n "$scenes" ] || return 1
    printf 'reason=overflow\nscenes=%s\ncount=%s\nheld=%s\n' \
      "$(head -n "$REFUSED_SCENES" <<< "$scenes" | paste -sd,)" "$(wc -l <<< "$scenes")" "$held"
  elif [[ "$line" =~ ^$REFUSED_LONG$ ]]; then
    printf 'reason=too-long\nminutes=%s\ncap=%s\nheld=%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[3]}" "$held"
  elif [[ "$line" =~ ^$REFUSED_LARGE$ ]]; then
    printf 'reason=too-large\nmb=%s\ncap=%s\nheld=%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$held"
  else
    return 1
  fi
}

# Whether `job`'s `key` is not to be tried now: a preview that failed stays
# failed (the owner asks for another, under a new id); one the policy refused
# waits until what it was refused for changes (`held_by`), then is tried at
# once; any other failure waits RETRY_AFTER.
failed_recently() {
  local job=$1 failed="$STATUS/$2.failed.log" refused="$STATUS/$2.refused" when held=
  [ -e "$failed" ] || return 1
  [ "$job" != preview ] || return 0
  if [ -e "$refused" ]; then
    held=$(sed -n 's/^held=//p' "$refused")
    [ -n "$held" ] && [ "$held" = "$(held_by "$job" "$2")" ]
    return
  fi
  when=$(stat -c %Y "$failed") || return 1
  [ $(( $(date +%s) - when )) -lt "$RETRY_AFTER" ]
}

# `key` of `job`, unless it has its video or failed lately.
render_one() {
  local job=$1 key=$2 outcome
  [ -e "$RENDERS/$key/$VIDEO" ] && return 0
  # Let go (a part of a call's week marked so) or no longer given: no job is
  # named for it, so the lane never sees one it will not admit.
  holds "$job" "$key" || return 0
  # A newer revision of a week of the call is taken in first, outside any
  # render (`promote`); one that can't be is tried again next pass.
  if [ "$job" = call ] && ! promote "$(week_of "$key")"; then
    echo "the newer revision of the week of $(week_of "$key") could not be taken in; it is tried again next pass" >&2
    end_job
    return 0
  fi
  # A part the week no longer has, or may not: a revision taken in this pass
  # drops the count its later parts were listed from, and a part past 1 is
  # listed only from a count, so none is named a job until one is asked.
  if [ "$job" = call ] && [ "$(part_of "$key")" -gt "$(parts_known "$(week_of "$key")")" ]; then
    # A job a restart left of it is over too.
    end_job
    return 0
  fi
  if failed_recently "$job" "$key"; then
    # A job a restart left of it is over: a refusal held again (the image it
    # was released for rolled back, say) never runs it.
    if left_job && [ "$LEFT_KIND" = "$job" ] && [ "$LEFT_KEY" = "$key" ]; then end_job; fi
    return 0
  fi
  echo "rendering the $job of $key"
  mkdir -p "$WORK" && prune_work "$job" "$key"
  # What a refusal holds for is taken before each attempt (`render_episode`),
  # after its wait for room: an input that changes while one runs is then a
  # change, and the refusal is not held. None before any attempt.
  HELD= ATTEMPT_FROM=0
  render_episode "$job" "$key"
  outcome=$?
  keep_cache_warnings "$key"
  case $outcome in
    0)
      rm -f "${STATUS:?}/.$key.log" "${STATUS:?}/$key.failed.log" "${STATUS:?}/$key.refused"
      echo "rendered the $job of $key" ;;
    2)
      rm -f "${STATUS:?}/.$key.log" "${STATUS:?}/$key.refused"
      echo "the $job of $key was let go before it rendered" ;;
    *)
      # The last attempt's refusal first: a failed.log is never read beside
      # another attempt's.
      rm -f "${STATUS:?}/$key.refused"
      mv -f "$STATUS/.$key.log" "$STATUS/$key.failed.log"
      # Its time is the failure's, which RETRY_AFTER counts from.
      touch "$STATUS/$key.failed.log"
      # A refusal by the policy is kept as its reason, a preview's too: the
      # phone says why (#929).
      if [ -n "$HELD" ] && refusal "$STATUS/$key.failed.log" "$HELD" "$ATTEMPT_FROM" > "$STATUS/.$key.refused"; then
        mv -f "$STATUS/.$key.refused" "$STATUS/$key.refused"
      else
        rm -f "${STATUS:?}/.$key.refused"
      fi
      if [ "$job" = preview ]; then
        echo "the preview $key did not render; the owner asks for another" >&2
      elif [ -e "$STATUS/$key.refused" ]; then
        echo "the $job of $key was refused by the policy; it is tried again once what it was refused for changes (its input, its cap, the recorders, the speaker or the brand files)" >&2
      else
        echo "the $job of $key did not render; it is tried again in an hour" >&2
      fi ;;
  esac
}

# One pass of `job`: each of its episodes with no video, oldest first.
run_job() {
  local job=$1 key
  use_job "$job"
  mkdir -p "$STATUS" && put last-pass "$(date +%s)"
  # A refusal outlives no failed.log: file_host removes a preview's when it
  # prunes it, and a refusal is the reason of that failure alone.
  local refused
  for refused in "$STATUS"/*.refused; do
    [ -e "$refused" ] && [ ! -e "${refused%.refused}.failed.log" ] && rm -f "$refused"
  done
  for key in $(episodes "$job"); do
    render_one "$job" "$key"
  done
}

# Removes the job's attempt logs, `.<key>.log`: called only between jobs,
# when none runs, so each is left by an attempt that was killed, and would
# read to the lane as batch work in progress for good.
clear_attempt_logs() {
  local log name
  for log in "$STATUS"/.*.log; do
    [ -e "$log" ] || continue
    name=${log##*/}
    case $name in *.cache.log) continue ;; esac
    rm -f "$log"
  done
}

# The job a restart left, first: it is the one the lane was holding. One
# whose episode is gone or rendered is over.
resume_left() {
  local job
  for job in "${JOBS[@]}"; do
    use_job "$job"
    clear_attempt_logs
  done
  use_job call
  if ! left_job; then
    end_job
    return 0
  fi
  local kind=$LEFT_KIND key=$LEFT_KEY
  use_job "$kind"
  if holds "$kind" "$key" && [ ! -e "$RENDERS/$key/$VIDEO" ]; then
    mkdir -p "$STATUS" && render_one "$kind" "$key"
  else
    end_job
  fi
}

# Stopping the container stops the render; its job stays, and runs first
# when the worker starts again. job.termed says the stop was asked for, so
# the restart is not one of the job's `restarts`.
on_term() {
  [ -n "${RECORDER:-}" ] && kill_tree "$RECORDER"
  [ -n "${LANE:-}" ] && [ -e "$LANE/job" ] && touch "$LANE/job.termed"
  exit 143
}

# shellcheck disable=SC2031 # the recorder's own TMPDIR is its subshell's
TMP_BASE=${TMPDIR:-/tmp}
RECORDER=

# The worker: the job a restart left, then a pass of every job each EVERY
# seconds, for good. A failed episode is tried again on this clock alone,
# once RETRY_AFTER has passed since its failure.
serve() {
  local job
  while true; do
    beat
    resume_left
    for job in "${JOBS[@]}"; do
      run_job "$job"
    done
    sleep "$EVERY"
  done
}

# Sourced, it only defines the above: render-calls.test.sh runs one pass at a
# time over a temporary ROOT, with a fake recorder and a fake lane (CW5,
# paulgsc/some-ui#2659; L4c, #708), and `serve` itself on a test's clock.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  trap on_term TERM INT
  serve
fi
