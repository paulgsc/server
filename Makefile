# Docker configuration
IMAGE_NAME ?=
DOCKER_REPO ?= paulgsc/$(IMAGE_NAME)
DOCKERFILE_PATH ?=
TAG := latest

# Get git commit hash for tagging
GIT_COMMIT := $(shell git rev-parse --short HEAD)
GIT_BRANCH := $(shell git branch --show-current)

# Docker build targets
.PHONY: build push pull run stop clean cleanup login help deploy

help: ## Show this help message
	@echo "Available targets:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $1, $2}'

login: ## Login to Docker Hub
	@echo "Logging into Docker Hub..."
	@docker login

build: ## Build the Docker image
ifndef IMAGE_NAME
	$(error IMAGE_NAME is not set. Usage: make build IMAGE_NAME=... DOCKER_REPO=... DOCKERFILE_PATH=...)
endif
ifndef DOCKER_REPO
	$(error DOCKER_REPO is not set. Usage: make build IMAGE_NAME=... DOCKER_REPO=... DOCKERFILE_PATH=...)
endif
ifndef DOCKERFILE_PATH
	$(error DOCKERFILE_PATH is not set. Usage: make build IMAGE_NAME=... DOCKER_REPO=... DOCKERFILE_PATH=...)
endif
	@echo "Building Docker image: $(DOCKER_REPO):$(TAG)"
	@docker build -f $(DOCKERFILE_PATH) -t $(DOCKER_REPO):$(TAG) .
	@docker tag $(DOCKER_REPO):$(TAG) $(DOCKER_REPO):dev-$(GIT_COMMIT)
	@echo "Built images:"
	@echo "  - $(DOCKER_REPO):$(TAG)"
	@echo "  - $(DOCKER_REPO):dev-$(GIT_COMMIT)"

push: ## Push the Docker image to Docker Hub
	@echo "Pushing Docker images to Docker Hub..."
	@docker push $(DOCKER_REPO):$(TAG)
	@docker push $(DOCKER_REPO):dev-$(GIT_COMMIT)
	@echo "Successfully pushed images"

deploy: login build push ## One command to rule them all - build and push
	@echo "🚀 Deploy complete! Your server can now pull with:"
	@echo "   docker pull $(DOCKER_REPO):$(TAG)"

pull: ## Pull the latest image from Docker Hub
	@echo "Pulling latest image from Docker Hub..."
	@docker pull $(DOCKER_REPO):$(TAG)

run: ## Run the container locally
	@echo "Running container from $(DOCKER_REPO):$(TAG)..."
	@docker run -d --name $(IMAGE_NAME) -p 127.0.0.1:3000:3000 $(DOCKER_REPO):$(TAG)

stop: ## Stop the running container
	@echo "Stopping container..."
	@docker stop $(IMAGE_NAME) || true
	@docker rm $(IMAGE_NAME) || true

clean: ## Remove local Docker images
	@echo "Cleaning up local images..."
	@docker rmi $(DOCKER_REPO):$(TAG) || true
	@docker rmi $(DOCKER_REPO):$(GIT_COMMIT) || true
	@docker rmi $(DOCKER_REPO):$(GIT_BRANCH)-$(GIT_COMMIT) || true

cleanup: ## Remove dangling images and containers
	@echo "Cleaning up dangling images and stopped containers..."
	@docker system prune -f
	@docker image prune -f

build-and-push: login build push ## Build and push in one command
	@echo "Build and push completed successfully!"

# Development helpers
dev-build: ## Build with dev tag
	@docker build -f $(DOCKERFILE_PATH) -t $(DOCKER_REPO):dev .

dev-run: ## Run development container
	@docker run -it --rm --name $(IMAGE_NAME)-dev -p 127.0.0.1:3000:3000 $(DOCKER_REPO):dev

# Multi-platform build (requires buildx)
build-multi: ## Build multi-platform image (linux/amd64,linux/arm64)
	@echo "Building multi-platform image..."
	@docker buildx build --platform linux/amd64,linux/arm64 -f $(DOCKERFILE_PATH) -t $(DOCKER_REPO):$(TAG) . --push

