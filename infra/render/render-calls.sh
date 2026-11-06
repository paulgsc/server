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
set -uo pipefail

WEEKS=/weeks
RENDERS=/renders/call
WORK=/tmp/render
EVERY=3600
# tsx (a root devDependency of some-ui), not through pnpm: Corepack would fetch
# pnpm at run time, and nothing here reaches the internet.
APP_DIR=/opt/some-ui/apps/www
RECORDER=(/opt/some-ui/node_modules/.bin/tsx tests/record/call.ts
  --app http://call-stage --voices kokoro --speaker http://kokoro:8880)

render_week() {
  local file=$1 from=$2
  rm -rf "$WORK" && mkdir -p "$WORK" || return 1
  (cd "$APP_DIR" && "${RECORDER[@]}" --week-file "$file" --out "$WORK") || return 1
  # The renderer's intermediates can be made again from what is kept.
  rm -rf "$WORK/$from/cache"
  mkdir -p "$RENDERS" && rm -rf "$RENDERS/.$from" || return 1
  cp -r "$WORK/$from" "$RENDERS/.$from" && mv "$RENDERS/.$from" "$RENDERS/$from"
}

while true; do
  for file in "$WEEKS"/*.json; do
    [ -e "$file" ] || continue
    from=$(basename "$file" .json)
    [[ "$from" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || continue
    [ -e "$RENDERS/$from/call.mp4" ] && continue
    echo "rendering the call of $from"
    if render_week "$file" "$from"; then
      echo "rendered the call of $from"
    else
      echo "the call of $from did not render; it is tried again in an hour" >&2
    fi
  done
  sleep "$EVERY"
done
