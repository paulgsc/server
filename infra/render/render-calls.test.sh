#!/usr/bin/env bash
# The render worker's own guarantees (CW5, CW6; paulgsc/some-ui#2659), for the
# call's job, one pass at a time: render-calls.sh is sourced over a temporary
# ROOT, and tsx is a fake recorder that takes the call's real arguments. CI
# runs it (test.yml, metric_contract_ci). Each case says the guarantee it holds:
#
# - a failed attempt leaves no folder in renders/call, keeps its work, and
#   the next pass reuses it;
# - a finished week is moved whole, without its cache, and its work removed;
# - a pass over a finished week leaves its folder untouched;
# - the all-weeks cap removes only another waiting week's work, never a video;
# - a week no longer held loses its waiting work and is not tried again;
# - `cache warning:` lines reach <week>.cache.log, a failed attempt's too;
# - the recorder is handed the render policy's caps, the pinned stage and
#   home's Kokoro speaker, and a video it refuses as past MAX_VIDEO_MB fails
#   its attempt, which is tried again only RETRY_AFTER later (#752);
# - the worker's own loop (`serve`) tries a failed week again on its own
#   clock, RETRY_AFTER after the failure, with no pass run by hand;
# - the stage and the speaker are pinned by digest, the recorders by commit,
#   and the stage's pin names the recorders' commit.
#
# And the lane's (L4c, paulgsc/server#708), with a fake lane that rewrites
# renders/call-worker/lane as file_host does, and a recorder whose steps are
# slow enough to be caught half way:
#
# - no render starts on a missing or stale decision, nor before the lane
#   has seen the job; it starts once the lane says run;
# - a pause freezes the recorder and what it started, and the render then
#   finishes with the same bytes, no failed.log, its steps made once;
# - a stop kills them, keeps the work and the job's id, and the render
#   resumes from its kept steps, with the same bytes as one never stopped;
# - an attempt that fails after a pause is run again, not failed;
# - the job, its state and its measured peak are written where the lane
#   reads them, and the job is gone once it renders, fails or is let go;
# - a job a restart left keeps its id and its work, and runs first;
# - a failed week waits RETRY_AFTER before it is tried again.
#
# And a template's preview (SFEED12, paulgsc/some-ui#2757): rendered with
# the template its id names and kept whole; a failed one never tried again;
# one a restart left resumed first.
#
# And a clip's walk (paulgsc/some-ui#2670's clips): the motion walks the app
# file_host named over its sample, kept whole in renders/clip-walk; a walk
# naming an app that is not a plain name is never run.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=render-calls.sh
source "$HERE/render-calls.sh"

SCRATCH=$(mktemp -d)
FAKE=$SCRATCH/fake
APP_DIR=$SCRATCH/app
TSX=$FAKE/tsx
MOTION_DIR=$APP_DIR
MOTION_TSX=$TSX
mkdir -p "$FAKE" "$APP_DIR"
# The worker's timing, at a test's pace; a failed week is tried again on the
# next pass unless a case says otherwise.
LANE_POLL=0.1
STALE_AFTER=2
KILL_WAIT=5
RETRY_AFTER=0
TMP_BASE=$SCRATCH/tmp
CGROUP=$SCRATCH/cgroup
mkdir -p "$CGROUP"
echo 3000000000 > "$CGROUP/memory.peak"

# Written whole, so a reader never sees a half-written file.
whole() {
  printf '%s\n' "$2" > "$1.tmp" && mv "$1.tmp" "$1"
}

# The fake lane: file_host's decision, rewritten whole every 0.1 s into the
# current ROOT's lane folder, saying $SCRATCH/verb (`stale` writes one a
# minute old; `hold` stops writing, as a file_host that stopped would).
#
# Nothing here leans on the machine's pace (#747, #745):
# - **Its own clock.** Each round it stamps $SCRATCH/tick, and the worker
#   reads a decision's age against that tick (`lane_now`, below), not
#   against the wall. A fake lane a busy runner starves is late, never
#   stale. `hold` keeps the tick going and stops the decisions, so a held
#   lane does go stale, as a file_host that stopped would.
# - **Its word, whole.** $SCRATCH/verb is `<n> <verb>`, written whole by
#   `lane_says`, and each round echoes back in $SCRATCH/wrote the `<n>` it
#   has written. `lane_says` waits for that echo, not for a guessed time.
whole "$SCRATCH/verb" "0 run"
(
  while :; do
    now=$(date +%s)
    whole "$SCRATCH/tick" "$now"
    read -r n verb < "$SCRATCH/verb"
    root=$(cat "$SCRATCH/root" 2> /dev/null) && [ -n "$root" ] && mkdir -p "$root/renders/call-worker" && {
      at=$now
      [ "$verb" = stale ] && verb=run at=$(( now - 60 ))
      if [ "$verb" != hold ]; then
        cat "$root/renders/call-worker/job" 2> /dev/null >> "$SCRATCH/saw" || echo none >> "$SCRATCH/saw"
        printf 'verb=%s\nwhy=test\nstops=0\nstuck=false\nat=%s\n' "$verb" "$at" > "$root/renders/call-worker/.lane" &&
          mv "$root/renders/call-worker/.lane" "$root/renders/call-worker/lane"
      fi
      whole "$SCRATCH/wrote" "$n"
    }
    sleep 0.1
  done
) &
LANE_WRITER=$!
trap 'kill "$LANE_WRITER" 2> /dev/null; rm -rf "$SCRATCH"' EXIT

# A decision's age, against the fake lane's clock (render-calls.sh's own
# `lane_now` reads the wall).
lane_now() {
  cat "$SCRATCH/tick"
}

SAID=0
lane_said() {
  [ "$(cat "$SCRATCH/wrote" 2> /dev/null)" = "$1" ]
}

# The fake lane says `verb` from now on; back once it has written it.
lane_says() {
  SAID=$(( SAID + 1 ))
  whole "$SCRATCH/verb" "$SAID $1"
  within lane_said "$SAID" || echo "the fake lane did not take $1 within 20 s"
}

