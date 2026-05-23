# Shared constants and helpers for build scripts.
# Source after cd-ing to the repo root.

IMAGE="ghcr.io/fronalabs/frona"
DOCKERFILE="build/Dockerfile"
TARGET="prod"
PLATFORMS="${PLATFORM:-linux/amd64,linux/arm64}"

die() { echo "error: $*" >&2; exit 1; }

current_version() {
  grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/'
}

ensure_multiarch_builder() {
  docker buildx inspect multiarch >/dev/null 2>&1 || \
    docker buildx create --name multiarch --use
  docker buildx use multiarch
}

# Populates IMAGE_META_ARGS with --build-arg + --annotation flags.
# Args: version revision created
set_image_meta_args() {
  local version="$1" revision="$2" created="$3"
  IMAGE_META_ARGS=(
    --build-arg "VERSION=$version"
    --build-arg "REVISION=$revision"
    --build-arg "CREATED=$created"
    --annotation "index:org.opencontainers.image.source=https://github.com/fronalabs/frona"
    --annotation "index:org.opencontainers.image.description=Frona — personal AI assistant"
    --annotation "index:org.opencontainers.image.licenses=BSL-1.1"
    --annotation "index:org.opencontainers.image.title=frona"
    --annotation "index:org.opencontainers.image.version=$version"
    --annotation "index:org.opencontainers.image.revision=$revision"
    --annotation "index:org.opencontainers.image.created=$created"
  )
}
