#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
source build/common.sh

ensure_multiarch_builder
set_image_meta_args "$(current_version)" "$(git rev-parse HEAD)" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

docker buildx build --platform "$PLATFORMS" \
  -f "$DOCKERFILE" --target "$TARGET" \
  "${IMAGE_META_ARGS[@]}" \
  -t "$IMAGE:latest" \
  --push "$@" .
