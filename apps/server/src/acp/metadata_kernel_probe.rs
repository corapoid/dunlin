//! Probe programs contain fixture I/O assertions, never sandbox policy tokens.
pub(super) const PROBE: &str = r#"
import errno, json, os, sys
common, git, source_secret, other_secret, relay_db, mode = sys.argv[1:]

def denied(label, operation):
    try:
        operation()
    except OSError as error:
        assert error.errno in (errno.EACCES, errno.EPERM), (label, error.errno)
    else:
        raise AssertionError(label + ' escaped the production sandbox')

# Metadata Read and Write are distinct from ordinary workspace read/write.
assert open('README').read() == 'fixture'
with open('workspace-kernel-probe', 'w') as target:
    target.write('workspace remains writable')
for root in (common, git):
    assert open(os.path.join(root, 'HEAD')).read()
    existing = os.path.join(root, 'kernel-existing')
    assert open(existing).read() == 'original'
    created = os.path.join(root, 'kernel-created')
    if mode == 'read':
        denied('existing metadata append', lambda: open(existing, 'a').write('write'))
        denied('new metadata file', lambda: open(created, 'w').write('write'))
    else:
        with open(existing, 'a') as target:
            target.write('write')
        with open(created, 'w') as target:
            target.write('write')

# These files exist before sandbox startup. EPERM/EACCES proves an actual deny,
# rather than a missing-path error or an assertion about profile text alone.
for label, path in (('source workspace', source_secret),
                    ('other project', other_secret), ('relay DB', relay_db)):
    denied(label + ' read', lambda: open(path, 'rb').read(1))
    denied(label + ' append', lambda: open(path, 'ab').write(b'forbidden'))
print(json.dumps({'kernel': 'seatbelt', 'metadata': mode, 'protected_files': 3}))
"#;

#[cfg(target_os = "linux")]
pub(super) const CONTROL: &str = r#"
import json, os, sys
common, git, source_secret, other_secret, relay_db, mode = sys.argv[1:]
assert open('README').read() == 'fixture'
for root in (common, git):
    assert open(os.path.join(root, 'HEAD')).read()
    assert open(os.path.join(root, 'kernel-existing')).read() == 'original'
    with open(os.path.join(root, 'kernel-existing'), 'ab'): pass
    marker = os.path.join(root, 'kernel-control')
    with open(marker, 'w') as target: target.write('control')
    os.unlink(marker)
for path in (source_secret, other_secret, relay_db):
    assert open(path, 'rb').read(1)
    with open(path, 'ab'): pass
print(json.dumps({'control':'unconfined','protected_files':3}))
"#;
