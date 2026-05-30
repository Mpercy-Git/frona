# ============================
# PROD BUILD STAGES
# ============================

# Stage 1: frontend
FROM node:22-slim AS frontend-builder
WORKDIR /app/web
COPY web/package.json web/package-lock.json* ./
RUN npm ci
COPY web/ .
RUN npm run build

# Stage 2: cargo-chef planner
FROM rust:1.95-bookworm AS planner
RUN --mount=type=bind,source=build/pkgs/builder-rust-cargo.txt,target=/tmp/cargo.txt \
  for line in $(grep -vE '^\s*(#|$)' /tmp/cargo.txt); do \
    crate=${line%%=*}; version=${line#*=}; \
    cargo install "$crate" --version "$version" --locked; \
  done
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: backend builder
FROM rust:1.95-bookworm AS backend-builder
RUN --mount=type=bind,source=build/pkgs/builder-rust-apt.txt,target=/tmp/packages.txt \
  apt-get update && apt-get install -y --no-install-recommends \
  $(grep -vE '^\s*(#|$)' /tmp/packages.txt | tr '\n' ' ') \
  && rm -rf /var/lib/apt/lists/*
RUN --mount=type=bind,source=build/pkgs/builder-rust-cargo.txt,target=/tmp/cargo.txt \
  for line in $(grep -vE '^\s*(#|$)' /tmp/cargo.txt); do \
    crate=${line%%=*}; version=${line#*=}; \
    cargo install "$crate" --version "$version" --locked; \
  done
WORKDIR /app
COPY --from=planner /app/recipe.json recipe.json
COPY resources/ resources/
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY resources/ resources/
RUN cargo build --release -p frona-server -p frona-cli-mcp

# Stage 4: CLI tools (op, bw, syd)
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
FROM python:3.12-slim-bookworm AS python-builder
RUN --mount=type=bind,source=build/pkgs/builder-python-apt.txt,target=/tmp/packages.txt \
  apt-get update && apt-get install -y --no-install-recommends \
  $(grep -vE '^\s*(#|$)' /tmp/packages.txt | tr '\n' ' ') \
  && rm -rf /var/lib/apt/lists/*
COPY build/pkgs/builder-python-pip.txt .
RUN pip install --no-cache-dir --prefix=/install -r builder-python-pip.txt

# ============================
# DEV TARGET
# ============================

FROM rust:1.95-bookworm AS dev
RUN --mount=type=bind,source=build/pkgs/dev-apt.txt,target=/tmp/packages.txt \
  apt-get update \
  && apt-get install -y --no-install-recommends \
    $(grep -vE '^\s*(#|$)|^nodejs($|=)' /tmp/packages.txt | tr '\n' ' ') \
  && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
  && apt-get install -y --no-install-recommends \
    $(grep -E '^nodejs($|=)' /tmp/packages.txt | cut -d= -f1) \
  && rm -rf /var/lib/apt/lists/*
COPY build/pkgs/builder-python-pip.txt /tmp/requirements.txt
RUN pip install --no-cache-dir --break-system-packages -r /tmp/requirements.txt
RUN --mount=type=bind,source=build/pkgs/dev-rust-cargo.txt,target=/tmp/cargo.txt \
  for line in $(grep -vE '^\s*(#|$)' /tmp/cargo.txt); do \
    crate=${line%%=*}; version=${line#*=}; \
    cargo install "$crate" --version "$version" --locked; \
  done

COPY --from=cli-tools /usr/local/bin/op /usr/local/bin/bw /usr/local/bin/surreal /usr/local/bin/syd /usr/local/bin/

RUN groupadd -g 1000 frona && useradd -u 1000 -g frona -m -s /bin/bash frona
RUN mkdir -p /app /app/target /app/frontend/node_modules /app/data /app/bin \
    /usr/local/cargo/registry /usr/local/cargo/git \
    && chown -R frona:frona /app /usr/local/cargo
ENV PATH="/app/bin:${PATH}"
USER frona
WORKDIR /app
COPY --chown=frona:frona web/package.json web/package-lock.json* web/
RUN cd web && npm install
CMD ["bash", "-c", "cd web && npm install && cd /app && cargo watch --poll -w crates/ -w Cargo.toml -w Cargo.lock --delay 3 -s /app/build/dev-watch.sh & cd web && npm run dev & wait"]

# ============================
# PROD TARGET (default)
# ============================

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
  && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
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
COPY --chown=frona:frona --from=backend-builder /app/target/release/frona /app/bin/frona
COPY --chown=frona:frona --from=backend-builder /app/target/release/mcpctl /app/bin/mcpctl
COPY --chown=frona:frona --from=frontend-builder /app/web/out /app/static
COPY --chown=frona:frona resources /app/share

ENV PATH="/app/bin:${PATH}"
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
