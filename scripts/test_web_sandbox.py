#!/usr/bin/env python3
"""Offline web/core checks in cached Docker toolchains, with bounded scratch.

Trusted parent launcher: never imports project code or forwards host environment,
credentials, sockets or the user's home into the test container. No image pulls,
package installs, Docker context changes or operations on other containers.
"""
import argparse
import hashlib
import json
import os
import re
import shlex
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]
RUNTIME_POLICIES = {
    "runtime-seccomp.json": "3f308f5eb84680f293dccb841d6dda14f2326415b027a42e2d247e431df72858",
    "runtime.apparmor": "30079a3d44e0fce75b7a4e0e716726bb1fdf03f5d8e2813a5cdb1086b83540a9",
}


def runtime_security_options(directory):
    directory = Path(directory).resolve(strict=True)
    for name, expected in RUNTIME_POLICIES.items():
        path = directory / name
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"Runtime policy mismatch: {name}; provision the pinned profiles first")
    return ["--security-opt", "apparmor=dunlin-runtime-checks",
            "--security-opt", "systempaths=unconfined",
            "--security-opt", f"seccomp={directory / 'runtime-seccomp.json'}"]


def rust_test_arg(args):
    value = getattr(args, "rust_test", None)
    if value is None:
        return ""
    if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_:]{0,255}", value) is None:
        raise ValueError("Rust test filter must be a bounded test identifier")
    if args.suite == "runtime":
        if not value.rsplit("::", 1)[-1].startswith("production_browser_acceptance_runtime_"):
            raise ValueError("Focused runtime filters must select production_browser_acceptance_runtime_ acceptance tests")
    elif args.suite not in {"server", "sessions", "domain", "usage", "workspace", "mcp", "macos-profile"}:
        raise ValueError("Rust test filter is not supported by this suite")
    return " " + shlex.quote(value)

def runtime_browser_enabled(args):
    enabled = bool(getattr(args, "runtime_browser", False))
    if enabled and (args.suite != "runtime" or getattr(args, "rust_test", None) not in {
        None, "production_browser_acceptance_runtime_remote_session_journal",
        "production_browser_acceptance_runtime_remote_mcp_product_pipeline",
        "production_browser_acceptance_runtime_remote_mcp_product_urls",
        "production_browser_acceptance_runtime_mcp_oauth_revocation",
        "production_browser_acceptance_runtime_remote_",
    }):
        raise ValueError("Runtime browser requires runtime suite and a supported remote-session browser filter or full regression")
    return enabled

def macos_profile_fixture(args):
    """The sole additional host mount is a bounded, owned test-fixture tree."""
    value = getattr(args, "macos_profile_fixture", None)
    if args.suite != "macos-profile":
        if value is not None:
            raise ValueError("--macos-profile-fixture is restricted to --suite macos-profile")
        return None
    if value is None:
        raise ValueError("macos-profile requires an explicit --macos-profile-fixture")
    root = Path(value)
    allowed = (ROOT / "target/web-checks").resolve()
    if (not root.is_absolute() or root.is_symlink() or root.resolve(strict=True) != root
            or root.parent != allowed or re.fullmatch(r"macos-metadata-[a-f0-9]{32}", root.name) is None):
        raise ValueError("Mac profile fixture must be a canonical owned target/web-checks/macos-metadata-UUID directory")
    size = 0
    for path in [root, *root.rglob("*")]:
        if path.is_symlink():
            raise ValueError("Mac profile fixture may not contain symlinks")
        metadata = path.stat()
        if metadata.st_uid != os.getuid() or not (path.is_dir() or path.is_file()):
            raise ValueError("Mac profile fixture must contain only current-user regular files/directories")
        if path.is_file():
            size += metadata.st_size
            if metadata.st_size > 1024 * 1024 or size > 8 * 1024 * 1024:
                raise ValueError("Mac profile fixture exceeds its read-only mount budget")
    descriptor = root / "fixture.json"
    if descriptor.stat().st_size > 16 * 1024:
        raise ValueError("Mac profile descriptor exceeds its budget")
    data = json.loads(descriptor.read_text())
    if data.get("schema_version") != 1 or data.get("fixture_root") != str(root) or data.get("home") != str(root / "home"):
        raise ValueError("Mac profile fixture descriptor has a different scope/home")
    if not (root / "home").is_dir():
        raise ValueError("Mac profile fixture HOME is missing")
    return root

