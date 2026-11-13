#!/usr/bin/env python3
"""Generate production seatbelt policy in isolated Linux, execute it on macOS.

No host Rust build/test is performed. All I/O targets are newly owned fixtures;
policy strings and probe code come verbatim from the shared Rust implementation.
This proves profile/kernel behavior, not the full macOS binary or portable FDs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "DUNLIN_MACOS_PROFILE_BUNDLE:"
LOG_LIMIT = 8 * 1024 * 1024
BUNDLE_LIMIT = 384 * 1024


def digest(value):
    return hashlib.sha256(value).hexdigest()


def fixture_paths(root):
    home = root / "home"
    source = home / "Documents/source"
    return {"schema_version": 1, "fixture_root": str(root), "home": str(home),
            "checkout": str(root / "checkout"), "source": str(source),
            "other": str(root / "other-project"), "db": str(root / "relay.db"),
            "common": str(source / ".git"), "git": str(source / ".git/worktrees/checkout")}


def command(argv, *, cwd, environment):
    result = subprocess.run(argv, cwd=cwd, env=environment, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False)
    if result.returncode:
        raise ValueError(f"Fixture/probe command failed ({result.returncode}): {result.stderr.decode(errors='replace')[:8192]}")
    if len(result.stdout) > BUNDLE_LIMIT or len(result.stderr) > BUNDLE_LIMIT:
        raise ValueError("Fixture/probe output exceeds its bounded receipt budget")
    return result.stdout


def create_fixture(root):
    root.mkdir(mode=0o755)
    paths = fixture_paths(root)
    for path in [Path(paths["source"]), Path(paths["other"]), root / "server/tools", root / "server/node", root / "server/agent-homes", root / "temp"]:
        path.mkdir(parents=True, mode=0o755)
    (root / "server/settings.json").write_text(json.dumps({"isolated_agents": True, "host_accounts": False}))
    environment = {"HOME": paths["home"], "PATH": "/usr/bin:/bin", "TMPDIR": str(root / "temp")}
    source = Path(paths["source"])
    git = ["/usr/bin/git", "-c", "user.name=Mac kernel fixture", "-c", "user.email=fixture@example.invalid",
           "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false"]
    command([*git, "init", "-q", "-b", "main"], cwd=source, environment=environment)
    (source / "README").write_text("fixture")
    command([*git, "add", "README"], cwd=source, environment=environment)
    command([*git, "commit", "-qm", "fixture"], cwd=source, environment=environment)
    command([*git, "worktree", "add", "--detach", paths["checkout"], "HEAD"], cwd=source, environment=environment)
    actual_git = command([*git, "rev-parse", "--absolute-git-dir"], cwd=paths["checkout"], environment=environment).decode().strip()
    if Path(actual_git).resolve(strict=True) != Path(paths["git"]):
        raise ValueError("Git fixture did not produce the exact expected linked metadata directory")
    (source / "source-only-secret").write_text("private-source-workspace")
    (Path(paths["other"]) / "other-project-secret").write_text("fixture-only-private")
    with sqlite3.connect(paths["db"]) as database:
        database.execute("CREATE TABLE fixture (marker TEXT NOT NULL)")
        database.execute("INSERT INTO fixture VALUES ('fixture-only-relay-database')")
    for metadata in [Path(paths["common"]), Path(paths["git"])]:
        (metadata / "kernel-existing").write_text("original")
    (root / "fixture.json").write_text(json.dumps(paths))
    # The container receives a read-only view as UID 65534. Only this newly
    # created, symlink-free tree is made readable; no host-private file changes.
    for path in [root, *root.rglob("*")]:
        if path.is_symlink():
            raise ValueError("Unexpected symlink in owned Git fixture")
        path.chmod(0o755 if path.is_dir() else 0o644)
    return paths


def export_profiles(args, root, log_path):
    argv = [sys.executable, "-I", str(ROOT / "scripts/test_web_sandbox.py"), "--suite", "macos-profile",
            "--macos-profile-fixture", str(root), "--docker-host", args.docker_host,
            "--registry", str(args.registry), "--wall-seconds", str(args.wall_seconds)]
    if args.rust_test:
        argv += ["--rust-test", args.rust_test]
    if args.rust_image:
        argv += ["--rust-image", args.rust_image]
    if args.rust_home:
        argv += ["--rust-home", args.rust_home]
    if args.tmp_root:
        argv += ["--tmp-root", str(args.tmp_root)]
    process = subprocess.Popen(argv, env={"HOME": str(root / "home"), "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"},
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               start_new_session=True)
    deadline = time.monotonic() + args.wall_seconds + 45
    total = 0
    try:
        with selectors.DefaultSelector() as selector, log_path.open("xb") as log:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValueError("Linux profile export exceeded its deadline")
                for key, _ in selector.select(min(remaining, 1)):
                    block = os.read(key.fileobj.fileno(), 65536)
                    if not block:
                        selector.unregister(key.fileobj)
                        continue
                    total += len(block)
                    if total > LOG_LIMIT:
                        raise ValueError("Linux profile export log exceeds its budget")
                    log.write(block)
        if process.wait(timeout=5) != 0:
            raise ValueError(f"Linux profile export failed; inspect {log_path}")
    finally:
        if process.poll() is None:
            process.terminate()  # The launcher unwinds its own scoped Docker cleanup.
            try:
                process.wait(timeout=35)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
    raw = log_path.read_text()
    bundles = [line.removeprefix(PREFIX) for line in raw.splitlines() if line.startswith(PREFIX)]
    sources = re.findall(r"source_sha256=([a-f0-9]{64})", raw)
    if len(bundles) != 1 or len(sources) != 1 or len(bundles[0].encode()) > BUNDLE_LIMIT:
        raise ValueError("Expected exactly one bounded production bundle/source receipt")
    return bundles[0].encode(), sources[0]


def shared_probe_programs():
    source = (ROOT / "apps/server/src/acp/metadata_kernel_probe.rs").read_text()
    if len(source.encode()) > 32 * 1024:
        raise ValueError("Shared probe source exceeds its budget")
    result = {}
    for name in ["PROBE", "CONTROL"]:
        values = re.findall(r'pub\(super\) const ' + name + r': &str = r#"(.*?)"#;', source, re.DOTALL)
        if len(values) != 1:
            raise ValueError("Cannot identify the exact shared Rust probe program")
        result[name] = values[0]
    return result

def validate_bundle(encoded, paths):
    if len(encoded) > BUNDLE_LIMIT:
        raise ValueError("Production profile bundle exceeds its budget")
    bundle = json.loads(encoded)
    expected = {"schema_version", "generator", "authority_platform", "fixture_root", "profile_home", "process_home", "program", "python_args", "probe", "control", "profiles"}
    if not isinstance(bundle, dict) or set(bundle) != expected or bundle["schema_version"] != 1 or bundle["generator"] != "production_macos_harness_profile" or bundle["authority_platform"] != "linux-fixture-only":
        raise ValueError("Unexpected profile generator contract")
    root = Path(paths["fixture_root"])
    if bundle["fixture_root"] != str(root) or bundle["profile_home"] != paths["home"] or bundle["program"] != "/usr/bin/python3" or bundle["python_args"] != ["-I", "-S"]:
        raise ValueError("Profile bundle changed its scope or executable")
    home = Path(bundle["process_home"])
    if home.parent != root / "server/agent-homes" or re.fullmatch(r"usr-[A-Za-z0-9_-]{1,124}", home.name) is None:
        raise ValueError("Profile process HOME escaped the owned fixture")
    for name in ["probe", "control"]:
        if not isinstance(bundle[name], str) or not 1 <= len(bundle[name].encode()) <= 16 * 1024:
            raise ValueError("Invalid bounded fixture probe program")
    shared = shared_probe_programs()
    if bundle["probe"] != shared["PROBE"] or bundle["control"] != shared["CONTROL"]:
        raise ValueError("Exported probe code differs from the exact shared Rust source")
    if not isinstance(bundle["profiles"], list) or len(bundle["profiles"]) != 2:
        raise ValueError("Exactly Read/Write production profiles are required")
    for profile, mode in zip(bundle["profiles"], ["read", "write"]):
        if not isinstance(profile, dict) or set(profile) != {"permission", "profile_sha256", "profile", "cwd", "args"}:
            raise ValueError("Unexpected profile receipt fields")
        expected_args = [paths["common"], paths["git"], f"{paths['source']}/source-only-secret", f"{paths['other']}/other-project-secret", paths["db"], mode]
        if profile["permission"] != mode or profile["cwd"] != paths["checkout"] or profile["args"] != expected_args:
            raise ValueError("Profile fixture paths/permission were retargeted")
        policy = profile["profile"]
        if not isinstance(policy, str) or not 1 <= len(policy.encode()) <= 128 * 1024 or "\x00" in policy or digest(policy.encode()) != profile["profile_sha256"]:
            raise ValueError("Production policy bytes do not match their hash/budget")
    return bundle


def prove(bundle, paths, output):
    process_home = Path(bundle["process_home"])
    process_home.mkdir(mode=0o700)
    environment = {"HOME": str(process_home), "PATH": "/usr/bin:/bin", "TMPDIR": str(Path(paths["fixture_root"]) / "temp")}
    protected = [Path(paths["source"]) / "source-only-secret", Path(paths["other"]) / "other-project-secret", Path(paths["db"])]
    before = [digest(path.read_bytes()) for path in protected]
    first = bundle["profiles"][0]
    control = command([bundle["program"], *bundle["python_args"], "-c", bundle["control"], *first["args"]], cwd=first["cwd"], environment=environment)
    if json.loads(control) != {"control": "unconfined", "protected_files": 3}:
        raise ValueError("Unsandboxed positive control did not prove fixture accessibility")
    receipts = []
    for profile in bundle["profiles"]:
        # Pass the exact exported bytes. No profile tokens or paths are patched.
        policy_bytes = profile["profile"].encode()
        (output / f"{profile['permission']}.sbpl").write_bytes(policy_bytes)
        result = command(["/usr/bin/sandbox-exec", "-p", profile["profile"], bundle["program"], *bundle["python_args"], "-c", bundle["probe"], *profile["args"]], cwd=profile["cwd"], environment=environment)
        receipt = json.loads(result)
        if receipt != {"kernel": "seatbelt", "metadata": profile["permission"], "protected_files": 3}:
            raise ValueError("Unexpected actual kernel receipt")
        for metadata in [Path(paths["common"]), Path(paths["git"])]:
            mode = profile["permission"]
            expected = "original" if mode == "read" else "originalwrite"
            if (metadata / "kernel-existing").read_text() != expected:
                raise ValueError("Actual metadata file content contradicts the kernel receipt")
            created = metadata / "kernel-created"
            if (mode == "read" and created.exists()) or (mode == "write" and created.read_text() != "write"):
                raise ValueError("Actual metadata creation contradicts the kernel receipt")
        receipts.append({"permission": profile["permission"], "profile_sha256": digest(policy_bytes), "receipt": receipt})
    if before != [digest(path.read_bytes()) for path in protected]:
        raise ValueError("A protected fixture file changed during the probe")
    with sqlite3.connect(f"file:{paths['db']}?mode=ro", uri=True) as database:
        if database.execute("PRAGMA integrity_check").fetchone() != ("ok",):
            raise ValueError("Protected fixture database was corrupted")
    return {"control": json.loads(control), "profiles": receipts, "protected_files_unchanged": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--docker-host", required=True)
    parser.add_argument("--registry", type=Path, default=Path.home() / ".cargo/registry")
    parser.add_argument("--rust-test", help="Optional normal Linux precheck; the exact profile export always follows")
    parser.add_argument("--rust-image")
    parser.add_argument("--rust-home")
    parser.add_argument("--tmp-root", type=Path)
    parser.add_argument("--wall-seconds", type=int, choices=range(1, 901), metavar="1..900", default=900)
    args = parser.parse_args()
    if sys.platform != "darwin" or not Path("/usr/bin/sandbox-exec").is_file():
        parser.error("This proof requires the actual macOS sandbox-exec adapter")
    if not args.docker_host.startswith("unix://"):
        parser.error("Use an explicit local UNIX Docker socket")
    if args.rust_test is not None and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_:]{0,255}", args.rust_test) is None:
        parser.error("Rust precheck must be a bounded test identifier")
    parent = ROOT / "target/web-checks"
    parent.mkdir(parents=True, exist_ok=True)
    identity = uuid.uuid4().hex
    root = (parent / f"macos-metadata-{identity}").resolve()
    output = parent / f"macos-profile-proof-{identity}"
    output.mkdir(mode=0o700)
    try:
        paths = create_fixture(root)
        encoded, source = export_profiles(args, root, output / "linux-export.log")
        bundle = validate_bundle(encoded, paths)
        (output / "production-bundle.json").write_bytes(encoded)
        receipt = prove(bundle, paths, output)
        receipt.update({"scope": "production-generated-profile-on-macOS-kernel-only", "source_sha256": source,
                        "bundle_sha256": digest(encoded), "full_macos_rust_test": "not_executed", "host_rust_build": False})
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"PASS actual macOS production metadata profiles: {output / 'receipt.json'}")
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Mac profile proof failed: {error}; evidence: {output}", file=sys.stderr)
        return 1
    finally:
        if root.exists():
            shutil.rmtree(root)


if __name__ == "__main__":
    sys.exit(main())