# The fake recorder: `tsx tests/record/call.ts … --week-file <f> --out <w>`.
# It writes one step into <w>/<week>/steps (saying `reused step` if the step
# was already there), then fails if $FAKE/fail-<week> exists, else writes the
# video and a cache folder. $FAKE/warn makes it say a cache warning;
# $FAKE/bulk-<week> makes its step that many MB. Each run is counted in
# $FAKE/runs. With $FAKE/steps (a count), it then makes that many steps, each
# kept whole, each taking $FAKE/slow seconds in a child it waits on, and
# saying `cache hit: step <n>` for one already kept; the video is made of
# them; while $FAKE/hold-at holds a number n, it waits before step n + 1,
# in one child that lives as long as the hold, so a case acts on it half
# way without racing it.
# $FAKE/fail-after-pause makes it fail once after its steps. It keeps
# its pid in $FAKE/pid, its child's in $FAKE/child, and the job it saw in
# the lane's folder in $FAKE/seen-job.
# Both recorders keep the arguments they were given, one to a line, in
# $FAKE/args-<script> (call, preview), and stamp each run's start and end in
# $FAKE/times (`start <s>`, `end <s>`). While $FAKE/too-large exists, each
# makes its video and then refuses it in the real recorder's words
# (some-ui packages/video-render/src/policy.ts), naming the --max-video-mb it
# was given, and removes it, as the real one does.
cat > "$TSX" <<'FAKE_RECORDER'
#!/usr/bin/env bash
set -u
fake=$(dirname "$0")
script=$(basename "$1" .ts)
printf '%s\n' "$@" > "$fake/args-$script"
echo "start $(date +%s)" >> "$fake/times"
trap 'echo "end $(date +%s)" >> "$fake/times"' EXIT
max_mb=
prev=
for arg in "$@"; do
  [ "$prev" = --max-video-mb ] && max_mb=$arg
  prev=$arg
done
# The real recorder's refusal of a video past --max-video-mb, once made.
too_large() {
  if [ -e "$fake/too-large" ]; then
    rm -f "$1"
    echo "the video is 512 MB, past the policy's $max_mb MB" >&2
    exit 1
  fi
}
# A preview (SFEED12): `tsx tests/record/preview.ts … --template <t> --out
# <w>/<id>` writes preview.mp4, its sample week (naming the template's
# version) and a cache into <w>/<id>, or fails as the real one does for a
# template with no sample fill while $FAKE/no-sample exists. Each run is
# counted in $FAKE/preview-runs as `<template> <out>`.
if [ "$1" = tests/record/preview.ts ]; then
  template= out=
  while [ $# -gt 0 ]; do
    case $1 in
      --template) template=$2; shift ;;
      --out) out=$2; shift ;;
      --week | --week-file) echo "preview.ts takes no $1"; exit 2 ;;
    esac
    shift
  done
  echo "$template $out" >> "$fake/preview-runs"
  if [ -e "$fake/no-sample" ]; then
    echo "$template has no sample fill yet" >&2
    exit 1
  fi
  mkdir -p "$out/cache"
  echo intermediate > "$out/cache/frame"
  printf '{"template":{"id":"%s","version":2}}\n' "$template" > "$out/week.json"
  echo "preview of $template" > "$out/preview.mp4"
  too_large "$out/preview.mp4"
  # As if an earlier attempt had written it: the worker's move dates it.
  touch -d '2 days ago' "$out/preview.mp4"
  exit 0
fi
# A clip's walk: `tsx src/cli.ts walk <app> --sample <f> --out <w>/<key>`
# writes walk.json and a snip into <w>/<key>, as the motion does. Each run is
# counted in $FAKE/walk-runs as `<app> <sample> <out>`.
if [ "$1" = src/cli.ts ]; then
  [ "$2" = walk ] || { echo "not a walk: $2"; exit 2; }
  app=$3 sample= out=
  shift 3
  while [ $# -gt 0 ]; do
    case $1 in
      --sample) sample=$2; shift ;;
      --out) out=$2; shift ;;
    esac
    shift
  done
  echo "$app $sample $out" >> "$fake/walk-runs"
  mkdir -p "$out/snips"
  echo png > "$out/snips/01.png"
  printf '{"schema":"some-ui.walk/1","app":"%s"}\n' "$app" > "$out/walk.json"
  exit 0
fi
# An essay (RR16): `tsx tests/record/essay.ts … --essay-file <f> --takes <d>
# --out <w>` writes essay.mp4 and a cache into <w>/<week>, the week being
# the essay file's folder's name, or fails as the real one does for a take
# its folder lacks while $FAKE/no-take exists. Each run is counted in
# $FAKE/essay-runs as `<essay file> <takes> <out>`.
if [ "$1" = tests/record/essay.ts ]; then
  essay= takes= out=
  while [ $# -gt 0 ]; do
    case $1 in
      --essay-file) essay=$2; shift ;;
      --takes) takes=$2; shift ;;
      --out) out=$2; shift ;;
      --week | --week-file) echo "essay.ts takes no $1"; exit 2 ;;
    esac
    shift
  done
  echo "$essay $takes $out" >> "$fake/essay-runs"
  if [ -e "$fake/correct-mid-render" ]; then
    rm -f "$fake/correct-mid-render"
    echo '{"r":2}' > "$essay"
    echo 2 > "$(dirname "$essay")/revision"
  fi
  if [ -e "$fake/no-take" ]; then
    echo "QM-311 is answered with a take, and $takes has none for it" >&2
    exit 1
  fi
  week=$(basename "$(dirname "$essay")")
  mkdir -p "$out/$week/cache"
  echo intermediate > "$out/$week/cache/frame"
  cp "$essay" "$out/$week/essay.json"
  echo "essay of $week" > "$out/$week/essay.mp4"
  too_large "$out/$week/essay.mp4"
  exit 0
fi
[ "$1" = tests/record/call.ts ] || { echo "not the call's recorder: $1"; exit 2; }
week_file= out=
while [ $# -gt 0 ]; do
  case $1 in
    --week-file) week_file=$2; shift ;;
    --out) out=$2; shift ;;
  esac
  shift
done
week=$(basename "$week_file" .json)
echo "$week" >> "$fake/runs"
echo "recording $week: no cache warning here"
echo $$ > "$fake/pid"
cat "$(dirname "$out")/call-worker/job" > "$fake/seen-job" 2> /dev/null
dir=$out/$week
mkdir -p "$dir/steps"
if [ -e "$dir/steps/said" ]; then
  echo "reused step said"
