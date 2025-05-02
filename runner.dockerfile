FROM rust:latest AS builder

RUN apt-get update && apt-get install -y git wget

WORKDIR /app

RUN wget https://github.com/asdf-vm/asdf/releases/download/v0.16.7/asdf-v0.16.7-linux-386.tar.gz
RUN tar -xvpf asdf-v0.16.7-linux-386.tar.gz

RUN ./asdf plugin add scarb
RUN ./asdf install scarb 2.8.4
RUN ./asdf set -u scarb 2.8.4

COPY . .

RUN ./asdf exec scarb build
RUN cargo build --release --bin openrpc-testgen-runner --features openrpc

FROM debian:bookworm-slim

WORKDIR /app

COPY --from=builder /app/target/release/openrpc-testgen-runner .
COPY --from=builder /app/target/dev target/dev

ENTRYPOINT ["./openrpc-testgen-runner"]
