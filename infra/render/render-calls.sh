#!/usr/bin/env bash
# Home's render worker, the weekly call's job (paulgsc/some-ui#2635, ST36;
# GWB10, paulgsc/some-ui#2084). infra/compose/call-render.yml runs it.
#
# Each pass, for each week in /weeks (file_host writes every week it holds
# there, `call_weeks::RENDER_FOLDER`) with no video in /renders/call/<from>/,
# it records the call's stage (`call-stage`, the pinned home-network app) with
# each scene said by home's speaker (`kokoro`), renders it, and keeps the
# result: the video, its takes and stills, its cut list and the week it was
# made from. The same week gives the same files.
#
# One week at a time, oldest first. A week that fails is left as it was and
# tried again next pass; a finished one is moved into place whole, so a
# folder in /renders/call/ is always a finished video. Nothing here deletes a
# finished video: the owner lets a week go from My data, once its YouTube
# link is recorded, and file_host deletes it (`call_weeks::videos`), first
# taking the week's file out of /weeks so it is never rendered again
# (paulgsc/some-ui#2661).
#
# What it reports, for file_host to read (`call_weeks::render`; the CALL
# dashboard and the Call* alerts): /renders/call-worker/last-pass, the Unix
# time each pass started, and /renders/call-worker/<from>.failed.log, the
# whole output of a week's last failed attempt, removed once it renders.
# The same output is on stdout too, for `docker logs render`.
#
# A waiting week's work, its step cache included, is kept between attempts
# in $WORK/<from>, so a recorder that keeps its steps (paulgsc/some-ui#2658)
# reruns only what changed, and leaves with the finished week. What a render
# may take is policy; nothing past it is silent: a step not cached (a
# `cache warning:` line from the recorder) is kept in
# /renders/call-worker/<week>.cache.log, and a week's work removed past the
# total cap in work.cache.log, both read by file_host (CallRenderCacheFull).
set -uo pipefail

# Policy (paulgsc/some-ui#2635, the owner's caps). The recorder refuses a
# video longer than MAX_VIDEO_MINUTES (from its words, before anything is
# said) or larger than MAX_VIDEO_MB (removed once made), a failed attempt
# that names the cap; it stops keeping a week's steps past CACHE_MB, with a
# `cache warning:` line. All waiting weeks' work together, $WORK, is held to
# WORK_MAX_MB: past it, the oldest other week's work is removed (its retry
# starts over).
MAX_VIDEO_MINUTES=15
MAX_VIDEO_MB=500
CACHE_MB=2048
WORK_MAX_MB=4096

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
  --app http://call-stage --voices kokoro --speaker http://kokoro:8880
  --max-video-minutes "$MAX_VIDEO_MINUTES" --max-video-mb "$MAX_VIDEO_MB" --cache-mb "$CACHE_MB")

render_week() {
  local file=$1 from=$2
  local log="$STATUS/.$from.log"
  : > "$log" || return 1
  if ! mkdir -p "$WORK" 2>> "$log"; then
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

# The `cache warning:` lines of `from`'s last attempt, kept where file_host
# reads them; nothing is written when there were none.
keep_cache_warnings() {
  local from=$1 log="$STATUS/.$1.log" warned="$STATUS/.$1.cache.log"
  [ -e "$log" ] || return 0
  if grep '^cache warning: ' "$log" > "$warned"; then
    mv "$warned" "$STATUS/$from.cache.log"
  else
    rm -f "$warned"
  fi
}

# Before `current` renders: the work of a week that is no longer waiting
# (rendered, or no longer held) goes, and past WORK_MAX_MB the oldest other
# waiting week's work goes too, each said as a cache warning in
# work.cache.log (a week's own warnings are rewritten after each attempt).
prune_work() {
  local current=$1 dir from
  for dir in "$WORK"/*/; do
    [ -d "$dir" ] || continue
    from=$(basename "$dir")
    if [ -e "$RENDERS/$from/call.mp4" ] || [ ! -e "$WEEKS/$from.json" ]; then
      rm -rf "$dir"
    fi
  done
  while [ "$(du -sm "$WORK" 2>/dev/null | cut -f1)" -gt "$WORK_MAX_MB" ]; do
    from=$(ls -1 "$WORK" | grep -vx "$current" | sort | head -n1)
    [ -n "$from" ] || break
    rm -rf "${WORK:?}/$from"
    echo "cache warning: removed the waiting work of $from: all weeks' work passed the policy's $WORK_MAX_MB MB" |
      tee -a "$STATUS/work.cache.log"
    # Its last hundred lines only: a warning is news, not a record.
    tail -n 100 "$STATUS/work.cache.log" > "$STATUS/.work.cache.log" &&
      mv "$STATUS/.work.cache.log" "$STATUS/work.cache.log"
  done
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
    mkdir -p "$WORK" && prune_work "$from"
    if render_week "$file" "$from"; then
      keep_cache_warnings "$from"
      rm -f "$STATUS/.$from.log" "$STATUS/$from.failed.log"
      echo "rendered the call of $from"
    else
      keep_cache_warnings "$from"
      mv -f "$STATUS/.$from.log" "$STATUS/$from.failed.log"
      echo "the call of $from did not render; it is tried again in an hour" >&2
    fi
  done
  sleep "$EVERY"
done
