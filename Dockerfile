# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.88-trixie AS builder

RUN apt-get update && apt-get install -y \
    python3-dev \
    pkg-config \
    libssl-dev \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y \
    python3 \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/discord-demo /usr/local/bin/edgar

RUN mkdir -p /app/data/docs

VOLUME ["/app/data"]

ENTRYPOINT ["edgar"]
