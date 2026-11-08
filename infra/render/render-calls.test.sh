#!/usr/bin/env bash
# The render worker's own guarantees (CW5, CW6; paulgsc/some-ui#2659), for the
# call's job, one pass at a time: render-calls.sh is sourced over a temporary ROOT, and tsx is
# a fake recorder that takes the call's real arguments. CI runs it
# (test.yml, metric_contract_ci). Each case says the guarantee it holds:
#
# - a failed attempt leaves no folder in renders/call, keeps its work, and
#   the next pass reuses it;
# - a finished week is moved whole, without its cache, and its work removed;
# - a pass over a finished week leaves its folder untouched;
# - the all-weeks cap removes only another waiting week's work, never a video;
# - a week no longer held loses its waiting work and is not tried again;
# - `cache warning:` lines reach <week>.cache.log, a failed attempt's too.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=render-calls.sh
source "$HERE/render-calls.sh"

SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT
FAKE=$SCRATCH/fake
APP_DIR=$SCRATCH/app
TSX=$FAKE/tsx
mkdir -p "$FAKE" "$APP_DIR"

# The fake recorder: `tsx tests/record/call.ts … --week-file <f> --out <w>`.
# It writes one step into <w>/<week>/steps (saying `reused step` if the step
# was already there), then fails if $FAKE/fail-<week> exists, else writes the
# video and a cache folder. $FAKE/warn makes it say a cache warning;
# $FAKE/bulk-<week> makes its step that many MB. Each run is counted in
# $FAKE/runs.
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
  exit 1
fi
mkdir -p "$dir/cache"
echo intermediate > "$dir/cache/frame"
echo "video of $week" > "$dir/call.mp4"
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
  rm -f "$FAKE"/fail-* "$FAKE"/bulk-* "$FAKE/warn" "$FAKE/runs"
}

pass() {
  run_job call > "$SCRATCH/pass.out" 2>&1
}

runs_of() {
  grep -cx "$1" "$FAKE/runs" 2>/dev/null || true
}

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

if [ "$failures" -ne 0 ]; then
  echo "$failures of the render worker's guarantees failed"
  exit 1
fi
echo "the render worker's guarantees hold"
