# Zero-downtime `file_host` deployments

Production deployments use Docker Swarm's rolling service update rather than
`docker compose down`, `pull`, and `up`. The service has two replicas behind a
VIP. An update is **start-first** and proceeds one task at a time: a replacement
must become healthy before Swarm removes an old task. A failed replacement
automatically rolls back. Caddy separately checks `/ready`, so a live process
whose SQLite, NATS, or Redis dependency is unavailable receives no new traffic.

## One-time host setup

The application, Caddy, Redis, NATS, and Prometheus must share an attachable
overlay network. Create it on a Swarm manager and attach the dependency stack:

```sh
docker swarm init                         # only on a new, single-node swarm
docker network create --driver overlay --attachable shared-dev-network
```

Do not replace an existing bridge network in place while the application is
serving. Create an overlay with another name, migrate the dependency services,
and set `DEPLOY_NETWORK` during the transition.

## Deploy

Publish an immutable tag, then run:

```sh
scripts/deploy-file-host.sh pgathondu/server:git-0123abcd
# or: scripts/deploy-file-host.sh pgathondu/server@sha256:...
```

The script refuses `latest`, verifies that Swarm and the overlay network are
available, deploys the stack, and waits until every desired replica is running
and the update is complete. It exits non-zero on timeout or rollback. The
normal image health check is the admission gate; Caddy's `/ready` probe is the
traffic gate.

`stop_grace_period` is two minutes so upgraded WebSocket connections get a
drain window. Clients still need reconnect logic because a WebSocket is pinned
to the task that accepted it; rolling orchestration prevents an HTTP outage but
cannot move an already-upgraded TCP connection between processes.

## Operational rules

* Never deploy production with `docker compose down`.
* Never deploy the mutable `latest` tag.
* Keep at least two replicas (`FILE_HOST_REPLICAS`, default `2`).
* Put migrations through backward-compatible expand/contract phases: old and
  new tasks overlap during every rollout.
* Redis, NATS, and the SQLite path are deliberately outside the application
  stack, so an application image update cannot restart its dependencies.
