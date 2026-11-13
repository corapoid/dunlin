# Tool provisioning only. Test execution uses --network=none and readonly mounts.
FROM rust:1.99-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends git python3
RUN rustup component add clippy
