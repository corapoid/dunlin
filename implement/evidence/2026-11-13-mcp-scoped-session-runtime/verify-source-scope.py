#!/usr/bin/env python3
"""Reconstruct the tested source in private copies; never build Rust on host."""
import importlib.util
import json
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[3]
TESTED = "439b6a8e099d152771cf0c1e8d0dc89f9505f22c4149eae06e6591c8fb108464"
PATHS = ["Cargo.toml", "Cargo.lock", "apps", "crates", "vendor", "tests/fixtures", "scripts/install-harness.sh"]
spec = importlib.util.spec_from_file_location("sandbox_launcher", ROOT / "scripts/test_web_sandbox.py")
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)
relative = Path("apps/server/src/store/remote_bindings/resolver/sessions/open.rs")
needle = b"    #[cfg(test)]\n    pub(super) fn into_legacy"
with tempfile.TemporaryDirectory(prefix="mcp-scope-current-", dir=ROOT / "target/web-checks") as first:
    source, current = launcher.source_snapshot(first, PATHS)
    path = source / relative
    data = path.read_bytes()
    assert data.count(needle) == 1
    path.write_bytes(data.replace(needle, b"    pub(super) fn into_legacy", 1))
    launcher.ROOT = source
    with tempfile.TemporaryDirectory(prefix="mcp-scope-tested-", dir=ROOT / "target/web-checks") as second:
        _, reconstructed = launcher.source_snapshot(second, PATHS)
    assert reconstructed == TESTED, (reconstructed, TESTED)
print(json.dumps({
    "current_source_sha256": current,
    "tested_source_sha256": TESTED,
    "reconstructed_tested_source_sha256": reconstructed,
    "changes": [{"path": str(relative), "change": "Add #[cfg(test)] to the legacy fixture-only into_legacy converter."}],
    "scope": "The full backend and focused tests compiled the same test function bodies with cfg(test). Final non-test compilation/runtime separately verify removal of the unused converter from production. This does not claim byte-identical executables.",
}, indent=2))
