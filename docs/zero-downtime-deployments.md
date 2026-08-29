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
environment, volumes, and healthcheck, and it stays the *only* such list:
the production rollout renders directly from `docker-compose.yml` itself,
not from a second, hand-maintained enumeration of fragment paths that could
drift out of sync with it.

```sh
docker compose --file docker-compose.yml --file infra/compose/file_host.swarm.yml \
  config file-host redis nats
```

* `docker-compose.yml` — unchanged, the same file `docker compose up` reads
  locally, `include:` list and all.
* `infra/compose/file_host.swarm.yml` — a small overlay that adds only what
  Swarm needs: replicas, `endpoint_mode: vip`, `update_config`/
  `rollback_config` (start-first, automatic rollback). It carries no image,
  environment, volume, or network config of its own — those stay defined
  exactly once, in `file_host.yml` and `base.yml`, so the two paths cannot
  drift apart.
* `config file-host redis nats` — `docker compose config` accepts a list of
  service names and narrows its output (services, and the networks/volumes
  they actually use) to just those, still resolved from the full merged
  project. `redis` and `nats` are named because they're file-host's actual
  `depends_on` graph in `file_host.yml`; if that ever changes, this is the
  one place to update it, and it names *services*, not files — adding,
  splitting, or renaming a fragment under `infra/compose/` needs no
  corresponding change here.

`scripts/deploy-file-host.sh` renders this filtered view through
`docker compose config` (the same command the `infra_ci` job validates it
with) and hands the fully-resolved result to `docker stack deploy`, rather
than letting Swarm's own, older compose-file parser merge and interpret
`docker-compose.yml` itself — see "Why a render step" below for why that
distinction matters.

`orchestrator`, `ollama`, and the monitoring/analytics services are
deliberately left out of the filtered set: including them would buy nothing
(Swarm's own defaults already behave like `restart: unless-stopped` for a
single-replica service) while pulling unrelated services into file-host's
blast radius. They're still part of the one root manifest — just not part
of this rollout's service subgraph.

`scripts/deploy-file-host.sh` is the only supported entry point that
composes `file_host.swarm.yml` this way. Do not `docker stack deploy` it
directly — see the header comment in that file for why.

## Why the network changes

A Swarm service's VIP routing mesh requires an overlay-scoped network. A
network can't be both a local `bridge` (what `docker compose up` used to
create) and `overlay` under the same name on one host, so `base.yml` now
declares `monitoring-network` as `overlay, attachable: true` unconditionally
— for the dev stack too, not just the Swarm rollout. The practical effect:
every environment needs `docker swarm init` run once before `docker compose
up` can create this network. Nothing else about local dev changes — a
single-node swarm with no services deployed to it costs nothing, and plain
`docker compose up` still ignores every `deploy:` field except
`resources`, exactly as before.

Upgrading an existing checkout: if `shared-dev-network` already exists as a
`bridge` network from before this change, remove it once
(`docker network rm shared-dev-network`, after stopping whatever is
using it) so the next `docker compose up` can recreate it as `overlay`.

`file_host.yml` also drops its `container_name`: Compose refuses to combine
a fixed container name with `deploy.replicas > 1`, since two replicas can't
share one name. `infra/prometheus/*.yml` and
`scripts/check_scrape_inventory.py` target `file-host-server:3000` by that
name, so it's declared as a network alias on the same service instead — one
that resolves to the single container under plain `docker compose up` and
to the VIP under the Swarm rollout, without the scrape-inventory contract
needing to change. (An earlier draft restored `container_name` as an alias
from inside `file_host.swarm.yml` using Compose's `!reset` merge tag — moved
into `file_host.yml` directly instead, since `!reset`/`!override` are
documented as unreliable across a multi-file merge, and, per the section
below, this file's alias would go through `docker stack deploy`'s parser
without the benefit of `docker compose`'s merge logic anyway.)

## Why a render step

`docker stack deploy` parses and merges its `--compose-file` arguments with
its own, older loader, not `docker compose`'s — `docker/cli#2527`, asking
Docker to unify the two, is still open. That loader isn't guaranteed to
understand syntax `docker compose config` accepts fine — `file_host.yml`'s
extended `env_file` mapping form (`path:`/`required:`), among other things —
so handing Swarm `docker-compose.yml` directly risks a parse failure in
production that `infra_ci`'s `docker compose config --quiet` check, which
uses the newer parser, would never catch.

`scripts/deploy-file-host.sh` avoids the gap instead of hoping it doesn't
matter: it renders through `docker compose config` first, which resolves
`env_file` into a plain `environment:` map and expands every
shorthand into its long form, then feeds *that* fully-resolved document —
plain scalars, lists, and maps only, nothing Compose-Specification-only left
for the older loader to trip on — to `docker stack deploy` in a single
`--compose-file`. `infra_ci`'s config-resolution check validates this same
render, so it now doubles as a check against exactly what the script
deploys.

## Deploy

Publish an immutable tag, then run:

```sh
scripts/deploy-file-host.sh pgathondu/server:git-0123abcd
# or: scripts/deploy-file-host.sh pgathondu/server@sha256:...
# or: make rollout IMAGE=pgathondu/server:git-0123abcd
```

The script rejects `latest`/untagged images (checking the last path segment
specifically, so a registry address's own `host:port` colon isn't mistaken
for a tag separator) and checks that this node has Swarm manager control
(`.Swarm.ControlAvailable`, not just `.Swarm.LocalNodeState` — a plain
worker node reports `active` too, but can't run `docker stack deploy`) —
then renders and deploys, blocking until Swarm reports every replica
converged, or exiting non-zero on rollback or timeout.

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

1. **Policy, not orchestration** — rejects `latest`/untagged images and
   confirms this node can actually run `docker stack deploy` before trying.
   Neither is a Swarm capability; both are this repo's own rules about what
   counts as a safe image reference and a sane starting state.
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
