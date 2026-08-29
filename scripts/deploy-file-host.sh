#!/usr/bin/env bash
set -Eeuo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly STACK_FILE="$ROOT/infra/deploy/file-host-stack.yml"
readonly STACK_NAME="${STACK_NAME:-file-host}"
readonly SERVICE="${STACK_NAME}_file-host"
readonly DEPLOY_NETWORK="${DEPLOY_NETWORK:-shared-dev-network}"
readonly TIMEOUT_SECONDS="${DEPLOY_TIMEOUT_SECONDS:-300}"

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

network_driver=$(docker network inspect --format '{{.Driver}}' "$DEPLOY_NETWORK" 2>/dev/null || true)
[[ "$network_driver" == overlay ]] || {
	echo "error: $DEPLOY_NETWORK must exist and use the overlay driver (found: ${network_driver:-missing})" >&2
	exit 69
}

export FILE_HOST_IMAGE="$IMAGE" REPO_ROOT="$ROOT" DEPLOY_NETWORK
echo "Deploying $IMAGE to $SERVICE (start-first, automatic rollback)..."
docker stack deploy --with-registry-auth --resolve-image always --prune \
	--compose-file "$STACK_FILE" "$STACK_NAME"

deadline=$((SECONDS + TIMEOUT_SECONDS))
while (( SECONDS < deadline )); do
	update_state=$(docker service inspect --format '{{if .UpdateStatus}}{{.UpdateStatus.State}}{{else}}completed{{end}}' "$SERVICE")
	replicas=$(docker service ls --filter "name=^${SERVICE}$" --format '{{.Replicas}}')
	wanted=${replicas#*/}
	running=${replicas%/*}

	case "$update_state" in
		rollback_started|rollback_paused|rollback_completed|paused)
			echo "error: deployment entered $update_state; the previous image remains selected" >&2
			docker service ps --no-trunc "$SERVICE" >&2
			exit 1
			;;
	esac

	if [[ "$update_state" == completed && -n "$wanted" && "$running" == "$wanted" ]]; then
		echo "Deployment complete: $replicas replicas are serving $IMAGE"
		exit 0
	fi

	sleep 2
done

echo "error: deployment did not converge within ${TIMEOUT_SECONDS}s" >&2
docker service ps --no-trunc "$SERVICE" >&2
exit 1
