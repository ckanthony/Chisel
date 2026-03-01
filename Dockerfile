# ── Stage 1: build ─────────────────────────────────────────────────────────
FROM rust:latest AS builder

WORKDIR /build

# Cache dependency compilation separately from source
COPY Cargo.toml Cargo.lock ./
COPY chisel-core/Cargo.toml chisel-core/Cargo.toml
COPY chisel/Cargo.toml chisel/Cargo.toml
RUN mkdir -p chisel-core/src chisel/src \
    && echo '' > chisel-core/src/lib.rs \
    && echo 'fn main(){}' > chisel/src/main.rs \
    && cargo build --release -p chisel \
    && rm -rf chisel-core/src chisel/src

COPY chisel-core ./chisel-core
COPY chisel ./chisel
# Touch main.rs so cargo rebuilds only our code, not deps
RUN touch chisel/src/main.rs && cargo build --release -p chisel

# ── Stage 2: runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# Install minimal runtime libs (openssl for TLS, ca-certs for HTTPS tool calls)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    grep \
    findutils \
    coreutils \
    sed \
    gawk \
    diffutils \
    file \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd -r -s /bin/false chisel

COPY --from=builder /build/target/release/chisel /usr/local/bin/chisel

# Data directory — operators mount their project directory here
RUN mkdir /data && chown chisel:chisel /data

USER chisel

EXPOSE 3000

# Secret is supplied at runtime via MCP_APP_SECRET environment variable
CMD ["chisel", "--root", "/data"]
