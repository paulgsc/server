#!/usr/bin/env bash
# Home's render worker, the weekly call's job (paulgsc/some-ui#2635, ST36;
# GWB10, paulgsc/some-ui#2084). infra/compose/call-render.yml runs it.
#
# Each pass, for each week in /weeks (file_host writes every week it holds
# there, `call_weeks::RENDER_FOLDER`) with no video in /renders/call/<from>/,
# it records the call's stage (`call-stage`, the pinned home-network app) with
# each scene said by home's speaker (`kokoro`), renders it, and keeps the
# result for good: the video, its takes and stills, its cut list and the week
# it was made from. The same week gives the same files.
#
# One week at a time, oldest first. A week that fails is left as it was and
# tried again next pass; a finished one is moved into place whole, so a
# folder in /renders/call/ is always a finished video. Nothing here deletes a
# finished video.
#
# What it reports, for file_host to read (`call_weeks::render`; the CALL
# dashboard and the Call* alerts): /renders/call-worker/last-pass, the Unix
# time each pass started, and /renders/call-worker/<from>.failed.log, the
# whole output of a week's last failed attempt, removed once it renders.
# The same output is on stdout too, for `docker logs render`.
set -uo pipefail

WEEKS=/weeks
RENDERS=/renders/call
STATUS=/renders/call-worker
# The work folder is on the same drive as the videos, not the RAM disk:
# what a render writes on /tmp counts against the container's memory cap,
# beside ffmpeg, and moving a finished week from here into $RENDERS is one
# rename on the same filesystem.
WORK=/renders/call-work
EVERY=3600
# tsx (a root devDependency of some-ui), not through pnpm: Corepack would fetch
# pnpm at run time, and nothing here reaches the internet.
APP_DIR=/opt/some-ui/apps/www
RECORDER=(/opt/some-ui/node_modules/.bin/tsx tests/record/call.ts
  --app http://call-stage --voices kokoro --speaker http://kokoro:8880)

render_week() {
  local file=$1 from=$2
  local log="$STATUS/.$from.log"
  : > "$log" || return 1
  if ! { rm -rf "$WORK" && mkdir -p "$WORK"; } 2>> "$log"; then
    echo "the work folder $WORK could not be made" | tee -a "$log"
    return 1
  fi
  (cd "$APP_DIR" && "${RECORDER[@]}" --week-file "$file" --out "$WORK" 2>&1) | tee -a "$log"
  local recorded=${PIPESTATUS[0]}
  if [ "$recorded" -ne 0 ]; then
    echo "the recorder exited $recorded" | tee -a "$log"
    return 1
  fi
  # The renderer's intermediates can be made again from what is kept.
  rm -rf "$WORK/$from/cache"
  if ! { mkdir -p "$RENDERS" && mv "$WORK/$from" "$RENDERS/$from"; } 2>> "$log"; then
    echo "the finished week could not be moved into $RENDERS" | tee -a "$log"
    return 1
  fi
}

# Written whole, so file_host never reads half a file.
put() {
  printf '%s\n' "$2" > "$STATUS/.$1" && mv "$STATUS/.$1" "$STATUS/$1"
}

while true; do
  mkdir -p "$STATUS" && put last-pass "$(date +%s)"
  for file in "$WEEKS"/*.json; do
    [ -e "$file" ] || continue
    from=$(basename "$file" .json)
    [[ "$from" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || continue
    [ -e "$RENDERS/$from/call.mp4" ] && continue
    echo "rendering the call of $from"
    if render_week "$file" "$from"; then
      rm -f "$STATUS/.$from.log" "$STATUS/$from.failed.log"
      echo "rendered the call of $from"
    else
      mv -f "$STATUS/.$from.log" "$STATUS/$from.failed.log"
      echo "the call of $from did not render; it is tried again in an hour" >&2
    fi
  done
  sleep "$EVERY"
done
