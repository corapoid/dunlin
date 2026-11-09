#!/usr/bin/env python3
"""Manual-only deployed relay acceptance. Never imported by release_qa or CI."""
from remote_e2e.runner import main

if __name__ == "__main__":
    raise SystemExit(main())