def source_snapshot(parent, paths):
    """Private, immutable-to-target source copy; never follow repository symlinks."""
    snapshot = Path(parent) / "source"
    snapshot.mkdir()
    copied = 0

    def bounded_copy(source, destination):
        nonlocal copied
        size = Path(source).stat().st_size
        if size > 128 * 1024**2 or copied + size > 2 * 1024**3:
            raise ValueError("Source file/tree exceeds its parent-side copy budget")
        copied += size
        return shutil.copyfile(source, destination)

    ignored = shutil.ignore_patterns(".git", "node_modules", "target", "dist", ".env", ".env.*", "*.db", "*.db-wal", "*.db-shm")
    for relative in paths:
        source = ROOT / relative
        destination = snapshot / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if source.is_symlink():
            raise ValueError(f"Top-level source input must not be a symlink: {relative}")
        if source.is_dir():
            shutil.copytree(source, destination, symlinks=True, ignore=ignored, copy_function=bounded_copy)
        else:
            bounded_copy(source, destination)
    digest = hashlib.sha256()
    size = 0
    for file in sorted(snapshot.rglob("*")):
        if file.is_symlink():
            digest.update(str(file.relative_to(snapshot)).encode())
            digest.update(file.readlink().as_posix().encode())
        elif file.is_file():
            size += file.stat().st_size
            if size > 2 * 1024**3:
                raise ValueError("Source snapshot exceeds its 2 GiB parent-side copy budget")
            digest.update(str(file.relative_to(snapshot)).encode())
            with file.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
    return snapshot, digest.hexdigest()


def mount(source, target):
    source = Path(source).resolve(strict=True)
    if "," in str(source):
        raise ValueError("Mount paths containing commas are not supported")
    return ["--mount", f"type=bind,src={source},dst={target},readonly"]


def file_size_limit(suite):
    return 2147483648 if suite in {"runtime", "workspace", "browser", "server", "macos-profile"} else 134217728

def memory_limit(suite, rust):
    return "3584m" if suite in {"server", "browser", "runtime"} else "3328m" if suite == "macos-profile" else "2304m" if suite == "clippy" else "1536m" if rust else "512m"


