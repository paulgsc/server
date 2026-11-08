#!/usr/bin/env bash
# Home's render worker (GWB10, paulgsc/some-ui#2084), with two jobs, one
# loop: the weekly call's (paulgsc/some-ui#2635, ST36) and Playoff Delta's
# (FEED4, paulgsc/some-ui#2647). infra/compose/call-render.yml runs it.
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
#
# A Playoff Delta key is <season>-<team>-<NN> (2026-SF-08, NN 00 the legend);
# file_host writes only fills of a voiced version (pd_fills::render), so a
# fill written for the owner's own voice is never tried here, never failed.
#
# One episode at a time, the call's first, each job's oldest first. An
# episode that fails is left as it was and tried again next pass; a finished
# one is moved into place whole, so a folder in a job's renders folder is
# always a finished video. Nothing here deletes a finished video: the owner
# lets a week go from My data, once its YouTube link is recorded, and
# file_host deletes it (`call_weeks::videos`), first taking the week's file
# out of /weeks so it is never rendered again (paulgsc/some-ui#2661).
#
# What it reports, per job, for file_host to read (`call_weeks::render`,
# `pd_fills::render`; the CALL dashboard and the Call* and PlayoffDelta*
# alerts): <status>/last-pass, the Unix time each pass started, and
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

EVERY=3600
# tsx (a root devDependency of some-ui), not through pnpm: Corepack would fetch
# pnpm at run time, and nothing here reaches the internet.
APP_DIR=/opt/some-ui/apps/www
TSX=/opt/some-ui/node_modules/.bin/tsx
# What both recorders take. /brand is home's folder of the owner's files a
# series names (its music bed), read-only (call-render.yml); a series that
# names none never reads it.
COMMON=(--app http://call-stage --voices kokoro --speaker http://kokoro:8880 --brand /brand
  --max-video-minutes "$MAX_VIDEO_MINUTES" --max-video-mb "$MAX_VIDEO_MB" --cache-mb "$CACHE_MB")

JOBS=(call playoff-delta)

# The job's folders and video's name, as globals: INPUT, RENDERS, STATUS,
# WORK, VIDEO. The work folder is on the same drive as the videos, not the
# RAM disk: what a render writes on /tmp counts against the container's
# memory cap, beside ffmpeg, and moving a finished episode into RENDERS is
# one rename on the same filesystem.
use_job() {
  case $1 in
    call)
      INPUT=/weeks RENDERS=/renders/call STATUS=/renders/call-worker WORK=/renders/call-work VIDEO=call.mp4 ;;
    playoff-delta)
      INPUT=/episodes RENDERS=/renders/playoff-delta STATUS=/renders/playoff-delta-worker
      WORK=/renders/playoff-delta-work VIDEO=episode.mp4 ;;
  esac
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
  esac
}

# Whether the job still holds `key` (file_host takes an episode out of its
# input once the owner lets it go).
holds() {
  case $1 in
    call) [ -e "$INPUT/$2.json" ] ;;
    playoff-delta) [ -d "$INPUT/$2" ] ;;
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
  esac
}

# Where `record` leaves `key`'s finished folder: the call's recorder names it
# for the week's first day in --out, Playoff Delta's <season>-week-NN in it.
finished() {
  case $1 in
    call) echo "$WORK/$2" ;;
    playoff-delta) echo "$WORK/$2/${2%%-*}-week-${2##*-}" ;;
  esac
}

render_episode() {
  local job=$1 key=$2
  local log="$STATUS/.$key.log" made
  : > "$log" || return 1
  if ! mkdir -p "$WORK" 2>> "$log"; then
    echo "the work folder $WORK could not be made" | tee -a "$log"
    return 1
  fi
  record "$job" "$key" 2>&1 | tee -a "$log"
  local recorded=${PIPESTATUS[0]}
  if [ "$recorded" -ne 0 ]; then
    echo "the recorder exited $recorded" | tee -a "$log"
    return 1
  fi
  made=$(finished "$job" "$key")
  # The renderer's intermediates can be made again from what is kept.
  rm -rf "$made/cache"
  if ! { mkdir -p "$RENDERS" && mv "$made" "$RENDERS/$key"; } 2>> "$log"; then
    echo "the finished $job of $key could not be moved into $RENDERS" | tee -a "$log"
    return 1
  fi
  # What is left of Playoff Delta's --out folder is empty.
  rm -rf "${WORK:?}/$key"
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
prune_work() {
  local job=$1 current=$2 dir key
  for dir in "$WORK"/*/; do
    [ -d "$dir" ] || continue
    key=$(basename "$dir")
    if [ -e "$RENDERS/$key/$VIDEO" ] || ! holds "$job" "$key"; then
      rm -rf "$dir"
    fi
  done
  while [ "$(du -sm "$WORK" 2>/dev/null | cut -f1)" -gt "$WORK_MAX_MB" ]; do
    key=$(ls -1 "$WORK" | grep -vx "$current" | sort | head -n1)
    [ -n "$key" ] || break
    rm -rf "${WORK:?}/$key"
    echo "cache warning: removed the waiting work of $key: all of the $job's work passed the policy's $WORK_MAX_MB MB" |
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

run_job() {
  local job=$1 key
  use_job "$job"
  mkdir -p "$STATUS" && put last-pass "$(date +%s)"
  for key in $(episodes "$job"); do
    [ -e "$RENDERS/$key/$VIDEO" ] && continue
    echo "rendering the $job of $key"
    mkdir -p "$WORK" && prune_work "$job" "$key"
    if render_episode "$job" "$key"; then
      keep_cache_warnings "$key"
      rm -f "$STATUS/.$key.log" "$STATUS/$key.failed.log"
      echo "rendered the $job of $key"
    else
      keep_cache_warnings "$key"
      mv -f "$STATUS/.$key.log" "$STATUS/$key.failed.log"
      echo "the $job of $key did not render; it is tried again in an hour" >&2
    fi
  done
}

while true; do
  for job in "${JOBS[@]}"; do
    run_job "$job"
  done
  sleep "$EVERY"
done
