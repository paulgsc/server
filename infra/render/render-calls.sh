#!/usr/bin/env bash
# Home's render worker (GWB10, paulgsc/some-ui#2084), with four jobs, one
# loop: the weekly call's (paulgsc/some-ui#2635, ST36), Playoff Delta's
# (FEED4, paulgsc/some-ui#2647), a template's preview (SFEED12,
# paulgsc/some-ui#2757) and a Rust Rounds essay's (RR16, paulgsc/some-ui#2691).
# infra/compose/call-render.yml runs it.
#
# Each pass, for each job, for each episode file_host has written for it with
# no video in the job's renders folder, it records the stage (`call-stage`,
# the pinned home-network app) with each scene said by home's speaker
# (`kokoro`), renders it, and keeps the result: the video, its takes and
# stills, its cut list and what it was made from. The same episode gives the
# same files.
#
# | job           | file_host writes (read-only here)      | kept in                     | video       |
# |---------------|----------------------------------------|-----------------------------|-------------|
# | call          | /weeks/<from>.json (call_weeks)        | /renders/call/<from>/       | call.mp4    |
# | playoff-delta | /episodes/<key>/{fill,season}.json     | /renders/playoff-delta/<key>/ | episode.mp4 |
# | preview       | /previews/<id>.json (previews)         | /renders/preview/<id>/      | preview.mp4 |
# | rust-essay    | /essays/<week>/essay.json (rust_essays) | /renders/rust-essay/<week>/ | essay.mp4   |
#
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
# sooner than its hold-down: home's own services start first.
set -uo pipefail

# Policy (paulgsc/some-ui#2635, the owner's caps). The recorder refuses a
# video longer than MAX_VIDEO_MINUTES (from its words, before anything is
# said) or larger than MAX_VIDEO_MB (removed once made), a failed attempt
# that names the cap; it stops keeping an episode's steps past CACHE_MB, with
# a `cache warning:` line. Each job's waiting work together is held to
# WORK_MAX_MB: past it, the oldest other episode's work is removed (its retry
# starts over).
MAX_VIDEO_MINUTES=15
MAX_VIDEO_MB=500
CACHE_MB=2048
WORK_MAX_MB=4096

# How often the worker looks for an episode with no video. Looking is a
# listing of three folders; what starts a render is the lane, not this.
EVERY=60
# A failed episode is tried again this long after its failure, not on every
# look: a week that fails for a reason that stays would fail every minute.
RETRY_AFTER=3600
# How often the worker reads the lane's decision while a job waits or runs.
# The lane decides every 5 s (`call_weeks::lane::TICK`).
LANE_POLL=2
# A decision older than this is stop (`call_weeks::lane::STALE_AFTER`).
STALE_AFTER=30
# How long a killed recorder is given to go before the worker goes on.
KILL_WAIT=10
# A recorder that fails this soon after a pause or a resume is taken to have
# failed of the pause (a page's timeout runs on while it is frozen), and runs
# again, at most PAUSE_FAILS times a job; later, or more often, it failed.
PAUSE_GRACE=300
PAUSE_FAILS=3
# The render container's own cgroup, whose memory.peak is the render's.
CGROUP=/sys/fs/cgroup
# What the render takes beyond its own container, which its cgroup doesn't
# see: the speaker's room above what it holds between renders (its 3 GiB cap
# less the 1.25 GB it keeps, #691) and the stage's 256 MiB, as file_host's
# declared peak counts them (`call_weeks::lane::DECLARED_PEAK_BYTES`).
BESIDE_BYTES=$(( 3 * 1024 * 1024 * 1024 - 1250 * 1000 * 1000 + 256 * 1024 * 1024 ))
# Where the folders below are mounted: the container's root. Only
# render-calls.test.sh moves it, to a temporary folder.
ROOT=
# tsx (a root devDependency of some-ui), not through pnpm: Corepack would fetch
# pnpm at run time, and nothing here reaches the internet.
APP_DIR=/opt/some-ui/apps/www
TSX=/opt/some-ui/node_modules/.bin/tsx
# What both recorders take. /brand is home's folder of the owner's files a
# series names (its music bed), read-only (call-render.yml); a series that
# names none never reads it.
COMMON=(--app http://call-stage --voices kokoro --speaker http://kokoro:8880 --brand /brand
  --max-video-minutes "$MAX_VIDEO_MINUTES" --max-video-mb "$MAX_VIDEO_MB" --cache-mb "$CACHE_MB")

