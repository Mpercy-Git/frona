#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
source build/common.sh

docker buildx build --platform "$PLATFORMS" \
  -f "$DOCKERFILE" --target "$TARGET" -t frona "$@" .