def run(args):
    test_filter = rust_test_arg(args)
    runtime_browser = runtime_browser_enabled(args)
    profile_fixture = macos_profile_fixture(args)
    docker = shutil.which("docker")
    if not docker:
        raise ValueError("Docker CLI is required; this launcher never installs it")
    with tempfile.TemporaryDirectory(prefix="dunlin-check-parent-", dir=args.tmp_root) as parent:
        environment = {"HOME": parent, "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"}
        cli = [docker, "--host", args.docker_host] if args.docker_host else [docker]
        rust = args.suite in {"domain", "server", "clippy", "browser", "sessions", "runtime", "usage", "workspace", "mcp", "macos-profile"}
        image = args.runtime_image if args.suite == "runtime" else args.browser_image if args.suite == "browser" else args.rust_image if rust or args.suite in {"tooling", "docs"} else args.node_image
        inspected = subprocess.run(cli + ["image", "inspect", image, "--format", "{{json .}}"],
                                   env=environment, check=True, capture_output=True, text=True, timeout=15)
        metadata = json.loads(inspected.stdout)
        if metadata["Os"] != "linux":
            raise ValueError("Checks require a cached Linux toolchain image")
        # Pin this execution to the local content id, not a moving registry tag.
        image_id = metadata["Id"]
        platform = f"linux/{metadata['Architecture']}"
        name = "dunlin-check-" + uuid.uuid4().hex
        wall = args.wall_seconds or (600 if rust else 60)
        memory = memory_limit(args.suite, rust)
        scratch = "1536m" if args.suite in {"runtime", "server", "browser", "macos-profile"} else "768m" if rust else "128m"
        # Workspace import preflight creates sparse >1GiB fixtures. The large
        # relay test ELF also exceeds 128MiB. Logical file size changes neither
        # tmpfs/RAM bounds nor which host paths are writable.
        file_size = file_size_limit(args.suite)
        command = cli + [
            "run", "--rm", "--name", name, "--pull=never", "--platform", platform,
            "--network=none", "--read-only", "--memory", memory, "--memory-swap", memory,
            "--cpus=1", "--pids-limit=256" if args.suite == "browser" or runtime_browser else "--pids-limit=96" if rust else "--pids-limit=64",
            "--ulimit", "cpu=600:600" if rust else "cpu=30:30",
            "--ulimit", f"fsize={file_size}:{file_size}", "--ipc=none", "--cap-drop=ALL",
            "--security-opt=no-new-privileges", "--user=65534:65534", "--log-driver=none",
            "--tmpfs", f"/scratch:rw,exec,nosuid,nodev,size={scratch},mode=1777",
            "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777",
            "--workdir=/repo/apps/web" if not rust or args.suite == "browser" else "--workdir=/repo",
        ]
        if args.suite == "runtime":
            command += runtime_security_options(args.runtime_policy_dir)
        if args.suite in {"tooling", "docs"}:
            paths = ["scripts", ".github", "README.md", "CONTRIBUTING.md", "docs", "web/src"]
            if args.suite == "docs":
                paths = ["scripts", ".github", "docs", "implement", "todo", "apps", "crates", "vendor", "tests", "assets", "web", "intent", "licenses", "plugins", "Cargo.lock", "Cargo.toml", "LICENSE", "CHANGELOG.md", "CONTRIBUTING.md", "README.md", "SECURITY.md", "THIRD_PARTY_NOTICES.md", "deny.toml", ".gitignore", ".gitattributes"]
            source, source_digest = source_snapshot(parent, paths)
            command += mount(source, "/repo") + ["--workdir=/repo"]
            variables = ["PATH=/usr/local/bin:/usr/bin:/bin", "PYTHONDONTWRITEBYTECODE=1"]
            payload = ["/usr/bin/python3", "-I", "scripts/check_docs.py"] if args.suite == "docs" else [
                "/bin/sh", "-ec", "python3 -I -m unittest discover -s scripts/tests -p test_ci_workflows.py -v\n"
                "exec python3 -I -m unittest discover -s scripts/tests -p test_web_sandbox_launcher.py -v"]
        elif rust:
            paths = ["Cargo.toml", "Cargo.lock", "apps", "crates", "vendor", "tests/fixtures", "scripts/install-harness.sh"]
            source, source_digest = source_snapshot(parent, paths)
            command += mount(source, "/repo")
            command += mount(args.registry, "/cache/registry")
            if profile_fixture is not None:
                command += mount(profile_fixture, str(profile_fixture))
            package = "-p dunlin-mcp-client -p dunlin-http -p dunlin-vault" if args.suite == "mcp" else "-p dunlin-workspace" if args.suite == "workspace" else "-p dunlin-usage" if args.suite == "usage" else "-p dunlin-session-runtime" if args.suite == "sessions" else "-p dunlin-relay-protocol -p dunlin-domain" if args.suite == "domain" else "-p dunlin-relay -p dunlin-identity -p dunlin-session-runtime -p dunlin-terminal -p dunlin-usage"
            check = f"cargo test {package}{rust_test_arg(args)} --offline --locked" if args.suite != "clippy" else "cargo clippy -p dunlin-relay -p dunlin-terminal -p dunlin-session-runtime -p dunlin-identity -p dunlin-domain -p dunlin-relay-protocol -p dunlin-usage -p dunlin-workspace --all-targets --offline --locked --no-deps -- -D warnings"
            if args.suite == "mcp":
                check += " && cargo clippy -p dunlin-mcp-client -p dunlin-http -p dunlin-vault --all-targets --offline --locked --no-deps -- -D warnings"
            preparation = (
                'target_libdir="$(rustc --print target-libdir)"\n'
                'linker_dir="${target_libdir%/lib}/bin/gcc-ld"\n'
                'test -x "$linker_dir/ld.lld"\n'
                'export RUSTFLAGS="$RUSTFLAGS -C link-arg=-fuse-ld=lld -C link-arg=-B$linker_dir -C link-arg=-Wl,--threads=1"\n'
            )
            if args.suite == "browser" or runtime_browser:
                for relative, dependencies in [("apps/web/node_modules", args.node_modules), ("apps/web/e2e/node_modules", args.browser_node_modules)]:
                    (source / relative).mkdir()
                    command += mount(dependencies, "/repo/" + relative)
                if runtime_browser:
                    preparation += "test -x /usr/bin/chromium\ncd /repo/apps/web\nnode node_modules/vite/bin/vite.js build --configLoader native --outDir /scratch/dist\ncd /repo\n"
                else:
                    preparation += "node node_modules/vite/bin/vite.js build --configLoader native --outDir /scratch/dist\n"
                if args.suite == "browser" and args.browser_test == "all":
                    preparation += "cargo test -p dunlin-relay -p dunlin-identity -p dunlin-session-runtime -p dunlin-terminal -p dunlin-usage --offline --locked\n"
                if args.suite == "browser":
                    check = "cargo test -p dunlin-relay " + ("production_browser_acceptance" + ("_" + args.browser_test if args.browser_test != "all" else "")) + " --offline --locked -- --ignored --nocapture --skip production_browser_acceptance_runtime_"
            if args.suite == "macos-profile":
                if getattr(args, "rust_test", None) is not None:
                    preparation += f"cargo test -p dunlin-relay{rust_test_arg(args)} --offline --locked\n"
                check = "cargo test -p dunlin-relay --offline --locked -- --exact acp::metadata_tests::profile_export::export_production_macos_metadata_profiles --ignored --nocapture"
            if args.suite == "runtime":
                preparation += "test -x /usr/bin/bwrap\n"
                preparation += "bwrap --die-with-parent --new-session --unshare-all --share-net --ro-bind /usr /usr --ro-bind /bin /bin --ro-bind /lib /lib --proc /proc --dev /dev --tmpfs /tmp -- /bin/true\n"
                preparation += "runtime_resources() { df -k /scratch; du -sk /scratch/target; cat /sys/fs/cgroup/memory.peak /sys/fs/cgroup/memory.events; }\n"
                # Link the larger test binary before retaining the production
                # executable in the same bounded tmpfs/memory cgroup. A focused
                # run compiles, but never executes, preparatory regressions.
                if not test_filter:
                    preparation += "cargo test -p dunlin-relay -p dunlin-identity -p dunlin-session-runtime -p dunlin-terminal -p dunlin-usage -p dunlin-workspace --offline --locked --no-run || { runtime_resources; exit 1; }\n"
                else:
                    preparation += "cargo test -p dunlin-relay --offline --locked --no-run || { runtime_resources; exit 1; }\n"
                preparation += "runtime_resources\n"
                preparation += "cargo build -p dunlin-relay --offline --locked || { runtime_resources; exit 1; }\n"
                preparation += "runtime_resources\n"
                if not test_filter:
                    # Normal regressions may use the executable too; execute
                    # them only after the real production build is available.
                    preparation += "cargo test -p dunlin-relay -p dunlin-identity -p dunlin-session-runtime -p dunlin-terminal -p dunlin-usage -p dunlin-workspace --offline --locked --no-fail-fast || { runtime_resources; exit 1; }\n"
                    preparation += "runtime_resources\n"
                selection = test_filter or " production_browser_acceptance_runtime_"
                check = f"cargo test -p dunlin-relay{selection} --offline --locked -- --ignored --nocapture"
            payload = ["/bin/sh", "-ec", "mkdir -p /scratch/cargo\n"
                       "ln -s /cache/registry /scratch/cargo/registry\n"
                       f"{preparation}check_status=0\n{check} || check_status=$?\n"
                       + ("df -k /scratch\ndu -sk /scratch/target\n" if args.suite in {"server", "runtime"} else "") +
                       "python3 -I -c 'import json; from pathlib import Path; print(\"Sandbox memory: \" + json.dumps({name: (Path(\"/sys/fs/cgroup\") / name).read_text().strip() for name in [\"memory.peak\", \"memory.events\"]}))'\n"
                       'exit "$check_status"']
            # Large server/test ELFs need bounded LLVM partition sizes.
            # Smaller LLVM units aim to limit compiler peaks without raising RAM or
            # build parallelism, changing selection, or disabling assertions.
            rust_codegen_units = 256 if args.suite in {"runtime", "server"} else 64 if args.suite in {"browser", "macos-profile"} else 8
            rust_strip = " -C strip=symbols" if args.suite in {"runtime", "server"} else ""
            variables = [f"PATH={args.rust_home}/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin", f"RUSTUP_HOME={args.rust_home}",
                          "CARGO_HOME=/scratch/cargo", "CARGO_TARGET_DIR=/scratch/target",
                          "CARGO_BUILD_JOBS=1", "CARGO_INCREMENTAL=0", "RUST_TEST_THREADS=1",
                           "CARGO_PROFILE_TEST_DEBUG=0", "CARGO_PROFILE_DEV_DEBUG=0",
                           f"RUSTFLAGS=-C debuginfo=0 -C codegen-units={rust_codegen_units}{rust_strip}"]
            if profile_fixture is not None:
                variables += [f"DUNLIN_MACOS_PROFILE_FIXTURE={profile_fixture}"]
            if args.suite == "browser" or runtime_browser:
                variables += ["DUNLIN_WEB_E2E_ROOT=/scratch/dist", "DUNLIN_CHROMIUM=/usr/bin/chromium"]
            if runtime_browser:
                variables += ["DUNLIN_REMOTE_SESSION_BROWSER=1"]
            if args.suite == "runtime":
                variables += ["DUNLIN_RELAY_E2E_BIN=/scratch/target/debug/dunlin-relay"]
        else:
            paths = ["apps/web/src", "apps/web/tests", "apps/web/package.json", "tests/fixtures/web-catalog"]
            if args.suite in {"typecheck", "build"}:
                paths += ["apps/web/tsconfig.json", "apps/web/vite.config.ts"]
            if args.suite == "build":
                paths += ["apps/web/index.html", "apps/web/postcss.config.mjs", "apps/web/REUI-LICENSE.txt"]
            source, source_digest = source_snapshot(parent, paths)
            command += mount(source, "/repo")
            variables = ["PATH=/usr/local/bin:/usr/bin:/bin"]
            if args.suite == "frontend":
                payload = ["/usr/local/bin/node", "--experimental-strip-types", "--test",
                           *[str(path.relative_to(source / "apps/web"))
                             for path in sorted((source / "apps/web/tests").glob("*.test.ts"))]]
            elif args.suite in {"typecheck", "build"}:
                (source / "apps/web/node_modules").mkdir()
                command += mount(args.node_modules, "/repo/apps/web/node_modules")
                payload = ["/usr/local/bin/node", "node_modules/typescript/bin/tsc", "--noEmit"] if args.suite == "typecheck" else [
                    "/usr/local/bin/node", "node_modules/vite/bin/vite.js", "build", "--configLoader", "native", "--outDir", "/scratch/dist"]
            else:
                payload = ["/usr/local/bin/node", "-e", PROBE]
        command += ["--entrypoint=/usr/bin/env", image_id, "-i", *variables,
                    f"HOME={profile_fixture / 'home' if profile_fixture is not None else '/scratch'}", "TMPDIR=/scratch", "/usr/bin/timeout", "--kill-after=5", str(wall), *payload]
        scope = ""
        if args.suite == "runtime":
            scope = f" scope=runtime-focused-acceptance-only filter={args.rust_test}" if test_filter else " scope=runtime-full-regression filter=all-runtime-acceptance"
        if runtime_browser:
            scope += " chromium=remote-session-production-ui pids=256"
        if rust:
            scope += f" codegen_units={rust_codegen_units}"
            if rust_strip:
                scope += " strip=symbols"
        print(f"Sandbox suite={args.suite} container={name} image={image_id} platform={platform} memory={memory} scratch={scratch} file_size={file_size} wall={wall}s source_sha256={source_digest}{scope}", flush=True)
        process = subprocess.Popen(command, env=environment)
        try:
            return process.wait(timeout=wall + 20)
        finally:
            # Only the random name generated by this invocation is eligible.
            try:
                subprocess.run(cli + ["rm", "--force", name], env=environment,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)
            finally:
                if process.returncode is None:
                    process.kill()
                    process.wait(timeout=5)


