FROM node:24.19.0-slim AS frontend-builder
WORKDIR /app/web
COPY web/package.json web/package-lock.json* ./
RUN npm ci
COPY web/ .
RUN npm run build

# Stage 2: shared Rust base — apt packages + cargo tools installed once,
# cached by Docker layer so both planner and backend-builder reuse it.
FROM rust:1.97.1-bookworm AS rust-base
RUN --mount=type=bind,source=build/pkgs/builder-rust-apt.txt,target=/tmp/packages.txt \
  apt-get update && apt-get install -y --no-install-recommends \
  $(grep -vE '^\s*(#|$)' /tmp/packages.txt | tr '\n' ' ') \
  && rm -rf /var/lib/apt/lists/*
RUN --mount=type=bind,source=build/pkgs/builder-rust-cargo.txt,target=/tmp/cargo.txt \
  for line in $(grep -vE '^\s*(#|$)' /tmp/cargo.txt); do \
    crate=${line%%=*}; version=${line#*=}; \
    cargo install "$crate" --version "$version" --locked; \
  done

# Stage 3: cargo-chef planner
FROM rust-base AS planner
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo chef prepare --recipe-path recipe.json

# Stage 4: backend builder
#
# The cargo target dir and registry are BuildKit cache mounts so incremental
# compilation and downloaded crates persist across builds — a small backend
# change then recompiles only what changed instead of the whole workspace.
# Cache mounts aren't part of the committed layer, so the built binaries are
# copied out to /app before the RUN ends for the prod stage to COPY.
FROM rust-base AS backend-builder
WORKDIR /app
COPY --from=planner /app/recipe.json recipe.json
COPY resources/ resources/
RUN --mount=type=cache,target=/app/target,id=frona-target \
    --mount=type=cache,target=/usr/local/cargo/registry,id=frona-cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=frona-cargo-git \
    cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY resources/ resources/
RUN --mount=type=cache,target=/app/target,id=frona-target \
    --mount=type=cache,target=/usr/local/cargo/registry,id=frona-cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=frona-cargo-git \
    cargo build --release -p frona-server -p frona-cli-mcp \
    && cp target/release/frona /app/frona \
    && cp target/release/mcpctl /app/mcpctl

#
# The reference ontologies are a published artifact of `frona-ontologies`, fetched at
# build time rather than committed: ~4.2 MB of gzipped Turtle that changes on its own
# release clock, so keeping it out of git means cutting a release needs no commit here.
# The server reads the `.gz` as a stream - nothing unpacks them, here or at runtime.
#
# `NOTICE` is **not** optional: the sources are CC-BY and the attribution has to ship
# with anything that redistributes them.
#
# Failing the build on an empty result is the point. Without that check a network
# hiccup produces an image whose PKM backend silently classifies nothing, which is
# exactly the class of quiet failure the ontology work exists to avoid.
FROM debian:bookworm-slim AS ontology
RUN apt-get update \
  && apt-get install -y --no-install-recommends curl ca-certificates jq \
  && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /ontology && cd /ontology \
  && curl -fsSL https://api.github.com/repos/Mpercy-Git/frona-ontologies/releases/latest \
     -o /tmp/release.json \
  && jq -r '.assets[].browser_download_url' /tmp/release.json \
     | grep -E '\.ttl\.gz$|/(metadata\.json|NOTICE)$' \
     | xargs -r -n1 curl -fsSL -O \
  && ls *.ttl.gz >/dev/null \
  && echo "ontology release $(jq -r .tag_name /tmp/release.json): $(ls | tr '\n' ' ')"

FROM debian:bookworm-slim AS cli-tools
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip xz-utils ca-certificates && rm -rf /var/lib/apt/lists/*
RUN --mount=type=bind,source=build/pkgs/prod-pkgs.txt,target=/tmp/pkgs.txt \
  OP_VER=$(grep '^op=' /tmp/pkgs.txt | cut -d= -f2) \
  && BW_VER=$(grep '^bw=' /tmp/pkgs.txt | cut -d= -f2) \
  && curl -sSfLo /tmp/op.zip "https://cache.agilebits.com/dist/1P/op2/pkg/v${OP_VER}/op_linux_${TARGETARCH}_v${OP_VER}.zip" \
  && unzip -o /tmp/op.zip -d /usr/local/bin op \
  && chmod +x /usr/local/bin/op \
  && rm /tmp/op.zip \
  && if [ "$TARGETARCH" = "amd64" ]; then BW_ARCH=""; else BW_ARCH="-${TARGETARCH}"; fi \
  && curl -sSfLo /tmp/bw.zip "https://github.com/bitwarden/clients/releases/download/cli-v${BW_VER}/bw-linux${BW_ARCH}-${BW_VER}.zip" \
  && unzip -o /tmp/bw.zip -d /usr/local/bin \
  && chmod +x /usr/local/bin/bw \
  && rm /tmp/bw.zip
RUN --mount=type=bind,source=build/pkgs/prod-pkgs.txt,target=/tmp/pkgs.txt \
  SYD_VER=$(grep '^syd=' /tmp/pkgs.txt | cut -d= -f2) \
  && SYD_TRIPLE=$([ "$TARGETARCH" = "amd64" ] && echo "x86_64-pc-linux-gnu" || echo "aarch64-unknown-linux-gnueabi") \
  && curl -sSfL "https://distfiles.exherbo.org/sydbox%2Fsyd-${SYD_VER}-${SYD_TRIPLE}.tar.xz" -o /tmp/syd.tar.xz \
  && tar xf /tmp/syd.tar.xz -C /tmp \
  && cp /tmp/syd-${SYD_VER}-${SYD_TRIPLE}/bin/syd /usr/local/bin/syd \
  && chmod +x /usr/local/bin/syd \
  && rm -rf /tmp/syd*
COPY Cargo.lock /tmp/Cargo.lock
RUN SURREAL_VER=$(grep -A1 'name = "surrealdb"' /tmp/Cargo.lock | grep version | sed 's/.*"\(.*\)".*/\1/') \
  && curl -sSfL "https://github.com/surrealdb/surrealdb/releases/download/v${SURREAL_VER}/surreal-v${SURREAL_VER}.linux-${TARGETARCH}.tgz" \
  | tar xz -C /usr/local/bin surreal \
  && chmod +x /usr/local/bin/surreal