JOBS=(call playoff-delta preview rust-essay)

# The templates a preview may be of (paulgsc/some-ui
# packages/video-render/src/templates.ts, TEMPLATE_IDS; file_host's
# `previews::TEMPLATES`).
PREVIEW_ID='^(conference-call|playoff-delta-legend|playoff-delta-week|rust-essay)-[0-9]{8}T[0-9]{6}Z$'

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
  esac
  # The lane's folder, whichever job runs: file_host reads the job and its
  # peak there, and writes its decision there.
  LANE=$ROOT/renders/call-worker
}

# Each episode the job has been given, one key a line, oldest first: a key
# that is not the job's shape is never read, so it is one plain path segment.
episodes() {
  local entry key
  case $1 in
    call)
      for entry in "$INPUT"/*.json; do
        [ -e "$entry" ] || continue
        key=$(basename "$entry" .json)
        [[ "$key" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] && echo "$key"
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
    rust-essay)
      for entry in "$INPUT"/*/; do
        [ -d "$entry" ] || continue
        key=$(basename "$entry")
        [[ "$key" =~ ^[0-9]{4}-w[0-9]{2}$ ]] || continue
        [ -e "$entry/essay.json" ] && echo "$key"
      done ;;
  esac
}

# Whether the job still holds `key` (file_host takes an episode out of its
# input once the owner lets it go).
holds() {
  case $1 in
    call) [ -e "$INPUT/$2.json" ] ;;
    playoff-delta) [ -d "$INPUT/$2" ] ;;
    preview) [ -e "$INPUT/$2.json" ] ;;
    rust-essay) [ -e "$INPUT/$2/essay.json" ] ;;
  esac
}

# Records and renders `key` into $WORK.
record() {
  local job=$1 key=$2
  case $job in
    call)
      (cd "$APP_DIR" && "$TSX" tests/record/call.ts "${COMMON[@]}" \
        --week-file "$INPUT/$key.json" --out "$WORK") ;;
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
  esac
}

# Which revision of `key` the job was given: an essay's `revision` file
# (file_host's `rust_essays`, C72), empty for every other job.
revision() {
  [ "$1" = rust-essay ] || return 0
  cat "$INPUT/$2/revision" 2> /dev/null
}

# Where `record` leaves `key`'s finished folder: the call's recorder names it
# for the week's first day in --out, Playoff Delta's <season>-week-NN in it,
# the essay's for its week in --out.
finished() {
  case $1 in
    call) echo "$WORK/$2" ;;
    playoff-delta) echo "$WORK/$2/${2%%-*}-week-${2##*-}" ;;
    preview) echo "$WORK/$2" ;;
    rust-essay) echo "$WORK/$2" ;;
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
# LEFT_KEY, LEFT_SINCE, LEFT_YIELDS and LEFT_RAN; false when there is none, or it
# can't be read, or <lane>/job names another.
left_job() {
  local line id=
  LEFT_ID='' LEFT_KIND='' LEFT_KEY='' LEFT_SINCE='' LEFT_YIELDS='' LEFT_RAN=0
  [ -r "$LANE/job.state" ] && [ -r "$LANE/job" ] || return 1
  while IFS= read -r line; do
    case $line in
      id=*) LEFT_ID=${line#id=} ;;
      kind=*) LEFT_KIND=${line#kind=} ;;
      key=*) LEFT_KEY=${line#key=} ;;
      since=*) LEFT_SINCE=${line#since=} ;;
      yields=*) LEFT_YIELDS=${line#yields=} ;;
      ran=*) LEFT_RAN=${line#ran=} ;;
    esac
  done < "$LANE/job.state"
  read -r id < "$LANE/job"
  fits "$LEFT_ID" && [ "$id" = "$LEFT_ID" ] || return 1
  [[ "$LEFT_SINCE" =~ ^[0-9]{1,12}$ ]] && [[ "$LEFT_YIELDS" =~ ^[0-9]{1,9}$ ]] || return 1
  [[ "$LEFT_RAN" =~ ^[0-9]{1,9}$ ]] || return 1
  [[ "$LEFT_KEY" =~ ^[0-9A-Za-z-]+$ ]] || return 1
  case $LEFT_KIND in
    call | playoff-delta | preview | rust-essay) return 0 ;;
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
# `took` (below) makes the page's rough bar.
job_state() {
  local now
  now=$(date +%s)
  if [ "${JOB_STATE:-}" = rendering ]; then
    JOB_RAN=$(( JOB_RAN + now - JOB_STATE_SINCE ))
  fi
  JOB_STATE=$1 JOB_STATE_SINCE=$now
  put_in "$LANE" job.state "$(printf 'id=%s\nkind=%s\nkey=%s\nsince=%s\nstate=%s\nstate_since=%s\nyields=%s\nran=%s' \
    "$JOB_ID" "$JOB_KIND" "$JOB_KEY" "$JOB_SINCE" "$1" "$now" "$JOB_YIELDS" "$JOB_RAN")"
}

