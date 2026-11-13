# Explicit provisioning; actual tests use --pull=never and --network=none.
FROM dunlin-web-checks:browser
RUN apt-get update && apt-get install -y --no-install-recommends bubblewrap && rm -rf /var/lib/apt/lists/*