# Stage 5: python packages
FROM python:3.12-bookworm AS python-builder
RUN --mount=type=bind,source=build/pkgs/builder-python-apt.txt,target=/tmp/packages.txt \
  apt-get update && apt-get install -y --no-install-recommends \
  $(grep -vE '^\s*(#|$)' /tmp/packages.txt | tr '\n' ' ') \
  && rm -rf /var/lib/apt/lists/*
COPY build/pkgs/builder-python-pip.txt .
RUN pip install --no-cache-dir --prefix=/install -r builder-python-pip.txt


# Same base as prod so dev and prod share the exact Python 3.12 interpreter and the
# same runtime deps; dev only layers on the Rust build toolchain (dev-apt.txt).
FROM python:3.12-slim-bookworm AS dev
RUN --mount=type=bind,source=build/pkgs/prod-apt.txt,target=/tmp/packages.txt \
  apt-get update \
  && apt-get install -y --no-install-recommends \
    $(grep -vE '^\s*(#|$)|^nodejs($|=)' /tmp/packages.txt | tr '\n' ' ') \
  && curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
  && apt-get install -y --no-install-recommends \
    $(grep -E '^nodejs($|=)' /tmp/packages.txt | cut -d= -f1) \
  && rm -rf /var/lib/apt/lists/*
RUN --mount=type=bind,source=build/pkgs/dev-apt.txt,target=/tmp/packages.txt \
  apt-get update \
  && apt-get install -y --no-install-recommends \
     $(grep -vE '^\s*(#|$)' /tmp/packages.txt | tr '\n' ' ') \
  && rm -rf /var/lib/apt/lists/*

# Rust toolchain via rustup (dev compiles live via cargo-watch; prod copies release
# binaries from backend-builder).
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --no-modify-path --profile minimal --default-toolchain 1.97.1 \
  && rustup component add clippy rustfmt

COPY --from=python-builder /install /usr/local
# Pre-compile .pyc so Python needs no write access to system dirs at runtime
# (syd sandbox blocks writes outside the agent workspace) - matches prod.
RUN python3 -m compileall -q /usr/local/lib/python3.12

RUN --mount=type=bind,source=build/pkgs/dev-rust-cargo.txt,target=/tmp/cargo.txt \
  for line in $(grep -vE '^\s*(#|$)' /tmp/cargo.txt); do \
    crate=${line%%=*}; version=${line#*=}; \
    cargo install "$crate" --version "$version" --locked; \
  done

COPY --from=cli-tools /usr/local/bin/op /usr/local/bin/bw /usr/local/bin/surreal /usr/local/bin/syd /usr/local/bin/

ARG DEV_UID=1000
ARG DEV_GID=1000
RUN groupadd -g "$DEV_GID" frona \
    && useradd -u "$DEV_UID" -g "$DEV_GID" -m -s /bin/bash frona
RUN mkdir -p /app /app/target /app/frontend/node_modules /app/data /app/bin \
    /usr/local/cargo/registry /usr/local/cargo/git \
    && chown -R frona:frona /app /usr/local/cargo
ENV PATH="/app/bin:${PATH}"
USER frona
WORKDIR /app
COPY --chown=frona:frona web/package.json web/package-lock.json* web/
RUN cd web && npm install
CMD ["bash", "-c", "cd web && npm install && cd /app && cargo watch -w crates/ -w Cargo.toml -w Cargo.lock --delay 3 -s /app/build/dev-watch.sh & cd web && npm run dev & wait"]


FROM python:3.12-slim-bookworm AS prod
ARG VERSION=dev
ARG REVISION=unknown
ARG CREATED=unknown
LABEL org.opencontainers.image.source="https://github.com/Mpercy-Git/frona"
LABEL org.opencontainers.image.description="Frona — personal AI assistant"
LABEL org.opencontainers.image.licenses="BSL-1.1"
LABEL org.opencontainers.image.title="frona"
LABEL org.opencontainers.image.version="$VERSION"
LABEL org.opencontainers.image.revision="$REVISION"
LABEL org.opencontainers.image.created="$CREATED"
RUN --mount=type=bind,source=build/pkgs/prod-apt.txt,target=/tmp/packages.txt \
  apt-get update \
  && apt-get install -y --no-install-recommends \
    $(grep -vE '^\s*(#|$)|^nodejs($|=)' /tmp/packages.txt | tr '\n' ' ') \
  && curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
  && apt-get install -y --no-install-recommends \
    $(grep -E '^nodejs($|=)' /tmp/packages.txt | cut -d= -f1) \
  && rm -rf /var/lib/apt/lists/*

COPY --from=cli-tools /usr/local/bin/op /usr/local/bin/bw /usr/local/bin/surreal /usr/local/bin/syd /usr/local/bin/

RUN groupadd -g 1000 frona && useradd -u 1000 -g frona -d /app -s /bin/bash frona
RUN mkdir -p /app /app/bin /data && chown -R frona:frona /app /data
WORKDIR /app

COPY --from=python-builder /install /usr/local
# Pre-compile .pyc files so Python doesn't need write access to system dirs
# at runtime (syd sandbox blocks writes outside the agent workspace).
RUN python3 -m compileall -q /usr/local/lib/python3.12
COPY --chown=frona:frona --from=backend-builder /app/frona /app/bin/frona
COPY --chown=frona:frona --from=backend-builder /app/mcpctl /app/bin/mcpctl
COPY --chown=frona:frona --from=frontend-builder /app/web/out /app/static
COPY --chown=frona:frona resources /app/share
# After `resources`, so it lands at the path `storage.shared_config_dir` derives:
# /app/share/ontology. Read-only here by design - anything the user adds goes to
# `storage.ontology_dir` on the data volume, which survives a restart.
COPY --chown=frona:frona --from=ontology /ontology /app/share/ontology

ENV PATH="/app/bin:${PATH}"
# Version reported at runtime (see core::app_version). Set from the VERSION
# build-arg so releases don't need a Cargo.toml bump — which would invalidate
# the cached Rust build and force a full recompile. Defaults to "dev".
ENV FRONA_VERSION=$VERSION
ENV FRONA_SERVER_STATIC_DIR=/app/static
ENV FRONA_SERVER_DATA_DIR=/app/data
ENV FRONA_SERVER_PORT=3001
ENV FRONA_STORAGE_SHARED_CONFIG_DIR=/app/share
ENV FRONA_LOG_LEVEL=info
ENV SURREAL_SYNC_DATA=true

VOLUME /app/data
EXPOSE 3001
USER frona
CMD ["frona"]