# Check if image exists locally
check: ## Check if image exists locally
	@docker images | grep $(DOCKER_REPO) || echo "No local images found for $(DOCKER_REPO)"


# ── Contract boundary ────────────────────────────────────────────────────────
# The client repo (paulgsc/some-ui) keeps a checked-in snapshot of this
# server's HTTP surface and diffs its contracts against it. Regenerate the
# snapshot whenever a route is added, renamed, or removed.
#
# `dump-routes` reads the route tables without building an `AppState` or a
# `Config`, so it needs no config, no database and no network at run time; but
# building it does require DATABASE_URL to point at a migrated SQLite file,
# because sqlx checks queries at compile time. CI (.github/workflows/routes.yml)
# runs this on every PR and merge and delivers the result to the client repo,
# so running it by hand is only for looking.

ROUTES_JSON ?= routes.server.json
ROUTES_TS ?= routes.server.ts

.PHONY: routes routes-check

routes: ## Emit the HTTP route inventory as JSON and TypeScript (override paths with ROUTES_JSON=..., ROUTES_TS=...)
	@cargo run -q --bin dump-routes > $(ROUTES_JSON)
	@cargo run -q --bin dump-routes -- --ts > $(ROUTES_TS)
	@echo "Wrote $(ROUTES_JSON) ($$(grep -c '"method"' $(ROUTES_JSON)) routes) and $(ROUTES_TS)"
	@echo "CI syncs these to the client repo; see .github/workflows/routes.yml"

routes-check: ## Run the route inventory's own tests (duplicates, nesting, assembly)
	@cargo test -p file_host --lib routes::inventory

# ── file_host from source, beside the production container ──────────────────
#
# The container holds 3000. `make dev` runs the server from source anyway: it
# starts at 3000, moves up to the next free port when that is taken, and
# records the port it got in DEV_PORT_FILE. paulgsc/some-ui's `vite dev` reads
# that file and proxies to this server while it runs, and back to the
# container once it stops - see apps/servers/file_host/src/listen.rs. The path
# is agreed with apps/www/vite.config.ts there; change both together.
#
# Its variables are deploy/dev/file_host.sops.env (#614), decrypted into its
# environment only, never to a file. A variable the shell already sets wins
# (`FOO=bar make dev`). Pointed at the container's database and VAPID keys,
# it would run a second nudge waker beside the container's, so dev's file
# gives it its own (README.md, "Development").

DEV_PORT_FILE ?= $(or $(XDG_RUNTIME_DIR),/tmp)/file_host/dev-port.json

# ── Configuration: every variable a secret, set once per deployment (#613) ───
#
# Each binary's clap Config is its schema; its tests hold a copy of it in
# infra/env/<binary>.json, the contract envctl (apps/envctl) reads. A
# deployment's values are deploy/<role>/<app>.sops.env, one file per app and
# one for compose's own ${...} variables (compose.sops.env), encrypted with
# sops to .sops.yaml's recipients: names readable, values not. Nothing on the
# box is a plain .env.
#
# `make up` decrypts them: each app's file to $(XDG_RUNTIME_DIR)/server/<pid> for its
# compose service only (tmpfs, mode 600, removed once compose has started it),
# compose's into compose's own environment. It runs the preflight first, which
# names every missing or refused variable at once. Any other compose command
# needs the same files: `make compose ARGS="logs -f file-host"`.
#
# DEPLOY is the role make up and the rest use, home by default; make dev's is
# DEV_DEPLOY, dev by default. cargo runs under the decrypted file too: sqlx's
# query! checks SQL at compile time against its DATABASE_URL, which no .env
# gives it any more. REPO_ROOT
# is this checkout, from make itself. Building envctl, like `make routes`,
# needs no database; `make env-schema` does (sqlx checks queries at compile
# time).