# The job for `job`'s `key`: the one a restart left for it, with its id, or
# a new one.
begin_job() {
  JOB_KIND=$1 JOB_KEY=$2 JOB_YIELDS=0 JOB_RAN=0 JOB_STATE=''
  if left_job && [ "$LEFT_KIND" = "$1" ] && [ "$LEFT_KEY" = "$2" ]; then
    JOB_ID=$LEFT_ID JOB_SINCE=$LEFT_SINCE JOB_YIELDS=$LEFT_YIELDS JOB_RAN=$LEFT_RAN
    # Left by a restart: whatever it held went with the container.
    LEFT_AFRESH=1
  else
    LEFT_AFRESH=0
    new_job_id || return 1
    JOB_SINCE=$(date +%s)
  fi
  put_in "$LANE" job "$JOB_ID" && job_state waiting
}

# The job is over: rendered, failed or let go.
end_job() {
  rm -f "${LANE:?}/job" "${LANE:?}/job.state"
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
  (
    # shellcheck disable=SC2030 # the recorder's alone
    export TMPDIR=$tmp
    record "$job" "$key" 2>&1 | tee -a "$log"
    exit "${PIPESTATUS[0]}"
  ) &
  RECORDER=$!
  job_state rendering
  while kill -0 "$RECORDER" 2> /dev/null; do
    sleep "$LANE_POLL"
    kill -0 "$RECORDER" 2> /dev/null || break
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
  # What is left of Playoff Delta's --out folder is empty.
  rm -rf "${WORK:?}/$key"
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
  for dir in "$WORK"/*/; do
    [ -d "$dir" ] || continue
    key=$(basename "$dir")
    if [ -e "$RENDERS/$key/$VIDEO" ] || ! holds "$job" "$key"; then
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

# Whether `job`'s `key` failed less than RETRY_AFTER ago. A preview that failed
# stays failed: the owner asks for another, under a new id.
failed_recently() {
  local job=$1 failed="$STATUS/$2.failed.log" when
  [ -e "$failed" ] || return 1
  [ "$job" != preview ] || return 0
  when=$(stat -c %Y "$failed") || return 1
  [ $(( $(date +%s) - when )) -lt "$RETRY_AFTER" ]
}

# `key` of `job`, unless it has its video or failed lately.
render_one() {
  local job=$1 key=$2 outcome
  [ -e "$RENDERS/$key/$VIDEO" ] && return 0
  failed_recently "$job" "$key" && return 0
  echo "rendering the $job of $key"
  mkdir -p "$WORK" && prune_work "$job" "$key"
  render_episode "$job" "$key"
  outcome=$?
  keep_cache_warnings "$key"
  case $outcome in
    0)
      rm -f "${STATUS:?}/.$key.log" "${STATUS:?}/$key.failed.log"
      echo "rendered the $job of $key" ;;
    2)
      rm -f "${STATUS:?}/.$key.log"
      echo "the $job of $key was let go before it rendered" ;;
    *)
      mv -f "$STATUS/.$key.log" "$STATUS/$key.failed.log"
      # Its time is the failure's, which RETRY_AFTER counts from.
      touch "$STATUS/$key.failed.log"
      if [ "$job" = preview ]; then
        echo "the preview $key did not render; the owner asks for another" >&2
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
# when the worker starts again.
on_term() {
  [ -n "${RECORDER:-}" ] && kill_tree "$RECORDER"
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
