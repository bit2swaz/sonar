# syntax=docker/dockerfile:1.7
FROM rust:1.94.1-slim-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        clang \
        libssl-dev \
        lld \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY .cargo ./.cargo
COPY bin ./bin
COPY config ./config
COPY crates ./crates
COPY echo_callback ./echo_callback
COPY program ./program
COPY programs ./programs
COPY Anchor.toml ./
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

RUN cargo build --locked --release --bin sonar-coordinator

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        gettext-base \
        libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /home/sonar --shell /usr/sbin/nologin sonar

WORKDIR /app

COPY --from=builder /app/target/release/sonar-coordinator /usr/local/bin/sonar-coordinator
COPY docker/config/offchain.prod.toml.tpl /app/config/offchain.prod.toml.tpl

ENV SONAR_CONFIG_PATH=/tmp/offchain.prod.toml
ENV RUST_LOG=info

USER sonar

ENTRYPOINT ["/bin/sh", "-lc", "envsubst < /app/config/offchain.prod.toml.tpl | sed \"s/__MOCK_PROVER__/${MOCK_PROVER}/g; s/__METRICS_PORT__/${METRICS_PORT}/g\" > ${SONAR_CONFIG_PATH} && exec /usr/local/bin/sonar-coordinator"]
