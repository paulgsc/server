# Docker configuration
IMAGE_NAME ?=
DOCKER_REPO ?= paulgsc/$(IMAGE_NAME)
DOCKERFILE_PATH ?=
TAG := latest

# Get git commit hash for tagging
GIT_COMMIT := $(shell git rev-parse --short HEAD)
GIT_BRANCH := $(shell git branch --show-current)

# Docker build targets
.PHONY: build push pull run stop clean cleanup login help deploy rollout

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

rollout: ## Zero-downtime production rollout of file-host (IMAGE=immutable-tag-or-digest)
	@test -n "$(IMAGE)" || { echo "IMAGE is required" >&2; exit 64; }
	@./scripts/deploy-file-host.sh "$(IMAGE)"

pull: ## Pull the latest image from Docker Hub
	@echo "Pulling latest image from Docker Hub..."
	@docker pull $(DOCKER_REPO):$(TAG)

run: ## Run the container locally
	@echo "Running container from $(DOCKER_REPO):$(TAG)..."
	@docker run -d --name $(IMAGE_NAME) -p 3000:3000 $(DOCKER_REPO):$(TAG)

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
	@docker run -it --rm --name $(IMAGE_NAME)-dev -p 3000:3000 $(DOCKER_REPO):dev

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
# `dump-routes` reads a compile-time constant, so it needs no config, no
# database and no network — but building it does require DATABASE_URL to point
# at a migrated SQLite file, because sqlx checks queries at compile time.

ROUTES_JSON ?= routes.server.json
ROUTES_TS ?= routes.server.ts

.PHONY: routes routes-check

routes: ## Emit the HTTP route inventory as JSON and TypeScript (override paths with ROUTES_JSON=..., ROUTES_TS=...)
	@cargo run -q --bin dump-routes > $(ROUTES_JSON)
	@cargo run -q --bin dump-routes -- --ts > $(ROUTES_TS)
	@echo "Wrote $(ROUTES_JSON) ($$(grep -c '"method"' $(ROUTES_JSON)) routes) and $(ROUTES_TS)"
	@echo "Copy both to the client repo at packages/contract-harness/"

routes-check: ## Assert the declared inventory still matches the actual routers
	@cargo test -p file_host --lib routes::inventory