else
  echo said > "$dir/steps/said"
fi
if [ -e "$fake/bulk-$week" ]; then
  head -c "$(( $(cat "$fake/bulk-$week") * 1048576 ))" /dev/urandom > "$dir/steps/bulk"
fi
[ -e "$fake/warn" ] && echo "cache warning: step said was not kept: past the policy's cache"
if [ -e "$fake/fail-$week" ]; then
  echo "the stage did not answer"
  [ -e "$fake/silent" ] && sleep "$(cat "$fake/silent")"
  exit 1
fi
if [ -e "$fake/steps" ]; then
  for i in $(seq 1 "$(cat "$fake/steps")"); do
    if [ -e "$dir/steps/step-$i" ]; then
      echo "cache hit: step $i"
      continue
    fi
    # Held in one child that lives as long as the hold, so the child a case
    # looks at is never one that has just ended between two short sleeps.
    if [ -e "$fake/hold-at" ] && [ "$i" -gt "$(cat "$fake/hold-at" 2> /dev/null || echo 99)" ]; then
      (
        while [ -e "$fake/hold-at" ] && [ "$i" -gt "$(cat "$fake/hold-at" 2> /dev/null || echo 99)" ]; do
          sleep 0.05
        done
      ) &
      echo $! > "$fake/child"
      wait $!
    fi
    sleep "$(cat "$fake/slow" 2> /dev/null || echo 0)" &
    echo $! > "$fake/child"
    wait $!
    echo "step $i of $week" > "$dir/steps/.step-$i" && mv "$dir/steps/.step-$i" "$dir/steps/step-$i"
    echo "made step $i"
  done
  if [ -e "$fake/fail-after-pause" ]; then
    rm "$fake/fail-after-pause"
    echo "a page timed out"
    exit 1
  fi
fi
mkdir -p "$dir/cache"
echo intermediate > "$dir/cache/frame"
echo "video of $week" > "$dir/call.mp4"
if [ -e "$fake/steps" ]; then
  cat "$dir"/steps/step-* >> "$dir/call.mp4"
fi
too_large "$dir/call.mp4"
FAKE_RECORDER
chmod +x "$TSX"

failures=0
check() {
  local what=$1
  shift
  if "$@"; then
    echo "ok: $what"
  else
    echo "FAILED: $what"
    # What the worker said, so a failure on a runner can be read from its log.
    [ -e "$SCRATCH/pass.out" ] && sed 's/^/  pass.out: /' "$SCRATCH/pass.out" | tail -n 40
    failures=$((failures + 1))
  fi
}

# A fresh home: ROOT with one or more weeks file_host has written.
fresh() {
  ROOT=$SCRATCH/root.$1
  shift
  mkdir -p "$ROOT/weeks"
  local week
  for week in "$@"; do
    echo '{}' > "$ROOT/weeks/$week.json"
  done
  rm -f "$FAKE"/fail-* "$FAKE"/bulk-* "$FAKE/warn" "$FAKE/runs" "$FAKE/steps" "$FAKE/slow" "$FAKE/hold-at" "$FAKE/silent" \
    "$FAKE/pid" "$FAKE/child" "$FAKE/seen-job" "$FAKE/no-sample" "$FAKE/preview-runs" "$FAKE/no-take" "$FAKE/essay-runs" "$FAKE/correct-mid-render" \
    "$FAKE"/args-* "$FAKE/times" "$FAKE/too-large" "$FAKE/walk-runs"
  echo "$ROOT" > "$SCRATCH/root"
  : > "$SCRATCH/saw"
  lane_says run
}

pass() {
  run_job call > "$SCRATCH/pass.out" 2>&1
}

# The lane saw the job taken away, then `id` named again after it.
taken_away_then() {
  grep -n . "$SCRATCH/saw" | awk -F: -v id="$1" '$2 == "none" { gone = 1 } gone && $2 == id { found = 1 } END { exit !found }'
}

runs_of() {
  if [ -e "$FAKE/runs" ]; then
    grep -cx "$1" "$FAKE/runs" || true
  else
    echo 0
  fi
}

# A pass in the background, for a case to act on half way.
pass_behind() {
  run_job call > "$SCRATCH/pass.out" 2>&1 &
  PASS=$!
}

# Waits up to 20 s for `cmd` to hold.
within() {
  local tries=0
  until "$@"; do
    tries=$(( tries + 1 ))
    [ "$tries" -lt 200 ] || return 1
    sleep 0.1
  done
}

steps_made() {
  ls "$ROOT/renders/call-work/2026-10-30/steps/" 2> /dev/null | grep -c '^step-' || true
}

has_steps() {
  [ "$(steps_made)" -ge "$1" ]
}

