#!/usr/bin/env python3
"""Reconstruct backend-tested source using private copies; no Rust on host."""
import importlib.util
import json
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = Path(__file__).resolve().parent
TESTED = "72f507d109012b0ca3d82d5827f1d261a857b3c0e3bc397f942c95b98361a602"
PATHS = ["Cargo.toml", "Cargo.lock", "apps", "crates", "vendor", "tests/fixtures", "scripts/install-harness.sh"]
ARCHIVES = {
    "apps/server/src/worker_execution/runtime/mcp.rs": "pre-lint-mcp.rs",
    "apps/server/src/worker_execution/runtime/fixtures/session_agent.py": "pre-delivery-session-agent.py",
    "apps/server/src/worker_fixture/session_vertical.rs": "pre-delivery-session-vertical.rs",
    "apps/server/src/worker_fixture/broker_client.rs": "pre-delivery-broker-client.rs",
}
spec = importlib.util.spec_from_file_location("sandbox_launcher", ROOT / "scripts/test_web_sandbox.py")
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)
with tempfile.TemporaryDirectory(prefix="native-mcp-current-", dir=ROOT / "target/web-checks") as first:
    source, current = launcher.source_snapshot(first, PATHS)
    for relative, archive in ARCHIVES.items():
        (source / relative).write_bytes((EVIDENCE / archive).read_bytes())
    launcher.ROOT = source
    with tempfile.TemporaryDirectory(prefix="native-mcp-backend-", dir=ROOT / "target/web-checks") as second:
        _, reconstructed = launcher.source_snapshot(second, PATHS)
    assert reconstructed == TESTED, (reconstructed, TESTED)
print(json.dumps({
    "current_source_sha256": current,
    "backend_source_sha256": TESTED,
    "reconstructed_backend_source_sha256": reconstructed,
    "archived_paths": ARCHIVES,
    "scope": "799 passing backend tests predate a map_err-to-inspect_err lint change, interruption on successful Unix response delivery failure, and the fourth ignored runtime scenario exercising that failure. This proof establishes the exact source delta; it does not certify the final changes. Production runtime acceptance and final Clippy must verify those separately.",
}, indent=2))
