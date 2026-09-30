#!/usr/bin/env bash
# Emits hostdir_usage_bytes{target="..."} and
# hostdir_usage_last_run_timestamp_seconds for node_exporter's textfile
# collector — see infra/grafana/dashboards/lib/forensic-panels.libsonnet's
# hostDirDiskUsage panel. cadvisor's container_fs_usage_bytes only sees a
# *running* container's own writable layer: it never sees the cargo
# registry/git caches or the workspace target/ dir, both host paths. This
# script closes that gap with a plain `du`.
#
# Deliberately doesn't also track Docker's own data-root — see
# infra/compose/monitoring.yml's disk-usage-exporter comment for why that
# needs a scoped Docker API call (docker-socket-proxy's /system/df
# equivalent), not raw filesystem access to a tree that holds every other
# container's full environment and every named volume's raw data.
#
# Run by the disk-usage-exporter sidecar in infra/compose/monitoring.yml, on
# a loop — this script itself is single-shot and idempotent, so it's also
# safe to invoke by hand, or from a systemd timer, if this ever moves off
# docker-compose.
set -euo pipefail

OUTPUT_DIR="${TEXTFILE_COLLECTOR_DIR:-/textfile-collector}"
OUTPUT_FILE="$OUTPUT_DIR/disk_usage.prom"
TMP_FILE="$OUTPUT_DIR/.disk_usage.prom.tmp.$$"

# Covers every early-exit path, not just the happy one — after a successful
# `mv` below this is already gone and `rm -f` is a no-op, but a `du` failure
# (see dir_size_bytes) used to abort mid-script under `set -e` and leave
# this behind forever.
trap 'rm -f "$TMP_FILE"' EXIT

# name:path pairs — bind-mounted read-only by the disk-usage-exporter
# service in infra/compose/monitoring.yml.
TARGETS=(
	"cargo_registry:/mnt/cargo/registry"
	"cargo_git:/mnt/cargo/git"
	"cargo_target:/mnt/workspace-target"
)

# Prints the directory's size, or 0 when it doesn't exist (an unset
# CARGO_HOME_PATH, or no git dependencies: see example.env). Fails, printing
# nothing, when `du` can't measure a directory that does exist.
dir_size_bytes() {
	local path="$1"
	local size
	if [ ! -d "$path" ]; then
		echo 0
		return 0
	fi
	# `-B1` (block-size 1, no `--apparent-size`), not `-b` — `-b` is GNU
	# du's shorthand for `--apparent-size --block-size=1`, which reports
	# a sparse file's logical length rather than the disk blocks it
	# actually occupies. A disk-usage panel should track the same
	# "space consumed" a filesystem would run out of, not a number that
	# can overstate it.
	#
	# Captured into a variable so nothing a failed `du` printed escapes into
	# the textfile. A failure (a file vanishing under a concurrent `cargo
	# build`/`cargo clean`, an I/O or permission error) is reported as a
	# failure, not as 0: a 0 would read on the dashboard as "empty".
	size="$(du -s -B1 "$path" 2>/dev/null | awk '{print $1}')" || return 1
	[ -n "$size" ] || return 1
	echo "$size"
}

# The previous pass's value of a line, for when this pass can't measure it.
previous() {
	[ -f "$OUTPUT_FILE" ] || return 0
	awk -v key="$1" '$1 == key { print $2 }' "$OUTPUT_FILE"
}

# Written to a per-run tmp file and renamed into place — a `mv` on the same
# filesystem is atomic, so node_exporter's textfile collector never reads a
# half-written scrape.
#
# A target `du` couldn't measure keeps its last measured value (or no line
# at all if it has never been measured), and the pass doesn't count as
# complete: hostdir_usage_last_run_timestamp_seconds keeps the last complete
# pass's time. So a directory that keeps failing shows its last good size
# while hostDirUsageStaleness climbs to red, instead of reading as empty
# beside a green scan age.
complete=1
{
	echo "# HELP hostdir_usage_bytes Bytes used by a tracked host directory (du -s -B1), refreshed periodically by scripts/disk-usage-textfile.sh."
	echo "# TYPE hostdir_usage_bytes gauge"
	for entry in "${TARGETS[@]}"; do
		name="${entry%%:*}"
		path="${entry#*:}"
		key="hostdir_usage_bytes{target=\"${name}\"}"
		if ! size="$(dir_size_bytes "$path")"; then
			complete=0
			size="$(previous "$key")"
		fi
		[ -z "$size" ] || echo "$key $size"
	done
	if [ "$complete" = 1 ]; then
		last_run="$(date +%s)"
	else
		last_run="$(previous hostdir_usage_last_run_timestamp_seconds)"
	fi
	echo "# HELP hostdir_usage_last_run_timestamp_seconds Unix time this script last measured every tracked directory."
	echo "# TYPE hostdir_usage_last_run_timestamp_seconds gauge"
	[ -z "$last_run" ] || echo "hostdir_usage_last_run_timestamp_seconds $last_run"
	# Read from the same env var the compose service's own sleep loop
	# uses (infra/compose/monitoring.yml), not hardcoded here too — so
	# hostDirUsageStaleness can judge staleness relative to whatever
	# interval is actually configured instead of a threshold hardcoded
	# against this default, which an operator raising the interval to
	# reduce du's traversal cost would silently invalidate.
	echo "# HELP hostdir_usage_scan_interval_seconds The configured interval between scans (DISK_USAGE_SCAN_INTERVAL)."
	echo "# TYPE hostdir_usage_scan_interval_seconds gauge"
	echo "hostdir_usage_scan_interval_seconds ${DISK_USAGE_SCAN_INTERVAL:-300}"
} >"$TMP_FILE"

mv "$TMP_FILE" "$OUTPUT_FILE"
