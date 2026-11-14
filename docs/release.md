# Home's release: one set of images, by digest

Home runs one release. `deploy/home/release.json` names the image of every
service compose starts there. Each entry is `repo:tag@sha256:…` once it is
published, and a few are held or not yet published (below). Epic #787 holds
the design and its record. Stories R1 (#788), R2 (#789), R3 (#790) and R4
(#793) built what this doc declares; **Planned** marks what is not built yet,
with its issue.

The owner's ask (2026-11-11): "There's only one way to deploy this. We need
all of them to be up". The deploy should know which versions go together,
not let any one image be pulled, built or forgotten on its own.

## Invariants

These are what a change to the release, its workflow, its scripts or the
compose files must keep. A change that would break one stops and is raised,
per CLAUDE.md, "Drift is loud, not silent".

- **RL1. One source.** Every `image:` compose starts on home is
  `${IMAGE_<SERVICE>:?…}`, and envctl sets those variables from the role's
  release and from nothing else: not `compose.sops.env`, and not the shell.
  `envctl check` refuses an `image:` written any other way, a service the
  release does not name, a release entry no service starts, and a malformed
  reference (`apps/envctl/src/release.rs`, R1).
- **RL2. Reconciled, not carried.** CI forms a release only by reconciling it
  with the sources of truth: main's image changesets and Docker Hub's tags
  (`scripts/release_lock.py`). A run holds no images of its own, and what
  started it is only a reason to look. So runs are idempotent and order-free.
  Any run computes the whole set, so a cancelled, dropped or reordered run
  loses nothing (R2). Runs go one at a time, so only main can move under one.
  A run forms the release again if main moved before it finished. When main
  already has the newest set, it closes a release pull request still open. Two review rounds on #804 found images lost by the shape
  that carried them, which is why this is written down.
- **RL3. Newest published, defined per image.** For each image the release
  follows (`SOURCES` in `release_lock.py`), "newest published" means exactly
  one thing:
  - **A server image:** the newest changeset on main that includes it, by its
    `source_sha`'s place in main's history, whose
    `pgathondu/<suffix>:<source_sha>` tag exists. A changeset whose publish
    failed falls back to the one before it.
  - **A some-ui image:** the most recently pushed tag of its release's shape
    (a whole commit for render), never `latest`.
- **RL4. Bytes, not names.** A reference CI writes always carries its digest.
  A tag alone can move under a release.
- **RL5. Held entries do not move.** An entry in `HELD` keeps whatever main
  says; none is held since R4 (#793) pinned `file-host` by digest, once the
  deploy applies whole releases. An entry the release does not follow keeps
  whatever main says too: third-party images until R7 (#796).
- **RL6. No approval of its own.** Forming the set adds no human step
  (C119). Each image keeps the one approval it has where it is published, and
  the release pull request auto-merges once its checks pass. **Planned:**
  until a `RELEASE_BOT_TOKEN` exists, the release pull request's runs wait
  for "Approve workflows to run" (docs/assurance.md), which is the owner's
  call.
- **RL7. A cross-repo image is followed only once its set can be checked.**
  render comes from some-ui and must match what server writes for it (#756:
  a www release without its matching render broke every preview). It is in
  `SOURCES` since R4 (#793): the set check (R3, #790) reads labels by digest,
  and with `file-host` pinned by digest it reads both ends of render's
  requirement, so the release pull request refuses a render that requires
  what its file_host does not provide. www is not run on home (call-stage
  went with #802), so it is in no release; an image that joins from some-ui
  later (resume-render, www: #853's server half) joins `SOURCES` only with
  its `requires` labels.
- **RL8. Every requirement is met inside the set.** Each image's
  `io.paulgsc.requires` is met by another image's `io.paulgsc.provides` in
  the same release ("Constraints", below). CI checks it from the registry by
  digest (`scripts/check_release.py`, Lint's `Release set` job, on every pull
  request that changes the release). A requirement whose name no readable
  image provides, while a server image has no digest, is a warning naming
  that image, not a failure; since R4 none has. The deploy checks the set
  again from the pulled images before `up`, where every image is read
  (`apps/envctl/src/release/constraints.rs`, called by `deploy.rs`): an
  unmet requirement replaces nothing. The two sides run on the same cases.
- **RL9. A release's tree holds only what git holds** (C344). The deploy runs
  compose on a worktree of the checkout at the release's commit, where
  `REPO_ROOT` is that worktree. So every `${REPO_ROOT}/…` bind mount is a
  tracked path, mounted read-only. State git does not hold (file_host's
  `data/`, `logs/` and `config/`, Grafana's built dashboards) is mounted
  from `${STATE_ROOT}`, the owner's checkout, which envctl sets for
  `make up` and the deploy alike (`--state-root`). `envctl check` refuses a
  `${REPO_ROOT}` mount that is writable or names an untracked path
  (`apps/envctl/src/mounts.rs`).
- **RL10. Applied whole, or put back whole.** `envctl deploy` brings up a
  release only after every image is pulled and the set checks, gates every
  service (`healthy`, or running a minute with no restart), and on any
  failure re-applies the release last applied, its tree and images, through
  the same `up` and gate. The failed release is skipped by serial until a
  newer one forms. Home never runs python (#786).

## Constraints

Each image's Dockerfile writes two labels, last, so no layer depends on
them:

```dockerfile
LABEL io.paulgsc.provides="contract=6,render-folders=1" \
      io.paulgsc.requires="nats-subjects=1,rust-runner-protocol=1"
```

Each is comma-separated `name=value` pairs. A value is an **epoch** (an
integer: a requirement of n is met by a provision of at least n) or a
**commit** (40 hex: met by the same commit only). An image never meets its
own requirement. An image without the labels requires and provides nothing:
third-party images, `:latest` references (a tag can move, so nothing fixed
is read), and images published before R3. A malformed label fails the check
rather than being skipped.

A provision of n promises what any requirement up to n needs. So a change
that drops an older layout or protocol is shipped only where both ends move
together, or with the provider still serving the older one until no
release holds the old requirer.

The edges written down so far, each a constant beside the code that owns
it, which a test holds equal to the Dockerfile's label:

| Edge | Provided by | Required by | Constant |
|---|---|---|---|
| `contract` | file_host | (clients, through `/compat`) | `CONTRACT_EPOCH`, `apps/servers/file_host/src/contract.rs` |
| `render-folders` | file_host | render (some-ui, U2 #792) | `RENDER_FOLDERS_EPOCH`, same file |
| `rust-runner-protocol` | rust-runner | file_host | `protocol::EPOCH`, `crates/rust_rounds_wire/src/protocol.rs` |
| `nats-subjects` (subjects and payloads; only subjects are pinned by a test) | orchestrator | file_host | `NATS_SUBJECTS_EPOCH`, `crates/ws-events/src/events/common/event_type.rs` |

`render-folders` covers the folders render reads and file_host writes:
`call-weeks`, `playoff-delta-episodes`, `preview-requests` and
`rust-essays`. There is no `stage` edge: call-stage was removed (#802), and
render carries its own stage slice. kokoro writes both labels empty.

## What a turned-down release means

A release pull request someone closes is not remembered. The next run
proposes the newest published set again, the turned-down image included if
it is still the newest. To keep an image off home, publish its fix: its newer
changeset replaces it. Remembering refusals would bring back state that runs
carry, against RL2.

## Where each part lives

| Part | File |
|---|---|
| The release | `deploy/home/release.json` |
| Compose reads it | `infra/compose/*.yml` (`${IMAGE_*}`), `apps/envctl/src/release.rs` |
| What the deploy's gate reads: each service's `healthy` (R5, #794; file-host, kokoro, rust-runner, orchestrator, render) | `infra/compose/*.yml` (`healthcheck:`), render's beat in `infra/render/render-calls.sh` |
| CI reconciles it | `scripts/release_lock.py`, `.github/workflows/release-lock.yml` |
| Called after a server publish | `.github/workflows/publish-image-changesets.yml` (`release`) |
| The set check | `scripts/check_release.py` (CI), `apps/envctl/src/release/constraints.rs` (deploy) |
| Each image's labels | `infra/docker/Dockerfile.*`; render's in some-ui (U2, #792) |
| The deploy applies a release whole | `apps/envctl/src/deploy.rs` (`make home-deploy`, `nix/modules/home-deploy.nix`) |
| What compose may mount from a release's tree | `apps/envctl/src/mounts.rs` (RL9) |
| **Planned:** guard against `:latest`, `build:`, per-service pulls | R6, #795 |
