#!/usr/bin/env bash
set -Eeuo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly COMPOSE_DIR="$ROOT/infra/compose"
readonly STACK_NAME="${STACK_NAME:-file-host}"
readonly SERVICE="${STACK_NAME}_file-host"
readonly TIMEOUT_SECONDS="${DEPLOY_TIMEOUT_SECONDS:-300}"

# The production rollout is composed from the same fragment files
# docker-compose.yml already uses for local dev: base.yml (network/volumes),
# file-host's two actual runtime dependencies (redis, nats), file_host.yml
# itself, and a swarm-only overlay that adds replicas/rolling-update/VIP
# semantics on top. This is deliberately a *subset* of the same harness, not
# a parallel definition — orchestrator/ollama/monitoring/analytics have no
# uptime SLA and stay on the plain `docker compose up` path untouched.
readonly COMPOSE_FILES=(
	"$COMPOSE_DIR/base.yml"
	"$COMPOSE_DIR/redis.yml"
	"$COMPOSE_DIR/nats.yml"
	"$COMPOSE_DIR/file_host.yml"
	"$COMPOSE_DIR/file_host.swarm.yml"
)

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
	"${COMPOSE_FILES[@]/#/--file=}" \
	config >"$rendered"

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
