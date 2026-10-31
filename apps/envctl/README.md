# envctl

Every variable the stack reads, from where each one is declared (#403). One
binary for the config tooling (CFG10, #606), run through `make`:

| Make target | `envctl` | What it does |
|---|---|---|
| `make deploy-check` | `check` | Holds every `deploy/<role>/<binary>.env` to its binary's schema, and each labelled compose service to reading it (CI's `Env schema`). |
| `make preflight` | `preflight` | Before `docker compose up`: every missing or refused variable, and every "ours" value `.env` would override. |
| `make env` | `list` | Every variable the stack reads, and whether `.env` sets it. |
| `make env-schema` | `example`, `nix` | Regenerates `example.env` and `nix/modules/generated/`. |
| `make dev` | `export` | Exports `deploy/dev/file_host.env`, the shell winning. |
| `make import` | `import` | On a running host: fills `deploy/<role>/<binary>.env` from each container labelled `env-schema`. Reads no secret or private value. |
| `make resolve` | `resolve` | Decides each "ours" value in a terminal UI, beside what runs and the variable's full help. |

It reads `infra/env/*.json`, which each binary's tests check in from its clap
`Config`, and `docker compose config --no-interpolate` (compose v5 or later).
README.md at the repository root, "Configuration" and "Deploying the stack",
says what each check promises.
