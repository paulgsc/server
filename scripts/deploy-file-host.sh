#!/usr/bin/env bash
set -Eeuo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly STACK_NAME="${STACK_NAME:-file-host}"
readonly SERVICE="${STACK_NAME}_file-host"
readonly TIMEOUT_SECONDS="${DEPLOY_TIMEOUT_SECONDS:-300}"

# root docker-compose.yml (via its own `include:`) stays the one place every
# compose fragment is enumerated — this script does not keep a second list
# of fragment paths to hand-sync with it. It renders straight from that file
# plus the swarm-only overlay, then narrows the result to file-host's own
# service subgraph by name. Keep this list in sync with
# infra/compose/file_host.yml's `depends_on`, not with which fragment files
# exist.
readonly ROLLOUT_SERVICES=(file-host redis nats)

usage() {
	cat <<'EOF'
Usage: scripts/deploy-file-host.sh IMAGE

Roll file_host without removing the serving task first. IMAGE must be an
immutable tag or digest (for example pgathondu/server:git-0123abcd or
pgathondu/server@sha256:...). The Swarm update starts one replacement, waits
for its health check, and only then drains the corresponding old task.
EOF
}

[[ $# -eq 1 ]] || { usage >&2; exit 64; }
IMAGE=$1

# Check the tag/digest on the *last path segment* only: a registry address
# can carry its own `host:port`, which also contains a colon but is not a
# tag separator (e.g. registry.example:5000/team/file-host has no tag at
# all, and would otherwise slip past a check that just looks for any `:`).
image_ref=${IMAGE##*/}
if [[ "$IMAGE" != *@sha256:* && ( "$image_ref" == *:latest || "$image_ref" != *:* ) ]]; then
	echo "error: IMAGE must use an immutable non-latest tag or sha256 digest" >&2
	exit 64
fi

command -v docker >/dev/null || { echo 'error: docker is required' >&2; exit 69; }
# LocalNodeState is also "active" on a plain worker node, which cannot run
# `docker stack deploy`. ControlAvailable is true only on a manager with
# control access, which is what this actually requires.
[[ "$(docker info --format '{{.Swarm.ControlAvailable}}')" == true ]] || {
	echo 'error: this node is not an active Docker Swarm manager' >&2
	exit 69
}

# `docker stack deploy` parses Compose files with its own, older loader —
# it does not track the current Compose Specification the way
# `docker compose` does (docker/cli#2527 is still open at the time of
# writing), so syntax this repo's fragments use (the extended `env_file`
# mapping form, for one) is not guaranteed to parse there even though
# `docker compose config` accepts it fine. Render through the modern parser
# first and hand Swarm the fully-resolved, plain-field output instead of
# the source fragments, so the legacy loader never has to understand
# anything but basic scalars/lists/maps.
rendered=$(mktemp "${TMPDIR:-/tmp}/file-host-stack.XXXXXX.yml")
trap 'rm -f "$rendered"' EXIT

FILE_HOST_IMAGE="$IMAGE" REPO_ROOT="$ROOT" docker compose \
	--file "$ROOT/docker-compose.yml" \
	--file "$ROOT/infra/compose/file_host.swarm.yml" \
	config "${ROLLOUT_SERVICES[@]}" >"$rendered"

echo "Deploying $IMAGE to $SERVICE (start-first, automatic rollback)..."

# `docker stack deploy --detach=false` already blocks on Swarm's own update
# state machine and returns non-zero on failed convergence or rollback — no
# need to hand-roll that by polling and parsing `docker service ls` output.
# `timeout` supplies the bounded wait Docker's own blocking mode doesn't have
# a flag for. `--resolve-image changed` overrides the CLI's own default
# (`always`): this stack also carries redis/nats for network topology
# reasons, and the default would force-repin their `:latest` tags on every
# file-host rollout.
set +e
timeout "$TIMEOUT_SECONDS" docker stack deploy \
	--detach=false \
	--with-registry-auth --resolve-image changed --prune \
	--compose-file "$rendered" \
	"$STACK_NAME"
status=$?
set -e

if [[ $status -ne 0 ]]; then
	if [[ $status -eq 124 ]]; then
		echo "error: deployment did not converge within ${TIMEOUT_SECONDS}s" >&2
	else
		echo "error: deployment failed or rolled back (exit $status)" >&2
	fi
	docker service ps --no-trunc "$SERVICE" >&2 || true
	exit 1
fi

echo "Deployment complete: $SERVICE is serving $IMAGE"
