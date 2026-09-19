#!/usr/bin/env bash
# Emits hostdir_usage_bytes{target="..."} and
# hostdir_usage_last_run_timestamp_seconds for node_exporter's textfile
# collector — see infra/grafana/dashboards/lib/forensic-panels.libsonnet's
# hostDirDiskUsage panel. cadvisor's container_fs_usage_bytes only sees a
# *running* container's own writable layer: it never sees the cargo
# registry/git caches or the workspace target/ dir on the host, and never
# the parts of Docker's own data-root (build cache, dangling images, unused
# volumes) that no running container's own usage would show either. This
# script closes that gap with a plain `du`.
#
# Run by the disk-usage-exporter sidecar in infra/compose/monitoring.yml, on
# a loop — this script itself is single-shot and idempotent, so it's also
# safe to invoke by hand, or from a systemd timer, if this ever moves off
# docker-compose.
set -euo pipefail

OUTPUT_DIR="${TEXTFILE_COLLECTOR_DIR:-/textfile-collector}"
OUTPUT_FILE="$OUTPUT_DIR/disk_usage.prom"
TMP_FILE="$OUTPUT_DIR/.disk_usage.prom.tmp.$$"

# name:path pairs — bind-mounted read-only by the disk-usage-exporter
# service in infra/compose/monitoring.yml.
TARGETS=(
	"cargo_registry:/mnt/cargo/registry"
	"cargo_git:/mnt/cargo/git"
	"cargo_target:/mnt/workspace-target"
	"docker_data_root:/mnt/docker"
)

dir_size_bytes() {
	local path="$1"
	if [ -d "$path" ]; then
		du -sb "$path" 2>/dev/null | awk '{print $1}'
	else
		echo 0
	fi
}

# Written to a per-run tmp file and renamed into place — a `mv` on the same
# filesystem is atomic, so node_exporter's textfile collector never reads a
# half-written scrape.
{
	echo "# HELP hostdir_usage_bytes Bytes used by a tracked host directory (du -sb), refreshed periodically by scripts/disk-usage-textfile.sh."
	echo "# TYPE hostdir_usage_bytes gauge"
	for entry in "${TARGETS[@]}"; do
		name="${entry%%:*}"
		path="${entry#*:}"
		size="$(dir_size_bytes "$path")"
		echo "hostdir_usage_bytes{target=\"${name}\"} ${size}"
	done
	echo "# HELP hostdir_usage_last_run_timestamp_seconds Unix time this script last completed a full pass."
	echo "# TYPE hostdir_usage_last_run_timestamp_seconds gauge"
	echo "hostdir_usage_last_run_timestamp_seconds $(date +%s)"
} >"$TMP_FILE"

mv "$TMP_FILE" "$OUTPUT_FILE"
