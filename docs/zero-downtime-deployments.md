# Zero-downtime `file_host` deployments

Production rollouts of `file_host` use Docker Swarm's rolling service update
instead of `docker compose down && pull && up`. The service runs two
replicas behind a VIP. An update is **start-first**: a replacement task must
pass its health check before Swarm removes the old one. A failed replacement
rolls back automatically.

`file_host` is the only service that gets this treatment. Everything else in
this repo — `orchestrator`, `ollama`, `nats`, `redis`, and the
monitoring/analytics stack — is an internal service with no uptime SLA, so
it stays on plain `docker compose up` with `restart: unless-stopped`. Taking
one of them offline for a few seconds during a redeploy is a non-event; the
same is not true of `file_host`'s long-lived WebSocket connections (see the
[WebSocket Service SLA](../apps/servers/file_host/docs/sla/WebsSocket_Service_SLA.md)).

## One harness, not two

`docker-compose.yml` includes a set of fragments under `infra/compose/` —
`base.yml` for the shared network/volumes, then one file per service. That
`include:` list is the single source of truth for every service's image,
environment, volumes, and healthcheck. The production rollout reuses those
exact fragments rather than re-declaring `file-host` from scratch in a
parallel `infra/deploy/` tree:

* `infra/compose/base.yml`, `redis.yml`, `nats.yml`, `file_host.yml` — the
  same files `docker compose up` reads locally.
* `infra/compose/file_host.swarm.yml` — a small overlay, layered on top of
  the files above via `docker stack deploy`'s own multi-`--compose-file`
  merging, that adds only what Swarm needs: replicas, `endpoint_mode: vip`,
  `update_config`/`rollback_config` (start-first, automatic rollback), and
  an overlay network declaration. It carries no image, environment, or
  volume of its own — those stay defined exactly once, in `file_host.yml`,
  so the two paths cannot drift apart.

Only `redis` and `nats` ride along with `file-host` in this rollout — they
are its actual `depends_on` graph. `orchestrator`, `ollama`, and the
monitoring/analytics fragments are deliberately left out: including them
would buy nothing (Swarm's own defaults already behave like `restart:
unless-stopped` for a single-replica service) while pulling unrelated
services into file-host's blast radius.

`scripts/deploy-file-host.sh` is the only supported entry point that
composes these files. Do not `docker stack deploy` `file_host.swarm.yml`
directly — see the header comment in that file for why.

## Why the network changes

`base.yml` declares `monitoring-network` as a local `bridge` network so
`docker compose up` has zero prerequisites in dev. A Swarm service's VIP
routing mesh requires an overlay-scoped network, so `file_host.swarm.yml`
re-declares the same network `name:` with `driver: overlay`. Re-using the
same name (rather than a stack-scoped one) is what keeps `redis` and `nats`
reachable at the DNS names `file_host.yml` already expects.

The one sharp edge: a network name can't be both a local bridge and an
overlay network on the same host at once. If `docker compose up` already
created `shared-dev-network` as a bridge network, `docker stack deploy` will
refuse to reconcile it — `scripts/deploy-file-host.sh` checks for and
fails loudly on exactly that conflict before ever calling `docker stack
deploy`, rather than surfacing Docker's own error several layers down.

`file_host.swarm.yml` also has to reset `container_name` (`!reset null`):
Compose refuses to combine a fixed container name with `deploy.replicas >
1`, since two replicas can't share one name. `infra/prometheus/*.yml` and
`scripts/check_scrape_inventory.py` target `file-host-server:3000` by that
container name, so it's restored as a network alias on the same service
instead — the existing scrape-inventory contract doesn't need to change.

## Deploy

Publish an immutable tag, then run:

```sh
scripts/deploy-file-host.sh pgathondu/server:git-0123abcd
# or: scripts/deploy-file-host.sh pgathondu/server@sha256:...
# or: make rollout IMAGE=pgathondu/server:git-0123abcd
```

The script rejects `latest`, checks that this node is an active Swarm
manager, and checks that `shared-dev-network` isn't already a conflicting
bridge network — then deploys and blocks until Swarm reports every replica
converged, or exits non-zero on rollback or timeout.

`stop_grace_period` is two minutes so an upgraded WebSocket connection gets
a drain window. Clients still need reconnect logic: a WebSocket is pinned to
the task that accepted it, so rolling orchestration prevents an HTTP outage
but cannot migrate an already-upgraded TCP connection between processes.

## Why a script at all

Docker Swarm owns every actual deployment decision here — health gating,
start-first replacement, and rollback are Swarm's, not this repo's. What
Swarm's CLI doesn't give you is a single command that (a) enforces this
repo's own deployment policy and (b) turns its asynchronous update into a
deterministic pass/fail result. Concretely, `scripts/deploy-file-host.sh`
does two things a bare `docker stack deploy` doesn't:

1. **Policy, not orchestration** — rejects `latest`/untagged images, and
   fails before touching Swarm if the network is in a state `docker stack
   deploy` would reject anyway. Neither is a Swarm capability; both are this
   repo's own rules about what counts as a safe image reference and a sane
   starting state.
2. **A bounded, truthful exit code** — `docker stack deploy --detach=false`
   already blocks on Swarm's real update state machine and exits non-zero on
   a failed convergence or rollback (Swarm's default, `--detach=true`,
   returns immediately after submitting the update and tells you nothing
   about whether it actually succeeded). `timeout` wraps that call because
   `--detach=false` has no bound of its own — without one, a stuck rollout
   would hang a deploy or a CI job forever instead of failing.

An earlier draft of this script (see the PR history) hand-rolled that second
part: a `sleep 2` polling loop that scraped `docker service ls`'s
tab-formatted `Replicas` column with shell parameter expansion. That is
exactly the kind of script-as-ersatz-orchestrator this file should not
become — `--detach=false` already does the same job atomically, against
Swarm's actual state rather than a periodic guess at it, so the loop was
removed rather than kept "just in case."

If Docker ever exposes richer native policy hooks (an immutable-tag
constraint, for instance), the corresponding check here should be deleted,
not kept for redundancy — the script's only job is to cover what Swarm
doesn't.

## Operational rules

* Never deploy production with `docker compose down`.
* Never deploy the mutable `latest` tag.
* Keep at least two replicas (`FILE_HOST_REPLICAS`, default `2`).
* Put migrations through backward-compatible expand/contract phases: old and
  new tasks overlap during every rollout.
* Redis and NATS are included in this rollout for network-topology reasons
  only (see above) — an application image update never touches them
  (`--resolve-image changed`, not `always`).

## Known gap: the Caddy proxy

`infra/Caddyfile` is not touched by this change. `docker-compose.yml`
comments out `infra/compose/caddy.yml` because Caddy runs at the NixOS
system level in production, outside Docker entirely — which means it isn't
attached to the overlay network `file-host`'s VIP lives on, and a
host-level process can't resolve a Swarm-internal service DNS name. Pointing
Caddy at the VIP (health-checked upstream, retry/lb tuning) needs whatever
mechanism actually joins that host-level Caddy to Docker's network, or a
published/ingress port on the host instead of a VIP-only DNS name — neither
of which lives in this repository. That wiring needs sign-off from whoever
maintains the NixOS host config before it's safe to change.
