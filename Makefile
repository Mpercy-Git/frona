REGISTRY  := ghcr.io/mpercy-git/frona
CACHE_DIR := $(HOME)/.docker/buildx-cache/frona
PLATFORM  := linux/amd64

# Detect native arch so the cache dir is arch-specific
ARCH      := $(shell uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
CACHE_DIR := $(HOME)/.docker/buildx-cache/frona-$(ARCH)

# ── Local build (native platform only, persistent local cache) ───────────────

## Build the prod image for the local platform and load it into Docker.
## Usage: make build [TAG=next]
TAG ?= next
build:
	docker buildx build \
	  --file build/Dockerfile \
	  --target prod \
	  --platform linux/$(ARCH) \
	  --cache-from type=local,src=$(CACHE_DIR) \
	  --cache-to   type=local,dest=$(CACHE_DIR),mode=max \
	  --tag $(REGISTRY):$(TAG) \
	  --load \
	  .

## Build and push to GHCR (native platform only — fast).
## Usage: make push [TAG=next]
push: build
	docker push $(REGISTRY):$(TAG)

## Build multi-arch and push (slow — same as CI; use for releases).
## Usage: make release TAG=v2026.6.11
release:
	docker buildx build \
	  --file build/Dockerfile \
	  --target prod \
	  --platform linux/amd64,linux/arm64 \
	  --cache-from type=local,src=$(HOME)/.docker/buildx-cache/frona-amd64 \
	  --cache-from type=local,src=$(HOME)/.docker/buildx-cache/frona-arm64 \
	  --tag $(REGISTRY):$(TAG) \
	  --push \
	  .

.PHONY: build push release