# A process's state letter: T when stopped; nothing when it is gone.
state_of() {
  local stat
  read -r stat 2> /dev/null < "/proc/$1/stat" || return 0
  stat=${stat##*) }
  echo "${stat%% *}"
}

# Gone, or a zombie: killed, holding no memory, waiting only to be reaped by
# whichever process inherited it, which on a busy runner may not be at once.
gone() {
  local state
  state=$(state_of "$1")
  [ -z "$state" ] || [ "$state" = Z ]
}

job_state_is() {
  grep -qx "state=$1" "$ROOT/renders/call-worker/job.state" 2> /dev/null
}

video_of() {
  cat "$ROOT/renders/call/2026-10-30/call.mp4" 2> /dev/null
}

# The video of 2026-10-30 rendered with six steps and never held back: what
# every case that holds it back must give, byte for byte.
fresh reference 2026-10-30
echo 6 > "$FAKE/steps"
pass
REFERENCE=$(video_of)

# A failed week, then the same week once its cause is gone.
fresh failed 2026-10-30
touch "$FAKE/fail-2026-10-30"
pass
check "a failed attempt leaves no folder in renders/call" test ! -e "$ROOT/renders/call/2026-10-30"
check "a failed attempt's output is its failed.log" grep -q "the stage did not answer" "$ROOT/renders/call-worker/2026-10-30.failed.log"
check "a failed attempt keeps its work" test -e "$ROOT/renders/call-work/2026-10-30/steps/said"
check "the pass stamped last-pass" test -s "$ROOT/renders/call-worker/last-pass"
pass
check "a failed week is tried again on the next pass" test "$(runs_of 2026-10-30)" -eq 2
rm "$FAKE/fail-2026-10-30"
pass
check "the next attempt reuses the kept work" grep -q "reused step said" "$SCRATCH/pass.out"
check "a finished week is moved into renders/call whole" test -e "$ROOT/renders/call/2026-10-30/call.mp4" -a -e "$ROOT/renders/call/2026-10-30/steps/said"
check "a finished week leaves its cache behind" test ! -e "$ROOT/renders/call/2026-10-30/cache"
check "a finished week's work is removed" test ! -e "$ROOT/renders/call-work/2026-10-30"
check "a finished week's failed.log is removed" test ! -e "$ROOT/renders/call-worker/2026-10-30.failed.log"

# A pass over a finished week.
# Name, size, change time to the nanosecond, inode, and the bytes of each file.
snapshot() {
  (cd "$ROOT/renders/call" && find . -exec stat -c '%n %s %z %i' {} + && find . -type f -exec md5sum {} +) | sort
}
before=$(snapshot)
pass
after=$(snapshot)
check "a pass over a finished week does not record it" test "$(runs_of 2026-10-30)" -eq 3
check "a pass over a finished week leaves its folder untouched" test "$before" = "$after"

# The all-weeks cap: two weeks waiting and a finished one, over a 1 MB cap.
# The oldest renders first, so the work removed is the other one's, and the
# finished week (2 MB, over the cap itself) is never counted or touched.
fresh cap 2026-10-16 2026-10-23 2026-10-30
WORK_MAX_MB=1
mkdir -p "$ROOT/renders/call/2026-10-30"
head -c 2097152 /dev/urandom > "$ROOT/renders/call/2026-10-30/call.mp4"
echo 2 > "$FAKE/bulk-2026-10-23"
touch "$FAKE/fail-2026-10-16" "$FAKE/fail-2026-10-23"
pass
check "both failed weeks keep their work" test -e "$ROOT/renders/call-work/2026-10-16/steps/said" -a -e "$ROOT/renders/call-work/2026-10-23/steps/bulk"
video_before=$(md5sum < "$ROOT/renders/call/2026-10-30/call.mp4")
rm "$FAKE/bulk-2026-10-23"
pass
check "past the cap, another waiting week's work is removed" test ! -e "$ROOT/renders/call-work/2026-10-23/steps/bulk"
check "past the cap, the week being rendered keeps its work" grep -q "reused step said" "$SCRATCH/pass.out"
check "the removal is said in work.cache.log" grep -q "^cache warning: removed the waiting work of 2026-10-23" "$ROOT/renders/call-worker/work.cache.log"
check "the cap never removes a video" test "$(md5sum < "$ROOT/renders/call/2026-10-30/call.mp4")" = "$video_before"
WORK_MAX_MB=4096

# A waiting week the owner lets go: file_host takes it out of /weeks, and its
# work goes before the next week renders.
fresh let-go 2026-10-23 2026-10-30
touch "$FAKE/fail-2026-10-30"
pass
rm "$ROOT/weeks/2026-10-30.json"
echo '{}' > "$ROOT/weeks/2026-11-06.json"
pass
check "a week no longer held loses its waiting work" test ! -e "$ROOT/renders/call-work/2026-10-30"
check "a week no longer held is not tried again" test "$(runs_of 2026-10-30)" -eq 1

# Cache warnings.
fresh warned 2026-10-30
touch "$FAKE/warn"
pass
check "a cache warning reaches <week>.cache.log" grep -qx "cache warning: step said was not kept: past the policy's cache" "$ROOT/renders/call-worker/2026-10-30.cache.log"
check "only the warning lines do" test "$(wc -l < "$ROOT/renders/call-worker/2026-10-30.cache.log")" -eq 1
fresh warned-failed 2026-10-30
touch "$FAKE/warn" "$FAKE/fail-2026-10-30"
pass
check "a failed attempt's cache warning reaches <week>.cache.log" grep -qx "cache warning: step said was not kept: past the policy's cache" "$ROOT/renders/call-worker/2026-10-30.cache.log"

# The lane: no render on a missing or stale decision.
fresh no-lane 2026-10-30
lane_says stale
pass_behind
sleep 1
check "a stale decision starts no render" test "$(runs_of 2026-10-30)" -eq 0
check "a job waiting for room is named where the lane reads it" test -s "$ROOT/renders/call-worker/job"
check "and its state says it waits" job_state_is waiting
check "and its log is an attempt in progress, so home is not called idle" test -e "$ROOT/renders/call-worker/.2026-10-30.log"
lane_says stop
rm -f "$ROOT/renders/call-worker/lane"
lane_says hold
rm -f "$ROOT/renders/call-worker/lane"
sleep 0.3
check "a missing decision starts no render" test "$(runs_of 2026-10-30)" -eq 0
waiting_id=$(cat "$ROOT/renders/call-worker/job")
lane_says run
wait "$PASS"
check "the render starts once the lane says run" test "$(runs_of 2026-10-30)" -eq 1
check "the job it ran under is the one the lane was shown" test "$(cat "$FAKE/seen-job")" = "$waiting_id"
check "a rendered job is no longer named" test ! -e "$ROOT/renders/call-worker/job" -a ! -e "$ROOT/renders/call-worker/job.state"
check "how long it rendered is kept for the next one's bar" grep -qx '[0-9]\+' "$ROOT/renders/call-worker/took"
check "its measured peak is the container's and what runs beside it" test "$(cat "$ROOT/renders/call-worker/peak")" -eq $(( 3000000000 + BESIDE_BYTES ))

# A run decision written before the job was is not the lane's word on it.
fresh unseen 2026-10-30
lane_says hold
pass_behind
sleep 1.5
check "a run decision older than the job starts no render" test "$(runs_of 2026-10-30)" -eq 0
lane_says run
wait "$PASS"
check "the next decision does" test "$(runs_of 2026-10-30)" -eq 1

# A decision that goes stale while a render runs is stop.
fresh went-stale 2026-10-30
echo 6 > "$FAKE/steps"
echo 1 > "$FAKE/hold-at"
pass_behind
check "the recorder reaches its first step" within has_steps 1
recorder=$(cat "$FAKE/pid")
id=$(cat "$ROOT/renders/call-worker/job")
lane_says hold
check "a decision gone stale stops a running render" within gone "$recorder"
check "and says why" within grep -q "gave way (the lane: stop, stale)" "$SCRATCH/pass.out"
rm "$FAKE/hold-at"
: > "$SCRATCH/saw"
lane_says run
wait "$PASS"
check "and it resumes with the same bytes" test "$(video_of)" = "$REFERENCE"
check "after the lane saw it taken away and named again, so it checked room" taken_away_then "$id"

# A pause: frozen, then finished with the same bytes.
fresh paused 2026-10-30
echo 6 > "$FAKE/steps"
echo 2 > "$FAKE/hold-at"
pass_behind
check "the recorder reaches its second step" within has_steps 2
lane_says pause
check "the job says it is paused" within job_state_is paused
held=$(steps_made)
rm "$FAKE/hold-at"
sleep 1
check "a pause makes no more steps, even with nothing holding it" test "$(steps_made)" -eq "$held"
check "a pause stops the recorder" test "$(state_of "$(cat "$FAKE/pid")")" = T
check "and what the recorder started" test "$(state_of "$(cat "$FAKE/child")")" = T
lane_says run
wait "$PASS"
check "a paused render finishes once the lane says run" test "$(video_of)" = "$REFERENCE"
check "a paused render made each step once" test "$(runs_of 2026-10-30)" -eq 1
check "a pause is no failure" test ! -e "$ROOT/renders/call-worker/2026-10-30.failed.log"
check "the pause is said in the log" grep -q "^paused the call of 2026-10-30" "$SCRATCH/pass.out"

# A stop: killed, the work and the id kept, resumed from the kept steps.
fresh stopped 2026-10-30
echo 6 > "$FAKE/steps"
echo 2 > "$FAKE/hold-at"
pass_behind
check "the recorder reaches its second step" within has_steps 2
id=$(cat "$ROOT/renders/call-worker/job")
recorder=$(cat "$FAKE/pid") child=$(cat "$FAKE/child")
: > "$SCRATCH/saw"
lane_says stop
check "a stop kills the recorder" within gone "$recorder"
check "and what the recorder started" within gone "$child"
check "the job says it waits" within job_state_is waiting
kept=$(steps_made)
check "a stop keeps the finished steps" test "$kept" -eq 2
rm "$FAKE/hold-at"
check "a stop keeps the job's id" test "$(cat "$ROOT/renders/call-worker/job")" = "$id"
check "a job that gave way counts it" grep -qx "yields=1" "$ROOT/renders/call-worker/job.state"
check "and the time it rendered before it did" grep -qx "ran=[0-9]\+" "$ROOT/renders/call-worker/job.state"
check "a stop is no failure" test ! -e "$ROOT/renders/call-worker/2026-10-30.failed.log"
sleep 0.5
check "a stopped render waits for room" test "$(runs_of 2026-10-30)" -eq 1
check "while the job stays named: a stop the lane decided keeps its count" bash -c '! grep -qx none "$1"' _ "$SCRATCH/saw"
lane_says run
wait "$PASS"
check "a stopped render resumes" test "$(runs_of 2026-10-30)" -eq 2
check "the resumed run ran under the same job" test "$(cat "$FAKE/seen-job")" = "$id"
check "it hits the cache for every step it had finished" test "$(grep -c '^cache hit: step' "$SCRATCH/pass.out")" -eq "$kept"
check "it makes each step once" test "$(grep -c '^made step' "$SCRATCH/pass.out")" -eq 6
check "a resumed render has the same bytes as one never stopped" test "$(video_of)" = "$REFERENCE"
check "a resumed render is no failure" test ! -e "$ROOT/renders/call-worker/2026-10-30.failed.log"

# An attempt that fails after a pause is run again, not failed.
fresh timed-out 2026-10-30
echo 3 > "$FAKE/steps"
echo 1 > "$FAKE/hold-at"
touch "$FAKE/fail-after-pause"
pass_behind
check "the recorder reaches its first step" within has_steps 1
id=$(cat "$ROOT/renders/call-worker/job")
lane_says pause
check "the job says it is paused" within job_state_is paused
rm "$FAKE/hold-at"
: > "$SCRATCH/saw"
lane_says run
wait "$PASS"
check "a failure soon after a pause is run again" test "$(runs_of 2026-10-30)" -eq 2
check "and is no failure" test ! -e "$ROOT/renders/call-worker/2026-10-30.failed.log"
check "and renders" test -e "$ROOT/renders/call/2026-10-30/call.mp4"
check "after the lane saw it taken away and named again" taken_away_then "$id"

# Past PAUSE_FAILS, a failure after a pause is a failure: a real one never
# loops.
fresh pause-fails 2026-10-30
echo 3 > "$FAKE/steps"
echo 1 > "$FAKE/hold-at"
touch "$FAKE/fail-after-pause"
PAUSE_FAILS=0
pass_behind
check "the recorder reaches its first step" within has_steps 1
lane_says pause
check "the job says it is paused" within job_state_is paused
rm "$FAKE/hold-at"
lane_says run
wait "$PASS"
PAUSE_FAILS=3
check "past the cap a failure after a pause is a failure" test -e "$ROOT/renders/call-worker/2026-10-30.failed.log"
check "and is not run again" test "$(runs_of 2026-10-30)" -eq 1

# A failed job is over; a peak that can't be read is not written.
fresh failed-job 2026-10-30
touch "$FAKE/fail-2026-10-30"
mv "$CGROUP/memory.peak" "$CGROUP/peak.away"
pass
check "a failed job is no longer named" test ! -e "$ROOT/renders/call-worker/job" -a ! -e "$ROOT/renders/call-worker/job.state"
rm "$FAKE/fail-2026-10-30"
pass
check "an unreadable memory.peak writes no peak" test ! -e "$ROOT/renders/call-worker/peak"
mv "$CGROUP/peak.away" "$CGROUP/memory.peak"

# A week let go while it waits for room.
fresh let-go-waiting 2026-10-30
lane_says stop
pass_behind
within job_state_is waiting
rm "$ROOT/weeks/2026-10-30.json"
wait "$PASS"
check "a week let go while it waits is never recorded" test "$(runs_of 2026-10-30)" -eq 0
check "and is no failure" test ! -e "$ROOT/renders/call-worker/2026-10-30.failed.log"
check "and its job is over" test ! -e "$ROOT/renders/call-worker/job"

# A job a restart left: its id and work are kept, and it runs first.
fresh restart 2026-10-16 2026-10-23
mkdir -p "$ROOT/renders/call-worker" "$ROOT/renders/call-work/2026-10-23/steps"
head -c 2097152 /dev/zero > "$ROOT/renders/call-work/2026-10-23/steps/bulk"
echo 42 > "$ROOT/renders/call-worker/job"
printf 'id=42\nkind=call\nkey=2026-10-23\nsince=1794000000\nstate=rendering\nstate_since=1794000000\nyields=0\n' \
  > "$ROOT/renders/call-worker/job.state"
WORK_MAX_MB=1
touch "$FAKE/fail-2026-10-16"
use_job call
render_one call 2026-10-16 > "$SCRATCH/pass.out" 2>&1
check "past the cap, a job a restart left keeps its work" test -e "$ROOT/renders/call-work/2026-10-23/steps/bulk"
WORK_MAX_MB=4096
rm "$FAKE/fail-2026-10-16"
echo 42 > "$ROOT/renders/call-worker/job"
printf 'id=42\nkind=call\nkey=2026-10-23\nsince=1794000000\nstate=rendering\nstate_since=1794000000\nyields=0\n' \
  > "$ROOT/renders/call-worker/job.state"
: > "$FAKE/runs"
sleep 0.3
: > "$SCRATCH/saw"
resume_left > "$SCRATCH/pass.out" 2>&1
check "a job a restart left runs first" test "$(head -n1 "$FAKE/runs")" = 2026-10-23
check "after the lane saw it taken away and named again, so it checked room" taken_away_then 42
check "under the id it had" test "$(cat "$FAKE/seen-job")" = 42
check "and renders" test -e "$ROOT/renders/call/2026-10-23/call.mp4"

# Attempt logs a killed attempt left are cleared between jobs; cache logs stay.
fresh stale-logs 2026-10-30
mkdir -p "$ROOT/renders/call-worker" "$ROOT/renders/playoff-delta-worker"
touch "$ROOT/renders/call-worker/.2026-10-23.log" "$ROOT/renders/playoff-delta-worker/.2026-SF-09.log" \
  "$ROOT/renders/call-worker/.2026-10-23.cache.log" "$ROOT/renders/call-worker/2026-10-23.cache.log"
resume_left > /dev/null 2>&1
check "an attempt log left by a killed attempt is cleared" test ! -e "$ROOT/renders/call-worker/.2026-10-23.log" -a ! -e "$ROOT/renders/playoff-delta-worker/.2026-SF-09.log"
check "cache logs are not" test -e "$ROOT/renders/call-worker/.2026-10-23.cache.log" -a -e "$ROOT/renders/call-worker/2026-10-23.cache.log"

# A new job's id is never one given before.
fresh ids 2026-10-30
mkdir -p "$ROOT/renders/call-worker"
echo 9000000000000000000 > "$ROOT/renders/call-worker/job-seq"
use_job call
new_job_id
check "a job's id is past the last one given, whatever the clock says" test "$JOB_ID" = 9000000000000000001
echo 5 > "$ROOT/renders/call-worker/job-seq"
new_job_id
first=$JOB_ID
new_job_id
check "two jobs never share an id" test "$JOB_ID" -gt "$first"
echo 9999999999999999999 > "$ROOT/renders/call-worker/job-seq"
new_job_id
check "a job-seq past what bash holds is not read" test "$JOB_ID" -gt 0

# A failed week waits RETRY_AFTER before it is tried again.
fresh retry 2026-10-30
RETRY_AFTER=3600
touch "$FAKE/fail-2026-10-30"
pass
pass
check "a failed week is not tried again within RETRY_AFTER" test "$(runs_of 2026-10-30)" -eq 1
touch -d '2 hours ago' "$ROOT/renders/call-worker/2026-10-30.failed.log"
pass
check "and is, RETRY_AFTER after its failure" test "$(runs_of 2026-10-30)" -eq 2

# RETRY_AFTER counts from the failure, not from the log's last line: a
# recorder silent for a while before it fails is not tried again at once.
# It is read from the log's own time, not from how soon the next pass
# comes, so a slow runner can't make it pass or fail.
fresh silent 2026-10-30
RETRY_AFTER=5
touch "$FAKE/fail-2026-10-30"
echo 6 > "$FAKE/silent"
began=$(date +%s)
pass
check "a failure's time is when it failed, not its log's last line" \
  test "$(stat -c %Y "$ROOT/renders/call-worker/2026-10-30.failed.log")" -ge $(( began + 6 ))
touch -d '1 second ago' "$ROOT/renders/call-worker/2026-10-30.failed.log"
pass
check "so a failure is not tried again within RETRY_AFTER of it" test "$(runs_of 2026-10-30)" -eq 1
RETRY_AFTER=0

# A template's preview (SFEED12, paulgsc/some-ui#2757): the owner's request
# file_host wrote is rendered with the template its id names, moved whole
# into renders/preview without its cache, and a request of another shape is
# never read.
fresh preview
mkdir -p "$ROOT/previews"
ID=conference-call-20261108T074500Z
echo '{}' > "$ROOT/previews/$ID.json"
echo '{}' > "$ROOT/previews/not-a-template-20261108T074500Z.json"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "a preview is recorded with its template, into its own work folder" \
  test "$(cat "$FAKE/preview-runs")" = "conference-call $ROOT/renders/preview-work/$ID"
check "and kept whole in renders/preview" test -e "$ROOT/renders/preview/$ID/preview.mp4" -a -e "$ROOT/renders/preview/$ID/week.json"
check "without its cache, and its work gone" test ! -e "$ROOT/renders/preview/$ID/cache" -a ! -e "$ROOT/renders/preview-work/$ID"
check "a request of a template there is none of is never read" test "$(wc -l < "$FAKE/preview-runs")" -eq 1
check "its job is over" test ! -e "$ROOT/renders/call-worker/job"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "a ready preview is not made again" test "$(wc -l < "$FAKE/preview-runs")" -eq 1
check "its video's time is when it was moved into place" test "$(( $(date +%s) - $(stat -c %Y "$ROOT/renders/preview/$ID/preview.mp4") ))" -lt 60
echo '{}' > "$ROOT/previews/rust-essay-20261110T050000Z.json"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "the essay's template is previewed too (RR16)" test -e "$ROOT/renders/preview/rust-essay-20261110T050000Z/preview.mp4"

# A preview that fails stays failed, RETRY_AFTER or not: the owner asks for
# another, under a new id.
fresh preview-failed
mkdir -p "$ROOT/previews"
ID=playoff-delta-week-20261108T074500Z
echo '{}' > "$ROOT/previews/$ID.json"
touch "$FAKE/no-sample"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "a failed preview's output is its failed.log" grep -q "has no sample fill yet" "$ROOT/renders/preview-worker/$ID.failed.log"
check "and it leaves no folder in renders/preview" test ! -e "$ROOT/renders/preview/$ID"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "a failed preview is never tried again" test "$(wc -l < "$FAKE/preview-runs")" -eq 1

# A Rust Rounds essay (RR16): each week file_host gave with its essay.json is
# recorded from it and its takes, kept whole in renders/rust-essay without
# its cache; a folder of another shape, or without its essay, is never read.
fresh essay
mkdir -p "$ROOT/essays/2026-w46/takes" "$ROOT/essays/2026-w47" "$ROOT/essays/not-a-week"
echo '{}' > "$ROOT/essays/2026-w46/essay.json"
echo '{}' > "$ROOT/essays/not-a-week/essay.json"
run_job rust-essay > "$SCRATCH/pass.out" 2>&1
check "an essay is recorded from its file and its takes, into its job's work folder" \
  test "$(cat "$FAKE/essay-runs")" = "$ROOT/essays/2026-w46/essay.json $ROOT/essays/2026-w46/takes $ROOT/renders/rust-essay-work"
check "and kept whole in renders/rust-essay" test -e "$ROOT/renders/rust-essay/2026-w46/essay.mp4" -a -e "$ROOT/renders/rust-essay/2026-w46/essay.json"
check "without its cache, and its work gone" test ! -e "$ROOT/renders/rust-essay/2026-w46/cache" -a ! -e "$ROOT/renders/rust-essay-work/2026-w46"
check "with the speaker and the caps every job has" grep -qx -- --max-video-mb "$FAKE/args-essay"
run_job rust-essay > "$SCRATCH/pass.out" 2>&1
check "a rendered essay is not made again, and no other folder is read" test "$(wc -l < "$FAKE/essay-runs")" -eq 1

# An essay corrected while it renders (C72): what was made of the old
# revision is never kept, and it runs again over the new one.
fresh essay-corrected
mkdir -p "$ROOT/essays/2026-w46/takes"
echo '{"r":1}' > "$ROOT/essays/2026-w46/essay.json"
echo 1 > "$ROOT/essays/2026-w46/revision"
# The fake recorder's first run finds the correction given meanwhile.
touch "$FAKE/correct-mid-render"
run_job rust-essay > "$SCRATCH/pass.out" 2>&1
check "a render of a superseded revision is run again" test "$(wc -l < "$FAKE/essay-runs")" -eq 2
check "and the kept essay is the newer revision's" grep -q '"r":2' "$ROOT/renders/rust-essay/2026-w46/essay.json"
check "and that is said" grep -q "corrected while it rendered" "$SCRATCH/pass.out"

# An essay that fails keeps its output for the owner and is tried again.
fresh essay-failed
mkdir -p "$ROOT/essays/2026-w46/takes"
echo '{}' > "$ROOT/essays/2026-w46/essay.json"
touch "$FAKE/no-take"
run_job rust-essay > "$SCRATCH/pass.out" 2>&1
check "a failed essay's output is its failed.log" grep -q "has none for it" "$ROOT/renders/rust-essay-worker/2026-w46.failed.log"
check "and it leaves no folder in renders/rust-essay" test ! -e "$ROOT/renders/rust-essay/2026-w46"
check "and the job is over" test ! -e "$ROOT/renders/call-worker/job"

# A clip's walk: each folder file_host gave with its sample and app is
# walked by the motion, kept whole in renders/clip-walk; a folder of another
# shape, or naming an app that is not plain, is never walked.
fresh walk
mkdir -p "$ROOT/clip-walks/ledger-wake-after" "$ROOT/clip-walks/bad-app-after" "$ROOT/clip-walks/not-a-side"
for key in ledger-wake-after bad-app-after not-a-side; do echo '{}' > "$ROOT/clip-walks/$key/sample.json"; done
echo suspender-ledger > "$ROOT/clip-walks/ledger-wake-after/app"
echo '../x' > "$ROOT/clip-walks/bad-app-after/app"
echo suspender-ledger > "$ROOT/clip-walks/not-a-side/app"
run_job clip-walk > "$SCRATCH/pass.out" 2>&1
check "a walk is the motion's, over its sample, into its key's work folder" \
  test "$(cat "$FAKE/walk-runs")" = "suspender-ledger $ROOT/clip-walks/ledger-wake-after/sample.json $ROOT/renders/clip-walk-work/ledger-wake-after"
check "and kept whole in renders/clip-walk" test -e "$ROOT/renders/clip-walk/ledger-wake-after/walk.json" -a -e "$ROOT/renders/clip-walk/ledger-wake-after/snips/01.png"
check "a walk naming no plain app fails, unwalked" grep -q "names no app" "$ROOT/renders/clip-walk-worker/bad-app-after.failed.log"
run_job clip-walk > "$SCRATCH/pass.out" 2>&1
check "a walk is not made again, and no other folder is read" test "$(wc -l < "$FAKE/walk-runs")" -eq 1

# A preview a restart left runs first, as any job does.
fresh preview-left
mkdir -p "$ROOT/previews" "$ROOT/renders/call-worker"
ID=conference-call-20261108T080000Z
echo '{}' > "$ROOT/previews/$ID.json"
echo 77 > "$ROOT/renders/call-worker/job"
printf 'id=77\nkind=preview\nkey=%s\nsince=1\nstate=rendering\nstate_since=1\nyields=0\nran=0' "$ID" > "$ROOT/renders/call-worker/job.state"
resume_left > "$SCRATCH/pass.out" 2>&1
check "a preview a restart left is resumed" test -e "$ROOT/renders/preview/$ID/preview.mp4"

# What every recorder is handed (CW5, paulgsc/some-ui#2659): the render
# policy's caps, the pinned stage, and home's Kokoro speaker. Read from what
# the fake recorders were given, written out, not from the script's variables,
# so a cap changed without the ledger moving fails here.
HANDED=(--app http://call-stage --voices kokoro --speaker http://kokoro:8880 --brand /brand
  --max-video-minutes 15 --max-video-mb 500 --cache-mb 2048)
# Whether `args` (one to a line) has each of HANDED's flags followed by its value.
handed() {
  local i
  for (( i = 0; i < ${#HANDED[@]}; i += 2 )); do
    grep -A1 -xF -- "${HANDED[i]}" "$1" | sed -n 2p | grep -qxF -- "${HANDED[i + 1]}" || return 1
  done
}
fresh handed 2026-10-30
mkdir -p "$ROOT/previews"
echo '{}' > "$ROOT/previews/conference-call-20261108T090000Z.json"
pass
run_job preview > "$SCRATCH/pass.out" 2>&1
check "the call's recorder is handed the policy's caps, the pinned stage and home's speaker" handed "$FAKE/args-call"
check "and a preview's recorder the same" handed "$FAKE/args-preview"

# A video past MAX_VIDEO_MB (#752; PV2, CW5): the recorder refuses it once
# made and removes it, and the worker takes that as any failed attempt. A
# week keeps no folder and is not tried again within RETRY_AFTER; a preview
# keeps none and is never tried again.
fresh too-large 2026-10-30
RETRY_AFTER=3600
touch "$FAKE/too-large"
pass
check "a video past MAX_VIDEO_MB fails its week's attempt, with the cap named" \
  grep -qF "the video is 512 MB, past the policy's 500 MB" "$ROOT/renders/call-worker/2026-10-30.failed.log"
check "and leaves no folder in renders/call" test ! -e "$ROOT/renders/call/2026-10-30"
check "nor the video it refused" test ! -e "$ROOT/renders/call-work/2026-10-30/call.mp4"
pass
check "and is not tried again within RETRY_AFTER" test "$(runs_of 2026-10-30)" -eq 1
RETRY_AFTER=0
mkdir -p "$ROOT/previews"
ID=conference-call-20261108T100000Z
echo '{}' > "$ROOT/previews/$ID.json"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "a preview past MAX_VIDEO_MB fails, with the cap named" \
  grep -qF "the video is 512 MB, past the policy's 500 MB" "$ROOT/renders/preview-worker/$ID.failed.log"
check "and leaves no folder in renders/preview" test ! -e "$ROOT/renders/preview/$ID"
run_job preview > "$SCRATCH/pass.out" 2>&1
check "and is never tried again" test "$(wc -l < "$FAKE/preview-runs")" -eq 1

# The worker's own loop retries a failed week on its own clock (CW5): `serve`
# runs unattended, every EVERY seconds, and the week's second attempt starts
# no sooner than RETRY_AFTER after its first one ended, and does start.
# Times are the fake recorder's own stamps, whole seconds as the worker
# counts them, so a slow runner can only make the gap longer.
fresh serve 2026-10-30
EVERY=0.2
RETRY_AFTER=3
touch "$FAKE/fail-2026-10-30"
tried_twice() {
  [ "$(runs_of 2026-10-30)" -ge 2 ]
}
( serve ) > "$SCRATCH/pass.out" 2>&1 &
SERVE=$!
within tried_twice
kill "$SERVE" 2> /dev/null
wait "$SERVE" 2> /dev/null
check "serve tries a failed week again by itself" tried_twice
ended=$(awk '$1 == "end" { print $2; exit }' "$FAKE/times")
again=$(awk '$1 == "start" { n++ } n == 2 { print $2; exit }' "$FAKE/times")
check "no sooner than RETRY_AFTER after its failure" test "$(( again - ended ))" -ge "$RETRY_AFTER"
RETRY_AFTER=0

# The stage and the speaker are pinned by digest, the recorders by a whole
# commit, and the stage's pin names the commit the recorders are built at,
# so the pages the recorders drive are the ones they were written for
# (CW5: "from the pinned stage with each scene said by home's Kokoro speaker").
COMPOSE=$HERE/../compose/call-render.yml
# The stage's image is home's release's (deploy/home/release.json, #788),
# which compose reads as ${IMAGE_CALL_STAGE}.
RELEASE=$HERE/../../deploy/home/release.json
stage=$(grep -oE '"call-stage": \{"image": "[^"]+"' "$RELEASE" | cut -d'"' -f6)
check "the stage is pinned by digest" grep -qE '^pgathondu/www:[0-9.]+@sha256:[0-9a-f]{64}$' <<< "$stage"
check "the speaker is built from a base pinned by digest" \
  grep -qE '^FROM [^ ]+@sha256:[0-9a-f]{64}$' "$HERE/../docker/Dockerfile.kokoro"
recorders=$(grep -oE '^ARG SOME_UI_COMMIT=[0-9a-f]+$' "$HERE/../docker/Dockerfile.render" | cut -d= -f2)
check "the recorders are pinned by a whole commit" grep -qE '^[0-9a-f]{40}$' <<< "$recorders"
published=$(grep -A1 '^  call-stage:' "$COMPOSE" | grep -oE 'paulgsc/some-ui [0-9a-f]{7,40}' | cut -d' ' -f2)
check "the stage's pin names the recorders' commit" test -n "$published" -a "${recorders#"$published"}" != "$recorders"

if [ "$failures" -ne 0 ]; then
  echo "$failures of the render worker's guarantees failed"
  exit 1
fi
echo "the render worker's guarantees hold"
