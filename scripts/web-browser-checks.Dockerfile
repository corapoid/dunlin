# Browser and tool provisioning only; the launcher runs the resulting image offline.
FROM node:22-bookworm-slim AS node
FROM dunlin-web-checks:rust-1.99
RUN apt-get update && apt-get install -y --no-install-recommends chromium
COPY --from=node /usr/local/bin/node /usr/local/bin/node
