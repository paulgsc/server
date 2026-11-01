# envctl

Every variable the stack reads, from where each one is declared (#403), and
every value a deployment sets, from its encrypted files (#613). It is run
through `make`:

| Make target | `envctl` | What it does |
|---|---|---|
| `make up` | `up` | The preflight, then `docker compose up -d`: compose's variables in its environment, each labelled service's own file on tmpfs for its `env_file`, removed once compose returns. |
| `make compose ARGS=…` | `compose` | Any other `docker compose` command, on the same files. |
| `make preflight` | `preflight` | Decrypts `deploy/<role>/` into memory and names every missing or refused variable at once. |
| `make dev` | `run` | Runs `file_host` from source on `deploy/dev/file_host.sops.env`, decrypted into its environment, the shell winning. |
| `make import` | `import` | Once, on the box (#615): writes `deploy/<role>/` from the running containers, straight through `sops encrypt`. |
| `make env` | `list` | Every variable the stack reads, and whether the role's files set it, read by name without decrypting. |
| `make deploy-check` | `check` | Without a key (CI's `Env schema`, ENV3 #616): every `deploy/` file sops-encrypted and naming only what its binary or compose reads, each labelled service reading only its own file, a schema for every binary that reads `app-config`, and no tracked plain-text env file. |
| `make env-schema` | `nix` | Regenerates `nix/modules/generated/`. |

It reads `infra/env/*.json`, which each binary's tests check in from its clap
`Config`, and `docker compose config --no-interpolate` (compose v5 or later).
It runs `sops` (3.9 or later; `$SOPS` names another) to decrypt into its own
memory and to encrypt from its stdin, writes a decrypted value only to tmpfs,
and prints names, never a value. README.md at the repository root,
"Configuration" and "Deploying the stack", says what each promises.
