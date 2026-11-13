"""Reconstruct tested v5 wire snapshots without modifying the worktree."""
import difflib
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile

root = Path(__file__).resolve().parents[3]
out = Path(__file__).parent
spec = importlib.util.spec_from_file_location('launcher', root / 'scripts/test_web_sandbox.py')
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)
paths = ['Cargo.toml', 'Cargo.lock', 'apps', 'crates', 'vendor', 'tests/fixtures', 'scripts/install-harness.sh']

def digest(source):
    result = hashlib.sha256()
    for file in sorted(source.rglob('*')):
        if file.is_symlink():
            result.update(str(file.relative_to(source)).encode())
            result.update(file.readlink().as_posix().encode())
        elif file.is_file():
            result.update(str(file.relative_to(source)).encode())
            result.update(file.read_bytes())
    return result.hexdigest()

def reverse(source, replacements, name):
    patch = ''
    for relative, new, old, count in replacements:
        file = source / relative
        current = file.read_text()
        assert current.count(new) == count, relative
        previous = current.replace(new, old)
        patch += ''.join(difflib.unified_diff(previous.splitlines(True), current.splitlines(True), fromfile=relative, tofile=relative))
        file.write_text(previous)
    (out / name).write_text(patch)
    return digest(source)

with tempfile.TemporaryDirectory(prefix='dunlin-v5-wire-proof-') as temporary:
    source, final_hash = launcher.source_snapshot(temporary, paths)
    runtime_hash = reverse(source, [
        ('apps/web/src/components/HostHealthLive.tsx', ", 5: 'Sesje trwałe'", '', 1),
    ], 'health-label-only.patch')
    assert runtime_hash == '2359ca5689209ce88683cbf66d56b29d5d92749af9679fddb32319e5e5e52658', runtime_hash
    added = source / 'apps/server/src/worker_fixture/broker_transport.rs'
    (out / 'manual-probe-test.patch').write_text(''.join(difflib.unified_diff([], added.read_text().splitlines(True), fromfile='/dev/null', tofile='apps/server/src/worker_fixture/broker_transport.rs')))
    added.unlink()
    server_hash = reverse(source, [
        ('apps/server/src/worker_fixture.rs', 'mod broker_transport;\n', '', 1),
        ('crates/relay-protocol/src/worker/execution/tests.rs', '    assert_eq!(worker_rate_limit(5), 4096);\n    assert_eq!(worker_rate_limit(6), 120);\n', '', 1),
        ('crates/relay-protocol/src/worker/session/broker.rs', '//! Hello5 requires explicit transport opt-in. The controller must resolve', '//! No active handshake selects this contract yet. The controller must resolve', 1),
        ('crates/relay-protocol/src/worker/session/broker/open.rs', '//! Version-specific v5 handshake/open shapes; MCP admission also needs a bound broker.', '//! Version-specific handshake/open shapes. No active WS handler selects v5 yet.', 1),
        ('apps/server/src/worker_client/broker.rs', '//! V5 negotiation alone does not attach this dispatcher to the runner runtime.', '//! No active socket selects v5 until the controller and runner bridges exist.', 1),
    ], 'tests-and-comments.patch')
    assert server_hash == '2901151ba42551a657f1a6950686baa2b1b2f700b8b0e4845cb14372c6299044', server_hash
    clippy_hash = reverse(source, [
        ('apps/web/src/lib/host-health.ts', '1 | 2 | 3 | 4 | 5', '1 | 2 | 3 | 4', 2),
        ('apps/web/src/lib/host-health.ts', '!integer(c.protocol_version, 1, 5)', '!integer(c.protocol_version, 1, 4)', 1),
        ('apps/web/tests/host-health.test.ts', '''  const broker = decodeHostHealth({ ...health, connection: { ...health.connection, protocol_version: 5 } }, 'workspace', 'host');
  assert.equal(broker.connection?.protocol_version, 5);
  assert.equal(broker.host.execution_ready, false);
''', '', 1),
        ('apps/web/tests/host-health.test.ts', '{ ...health, connection: { ...health.connection, protocol_version: 6 } }', '{ ...health, connection: { ...health.connection, protocol_version: 5 } }', 1),
    ], 'health-codec-only.patch')
    assert clippy_hash == '04bb0f058a25e5fab881cba99b16e6d588e0d0831ef020751b1acf92e244c37a', clippy_hash
    proof = {'final_source_sha256': final_hash, 'runtime_source_sha256': runtime_hash, 'runtime_difference': 'health-label-only.patch', 'server_source_sha256': server_hash, 'clippy_source_sha256': clippy_hash, 'server_additional_difference': ['manual-probe-test.patch', 'tests-and-comments.patch'], 'clippy_additional_difference': 'health-codec-only.patch', 'method': 'Exact reverse in a disposable source copy; same input set/digest as the sandbox launcher. Original worktree never mutated.'}
    (out / 'source-scope-proof.json').write_text(json.dumps(proof, indent=2) + '\n')
    print(json.dumps(proof, indent=2))