PROBE = r"""
const fs = require('node:fs');
const os = require('node:os');
const assert = require('node:assert/strict');
assert.equal(process.getuid(), 65534);
assert.equal(fs.existsSync('/var/run/docker.sock'), false);
assert.equal(fs.existsSync('/repo/.git'), false);
assert.equal(fs.existsSync('/Users'), false);
assert.deepEqual(Object.keys(process.env).sort(), ['HOME', 'PATH', 'TMPDIR']);
assert.deepEqual(Object.keys(os.networkInterfaces()), ['lo']);
assert.equal(fs.readFileSync('/sys/fs/cgroup/memory.max', 'utf8').trim(), '536870912');
assert.equal(fs.readFileSync('/sys/fs/cgroup/pids.max', 'utf8').trim(), '64');
assert.equal(fs.readFileSync('/sys/fs/cgroup/cpu.max', 'utf8').trim(), '100000 100000');
assert.throws(() => fs.writeFileSync('/repo/apps/web/src/.probe-write', 'denied'));
assert.throws(() => fs.writeFileSync('/etc/.probe-write', 'denied'));
fs.writeFileSync('/scratch/probe', 'allowed');
fs.writeFileSync('/tmp/probe', 'allowed');
const tmp = fs.statfsSync('/tmp');
assert.ok(tmp.blocks * tmp.bsize <= 16 * 1024 * 1024);
const stat = fs.statfsSync('/scratch');
assert.ok(stat.blocks * stat.bsize <= 128 * 1024 * 1024);
console.log('Isolation probe passed: no network/home/socket/git, readonly source/root, empty env, hard CPU/RAM/pids/disk limits.');
"""


