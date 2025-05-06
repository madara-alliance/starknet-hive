# =========================================================================== #
#                            Openrpc-testgen-runner                           #
#       Based off https://depot.dev/blog/rust-dockerfile-best-practices       #
# =========================================================================== #

# Step 0: setup tooling (rust)
FROM rust:1.85 AS base-rust
WORKDIR /app

RUN cargo install sccache
RUN cargo install cargo-chef
ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef prepare --recipe-path recipe.json

# Step 2: Build crate
FROM base-rust AS builder-rust

COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook --features openrpc --recipe-path recipe.json

COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --features openrpc --bin openrpc-testgen-runner

# Step 3: setup tooling (cairo)
FROM rust:1.85 AS base-cairo
WORKDIR /app

RUN apt-get update && apt-get install -y git wget
RUN wget https://github.com/asdf-vm/asdf/releases/download/v0.16.7/asdf-v0.16.7-linux-386.tar.gz
RUN tar -xvpf asdf-v0.16.7-linux-386.tar.gz

RUN ./asdf plugin add scarb
RUN ./asdf install scarb 2.8.4
RUN ./asdf set -u scarb 2.8.4

# Step 4: Build cairo contracts
FROM base-cairo AS builder-cairo

COPY . .
RUN ./asdf exec scarb build

# Step 5: runner
FROM debian:bookworm-slim

WORKDIR /app

COPY --from=builder-rust /app/target/debug/openrpc-testgen-runner .
COPY --from=builder-cairo /app/target/dev target/dev

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /tini
RUN chmod +x /tini

ENTRYPOINT ["/tini", "--", "./openrpc-testgen-runner"]