ROLE = $(or $(DEPLOY),home)
# make dev's own role, apart from DEPLOY: an exported DEPLOY=home must never
# run a second file_host on home's values (its VAPID keys, its waker).
DEV_DEPLOY ?= dev
REPO_ROOT := $(CURDIR)
TARGET_DIR := $(abspath $(or $(CARGO_TARGET_DIR),target))
# cargo is the flake's, not the system's: outside `nix develop`, each cargo
# command runs through the rust shell (which also has sops).
IN_RUST_SHELL := $(if $(shell command -v cargo 2>/dev/null),,nix develop $(CURDIR)\#rust -c)
ENVCTL ?= $(TARGET_DIR)/debug/envctl
ENVCTL_RUN = $(ENVCTL) --repo $(REPO_ROOT)

.PHONY: envctl dev env-schema env preflight up compose import prune tool deploy-check

# Cargo's own progress on stderr: the first build takes a minute, and a cargo
# blocked on another one's lock (rust-analyzer, say) says so instead of
# looking hung.
envctl: ## Build the config tooling (apps/envctl)
	@$(IN_RUST_SHELL) cargo build -p envctl

dev: envctl ## Run file_host from source on deploy/dev/file_host.sops.env: next free port from 3000, recorded for some-ui's vite dev
	@FILE_HOST_PORT_FALLBACK=true FILE_HOST_PORT_FILE=$(DEV_PORT_FILE) \
		$(ENVCTL_RUN) run --role $(DEV_DEPLOY) --app file_host -- $(IN_RUST_SHELL) cargo run -p file_host --bin file_host

env-schema: envctl ## Regenerate infra/env/*.json from each binary's Config, then nix/modules/generated from those
	@UPDATE_ENV_SCHEMA=1 $(IN_RUST_SHELL) cargo test -q -p file_host -p orchestrator -p some-obs --lib the_checked_in_schema_is_this_config
	@$(ENVCTL_RUN) nix
	@echo "Wrote infra/env/*.json and nix/modules/generated"

env: envctl ## List every variable the stack reads, its default, and whether deploy/$(ROLE)/ sets it (by name; nothing decrypted)
	@$(ENVCTL_RUN) list --role $(ROLE)

deploy-check: envctl ## Without a key, names only (CI's Env schema): every deploy/ file, service and binary conforms
	@$(ENVCTL_RUN) check

preflight: envctl ## Decrypt deploy/$(ROLE)/ into memory and name every missing or refused variable at once
	@$(ENVCTL_RUN) preflight --role $(ROLE)

up: envctl ## The preflight, then docker compose up -d on deploy/$(ROLE)/, decrypted to tmpfs only
	@$(ENVCTL_RUN) up --role $(ROLE)

compose: envctl ## Any docker compose command on deploy/$(ROLE)/: make compose ARGS="ps"
	@$(ENVCTL_RUN) compose --role $(ROLE) -- $(ARGS)

# Once, on the box (ENV2, #615): deploy/$(ROLE)/ written from what the
# running containers use, straight into sops. DRY=1 only names what would
# change. It ends by naming the .env to delete once `make up` has run.
import: envctl ## Write deploy/$(ROLE)/ from the running containers, through sops (DRY=1 to only report)
	@$(ENVCTL_RUN) import --role $(ROLE) $(if $(DRY),--dry-run)

# Prune variables of one app's file (ENV2b, #628): a checklist grouped by the
# schema's sections, PAGE rows at a time; a declared variable is stated unset,
# an undeclared one removed, written through sops. NAMES=A,B prunes without it.
# A command run by hand on the box (`sqlx migrate run`, say) gets the role's
# host-side values from deploy/$(ROLE)/tools.sops.env (#634), decrypted into
# its environment only. `nix develop .#rust` exports the same file for the
# whole shell; this is the one-off form, for any shell.
tool: envctl ## Run one command with deploy/$(ROLE)/tools.sops.env in its environment: make tool ARGS="sqlx migrate run --dry-run"
	@$(ENVCTL_RUN) run --role $(ROLE) --app tools -- $(IN_RUST_SHELL) $(ARGS)

prune: envctl ## Prune deploy/$(ROLE)/$(APP).sops.env's variables in a checklist (APP=file_host, NAMES=A,B to skip it)
	@$(ENVCTL_RUN) prune --role $(ROLE) --app $(or $(APP),file_host) $(if $(PAGE),--page $(PAGE)) $(if $(NAMES),--prune $(NAMES))
