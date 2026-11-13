"""Reconstruct earlier tested snapshots in a disposable private source copy."""
import difflib
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile

root = Path(__file__).resolve().parents[3]
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

def replace_once(path, before, after):
    value = path.read_text()
    assert value.count(before) == 1, path
    path.write_text(value.replace(before, after))

with tempfile.TemporaryDirectory(prefix='dunlin-v5-open-proof-') as temporary:
    source, final_hash = launcher.source_snapshot(temporary, paths)
    policy = source / 'apps/server/src/worker_exports/policy.rs'
    tests = source / 'apps/server/src/worker_exports/policy/tests.rs'
    final_files = {path: path.read_text() for path in (policy, tests)}
    replace_once(policy, '''    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_mcp_broker"
    )]
''', '''    #[serde(default, skip_serializing_if = "Option::is_none")]
''')
    replace_once(policy, '''fn explicit_mcp_broker<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<bool>, D::Error> {
    bool::deserialize(deserializer).map(Some)
}
''', '')
    replace_once(tests, '''    for version in [1, 2] {
        let mut value: serde_json::Value = serde_json::from_slice(RAW).unwrap();
        value["schema_version"] = serde_json::json!(version);
        value["mcp_broker"] = serde_json::Value::Null;
        assert!(PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).is_err());
    }
''', '')
    server_hash = digest(source)
    assert server_hash == '74fe43a06c603d55645f2ef733375935471aa684af5e944da09833b6d24cb00f', server_hash
    patch = ''.join(''.join(difflib.unified_diff(path.read_text().splitlines(True), final_files[path].splitlines(True), fromfile=str(path.relative_to(source)), tofile=str(path.relative_to(source)))) for path in (policy, tests))
    output = Path(__file__).parent
    (output / 'explicit-null-policy.patch').write_text(patch)
    for relative in ['apps/server/src/worker_client/broker.rs', 'apps/server/src/worker_client/broker/tests.rs']:
        (source / relative).unlink()
    replace_once(source / 'apps/server/src/worker_client.rs', '''#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "v5 socket owner and runner proxy integration is in progress"
    )
)]
pub(crate) mod broker;
''', '')
    protocol_hash = digest(source)
    assert protocol_hash == 'f6a01e17fab8461e08073ba1f86d8826fefe2ced420747228d8e2a58016f8790', protocol_hash
    proof = {'final_source_sha256': final_hash, 'full_server_source_sha256': server_hash, 'domain_protocol_source_sha256': protocol_hash, 'full_server_difference': 'explicit-null-policy.patch', 'domain_protocol_difference': 'Only worker_client broker module/declaration and the policy null rejection; crates/domain and crates/relay-protocol unchanged.', 'method': 'Exact reverse in disposable source copy using the sandbox launcher input set and digest algorithm; original worktree never mutated.'}
    (output / 'source-scope-proof.json').write_text(json.dumps(proof, indent=2) + '\n')
    print(json.dumps(proof, indent=2))
