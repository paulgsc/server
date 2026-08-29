#!/usr/bin/env bash
set -Eeuo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly COMPOSE_DIR="$ROOT/infra/compose"
readonly STACK_NAME="${STACK_NAME:-file-host}"
readonly SERVICE="${STACK_NAME}_file-host"
readonly TIMEOUT_SECONDS="${DEPLOY_TIMEOUT_SECONDS:-300}"
readonly NETWORK_NAME="shared-dev-network"

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
if [[ "$IMAGE" == *:latest || "$IMAGE" != *@sha256:* && "$IMAGE" != *:* ]]; then
	echo "error: IMAGE must use an immutable non-latest tag or sha256 digest" >&2
	exit 64
fi

command -v docker >/dev/null || { echo 'error: docker is required' >&2; exit 69; }
[[ "$(docker info --format '{{.Swarm.LocalNodeState}}')" == active ]] || {
	echo 'error: this node is not an active Docker Swarm manager' >&2
	exit 69
}

# file_host.swarm.yml declares this network as overlay so replicas get
# VIP-based load balancing. If a plain `docker compose up` already created it
# as a local bridge network on this host, Docker refuses to reconcile the two
# under the same name — fail loudly here instead of letting `docker stack
# deploy` produce that error deep in its own output.
network_driver=$(docker network inspect --format '{{.Driver}}' "$NETWORK_NAME" 2>/dev/null || true)
if [[ -n "$network_driver" && "$network_driver" != overlay ]]; then
	echo "error: '$NETWORK_NAME' already exists with driver '$network_driver' (expected overlay)" >&2
	echo "       This is almost always 'docker compose up' having created it as a local bridge" >&2
	echo "       network. Remove it once nothing on the compose stack depends on it (docker" >&2
	echo "       network rm $NETWORK_NAME), or move redis/nats off the plain compose stack before" >&2
	echo "       deploying file-host under Swarm." >&2
	exit 69
fi

export FILE_HOST_IMAGE="$IMAGE" REPO_ROOT="$ROOT"
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
	"${COMPOSE_FILES[@]/#/--compose-file=}" \
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
