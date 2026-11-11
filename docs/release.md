# Home's release: one set of images, by digest

Home runs one release. `deploy/home/release.json` names the image of every
service compose starts there. Each entry is `repo:tag@sha256:…` once it is
published, and a few are held or not yet published (below). Epic #787 holds
the design and its record. Stories R1 (#788) and R2 (#789) built what this doc
declares; **Planned** marks what is not built yet, with its issue.

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
  loses nothing (R2). Two review rounds on #804 found images lost by the shape
  that carried them, which is why this is written down.
- **RL3. Newest published, defined per image.** For each image the release
  follows (`SOURCES` in `release_lock.py`), "newest published" means exactly
  one thing:
  - **A server image:** the newest changeset on main that includes it, by its
    `source_sha`'s place in main's history, whose
    `pgathondu/<suffix>:<source_sha>` tag exists. A changeset whose publish
    failed falls back to the one before it.
  - **A some-ui image:** the most recently pushed tag of its release's shape
    (a whole commit for render, `X.Y.Z` for www), never `latest`.
- **RL4. Bytes, not names.** A reference CI writes always carries its digest.
  A tag alone can move under a release.
- **RL5. Held entries do not move.** `file-host` stays
  `pgathondu/server:latest` until the deploy applies whole releases (R4,
  #793), because `envctl deploy` pulls, tags and rolls back that tag. An
  entry the release does not follow keeps whatever main says: third-party
  images until R7 (#796), and locally built ones until they are published
  (kokoro, R2b #805).
- **RL6. No approval of its own.** Forming the set adds no human step
  (C119). Each image keeps the one approval it has where it is published, and
  the release pull request auto-merges once its checks pass. **Planned:**
  until a `RELEASE_BOT_TOKEN` exists, the release pull request's runs wait
  for "Approve workflows to run" (docs/assurance.md), which is the owner's
  call.
- **RL7. A cross-repo image is followed only once its set can be checked.**
  render and www come from some-ui and must match what server writes for them
  (#756: a www release without its matching render broke every preview). So
  they enter `SOURCES` only with R3's set check (#790), which refuses an
  inconsistent set. Until then their entries move by a reviewed pull request.

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
| CI reconciles it | `scripts/release_lock.py`, `.github/workflows/release-lock.yml` |
| Called after a server publish | `.github/workflows/publish-image-changesets.yml` (`release`) |
| **Planned:** set check | R3, #790 |
| **Planned:** deploy applies a release whole | R4, #793 |
| **Planned:** guard against `:latest`, `build:`, per-service pulls | R6, #795 |
