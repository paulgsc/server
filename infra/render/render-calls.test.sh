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
# - `cache warning:` lines reach <week>.cache.log, a failed attempt's too.
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
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=render-calls.sh
source "$HERE/render-calls.sh"

SCRATCH=$(mktemp -d)
FAKE=$SCRATCH/fake
APP_DIR=$SCRATCH/app
TSX=$FAKE/tsx
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

# The fake lane: file_host's decision, rewritten whole every 0.1 s into the
# current ROOT's lane folder, saying $SCRATCH/verb (`stale` writes one a
# minute old; `hold` stops writing, as a file_host that stopped would).
echo run > "$SCRATCH/verb"
(
  while :; do
    root=$(cat "$SCRATCH/root" 2> /dev/null) && [ -n "$root" ] && mkdir -p "$root/renders/call-worker" && {
      verb=$(cat "$SCRATCH/verb") at=$(date +%s)
      [ "$verb" = stale ] && verb=run at=$(( at - 60 ))
      if [ "$verb" != hold ]; then
        cat "$root/renders/call-worker/job" 2> /dev/null >> "$SCRATCH/saw" || echo none >> "$SCRATCH/saw"
        printf 'verb=%s\nwhy=test\nstops=0\nstuck=false\nat=%s\n' "$verb" "$at" > "$root/renders/call-worker/.lane" &&
          mv "$root/renders/call-worker/.lane" "$root/renders/call-worker/lane"
      fi
    }
    sleep 0.1
  done
) &
LANE_WRITER=$!
trap 'kill "$LANE_WRITER" 2> /dev/null; rm -rf "$SCRATCH"' EXIT

lane_says() {
  echo "$1" > "$SCRATCH/verb"
  # Long enough for the fake lane to write it.
  sleep 0.3
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
# in a child too, so a case acts on it half way without racing it.
# $FAKE/fail-after-pause makes it fail once after its steps. It keeps
# its pid in $FAKE/pid, its child's in $FAKE/child, and the job it saw in
# the lane's folder in $FAKE/seen-job.
cat > "$TSX" <<'FAKE_RECORDER'
#!/usr/bin/env bash
set -u
fake=$(dirname "$0")
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
    while [ -e "$fake/hold-at" ] && [ "$i" -gt "$(cat "$fake/hold-at" 2> /dev/null || echo 99)" ]; do
      sleep 0.05 &
      echo $! > "$fake/child"
      wait $!
    done
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
    "$FAKE/pid" "$FAKE/child" "$FAKE/seen-job"
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

gone() {
  [ -z "$(state_of "$1")" ]
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
rm -f "$ROOT/renders/call-worker/lane"
echo stop > "$SCRATCH/verb"
sleep 0.5
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
check "and says why" grep -q "gave way (the lane: stop, stale)" "$SCRATCH/pass.out"
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
check "and what the recorder started" gone "$child"
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
fresh silent 2026-10-30
RETRY_AFTER=2
touch "$FAKE/fail-2026-10-30"
echo 3 > "$FAKE/silent"
pass
pass
check "a failure's time is when it failed, not its log's last line" test "$(runs_of 2026-10-30)" -eq 1
RETRY_AFTER=0

if [ "$failures" -ne 0 ]; then
  echo "$failures of the render worker's guarantees failed"
  exit 1
fi
echo "the render worker's guarantees hold"