def main():
    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)

    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["probe", "frontend", "typecheck", "build", "domain", "server", "clippy", "browser", "sessions", "runtime", "usage", "workspace", "mcp", "tooling", "docs", "macos-profile"], required=True)
    parser.add_argument("--runtime-browser", action="store_true", help="Also exercise the production Remote session UI in Chromium; runtime only, requires existing Node/browser dependency caches")
    parser.add_argument("--browser-test", choices=["all", "oidc", "mcp_oauth", "ahp", "ahp_dispatch", "ahp_controls", "ahp_permissions", "ahp_sharing", "ahp_history", "ahp_create_idempotency", "hosts", "remote_bindings", "local_bindings"], default="all", help="Select all browser checks or a focused acceptance regression")
    parser.add_argument("--macos-profile-fixture", type=Path, help="Owned canonical fixture, mounted read-only at the identical absolute path; macos-profile only")
    parser.add_argument("--rust-test", help="Named Rust regression; runtime accepts only production_browser_acceptance_runtime_ names and runs targeted acceptance without full preparatory regressions; macos-profile uses a normal precheck before export")
    parser.add_argument("--docker-host", help="Explicit local Docker socket; global context is never changed")
    parser.add_argument("--node-image", default="node:22-bookworm-slim")
    parser.add_argument("--rust-image", default="dunlin-web-checks:rust-1.99")
    parser.add_argument("--rust-home", default="/usr/local/rustup")
    parser.add_argument("--browser-image", default="dunlin-web-checks:browser")
    parser.add_argument("--runtime-image", default="dunlin-web-checks:browser-bwrap")
    parser.add_argument("--runtime-policy-dir", type=Path, default=ROOT / "target/web-checks/runtime-policies")
    parser.add_argument("--browser-node-modules", type=Path, default=ROOT / "target/web-checks/browser-node-deps/node_modules")
    parser.add_argument("--wall-seconds", type=int, choices=range(1, 901), metavar="1..900")
    parser.add_argument("--registry", type=Path, default=Path.home() / ".cargo/registry")
    parser.add_argument("--node-modules", type=Path, default=ROOT / "web/node_modules")
    parser.add_argument("--tmp-root", type=Path)
    args = parser.parse_args()
    try:
        rust_test_arg(args)
        macos_profile_fixture(args)
    except ValueError as error:
        parser.error(str(error))
    if args.docker_host and not args.docker_host.startswith("unix://"):
        parser.error("Use a local UNIX Docker socket, never an external engine")
    try:
        return run(args)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"Sandbox check could not complete: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
