"""Build/source and published-artifact installation, scoped user services."""
import json
from pathlib import Path
import re
import shlex
import tarfile

from .core import ROOT, Blocked, command, require, sha256
from .providers import quoted, source_archive


def deploy(provider, model):
    run, ssh, root = provider.run, provider.ssh, provider.root
    ssh.exec(f"mkdir -p {quoted(root + '/bin')} {quoted(root + '/host-home')} "
             f"{quoted(root + '/project')} {quoted(root + '/server/tools/bin')}")
    if provider.options.relay_artifact:
        artifact = Path(provider.options.relay_artifact).resolve(strict=True)
        expected = provider.options.relay_sha256
        require(expected and sha256(artifact) == expected, "Relay artifact checksum missing or mismatched")
        if tarfile.is_tarfile(artifact):
            with tarfile.open(artifact) as archive:
                members = archive.getmembers()
                require(len(members) == 1 and members[0].name == "dunlin-relay" and members[0].isfile(),
                        "Expected a single regular dunlin-relay archive member")
                content = archive.extractfile(members[0]).read()
            ssh.exec(f"cat > {quoted(root + '/bin/dunlin-relay')}", input=content)
        else:
            ssh.put(artifact, root + "/bin/dunlin-relay")
        run.report["facts"]["relay_artifact_sha256"] = sha256(artifact)
    else:
        archive = source_archive(run)
        ssh.put(archive, root + "/source.tar.gz")
        # The archive contains only regular files from this checkout; no host mounts.
        ssh.exec(f"mkdir {quoted(root + '/source')}; tar -xzf {quoted(root + '/source.tar.gz')} "
                 f"-C {quoted(root + '/source')}")
        script = f"""set -eu
cd {quoted(root + '/source')}
export RUSTUP_HOME={quoted(root + '/rustup')} CARGO_HOME={quoted(root + '/cargo')}
curl --fail --silent --show-error --proto '=https' --tlsv1.2 https://sh.rustup.rs -o {quoted(root + '/rustup-init.sh')}
sh {quoted(root + '/rustup-init.sh')} -y --no-modify-path --profile minimal --default-toolchain {quoted(provider.options.rust_version)}
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
cargo build -p dunlin-relay --locked
cargo build -p dunlin-vault --features manual-acceptance --bin vault_acceptance --locked
cp target/debug/dunlin-relay {quoted(root + '/bin/dunlin-relay')}
cp target/debug/vault_acceptance {quoted(root + '/bin/vault_acceptance')}
"""
        result = ssh.exec(script, timeout=3600)
        run.log("guest-build", (result.stdout + result.stderr).decode(errors="replace"))
    ssh.exec(f"chmod 700 {quoted(root + '/bin/dunlin-relay')}; {quoted(root + '/bin/dunlin-relay')} --help")
    # Install a pinned real OpenCode, not a fake ACP executable. npm verifies its
    # registry integrity. Record the exact version and executable checksum below.
    version = provider.options.opencode_version
    require(re.fullmatch(r"\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?", version), "Pin an exact OpenCode version")
    tools = root + "/server/tools"
    ssh.exec(f"touch {quoted(root + '/npm-user.conf')} {quoted(root + '/npm-global.conf')}")
    result = ssh.exec(f"npm --userconfig={quoted(root + '/npm-user.conf')} --globalconfig={quoted(root + '/npm-global.conf')} install --no-audit --no-fund --prefix {quoted(tools)} @opencode/cli@{version}", timeout=600)
    run.log("opencode-install", (result.stdout + result.stderr).decode(errors="replace"))
    wrapper = f"#!/bin/sh\nexec {quoted(tools + '/node_modules/.bin/opencode')} \"$@\"\n"
    guard = ("import os,json,pathlib\n"
             f"allowed=pathlib.Path({(root + '/project/file.txt')!r})\n"
             "assert allowed.read_text()\n"
             f"secret=pathlib.Path({(root + '/server/host-secret')!r})\n"
             "try:\n secret.read_text()\nexcept (PermissionError,FileNotFoundError):\n denied=True\n"
             "else:\n raise RuntimeError('Agent sandbox exposed relay host secret')\n"
             "assert not os.environ.get('SSH_AUTH_SOCK')\n"
             "assert not os.environ.get('DBUS_SESSION_BUS_ADDRESS')\n"
             "assert not any(os.environ.get(k) for k in ('OPENCODE_API_KEY','OPENAI_API_KEY','ANTHROPIC_API_KEY','GITHUB_TOKEN'))\n"
             "pathlib.Path('.sandbox-evidence.json').write_text(json.dumps({'allowed_read':True,'host_secret_denied':denied,'credentials_absent':True}))\n")
    ssh.exec(f"cat > {quoted(tools + '/sandbox_guard.py')}", input=guard.encode())
    # This is a real harness wrapper, not an ACP mock. Only run the sandbox
    # assertions on ACP startup, not during version/catalog inspection.
    wrapper = (f"#!/bin/sh\nif [ \"${{1:-}}\" = acp ]; then python3 {quoted(tools + '/sandbox_guard.py')} || exit 79; fi\n"
               f"exec {quoted(tools + '/node_modules/.bin/opencode')} \"$@\"\n")
    ssh.exec(f"cat > {quoted(tools + '/bin/opencode')}; chmod 700 {quoted(tools + '/bin/opencode')}", input=wrapper.encode())
    provider.binary = root + "/bin/dunlin-relay"
    provider.db = root + "/relay.db"
    provider.relay = lambda *args: ssh.text(" ".join(quoted(a) for a in
                                       [provider.binary, *args, "--db", provider.db]))
    # Pairing codes are deliberately not written to logs.
    provider.relay("init")
    provider.relay("setup", "--name", "remote-e2e")
    provider.relay("project", "add", "fixture", root + "/project", "--user", "admin")
    provider.relay("user", "add", "member")
    provider.relay("user", "add", "outsider")
    provider.relay("grant", "fixture", "member")
    from .core import opencode_config
    config = json.dumps(opencode_config(model))
    ssh.exec(f"cat > {quoted(root + '/project/opencode.json')}", input=config.encode())
    ssh.exec(f"printf 'original\\n' > {quoted(root + '/project/file.txt')}; "
             f"printf 'server-secret-canary\\n' > {quoted(root + '/server/host-secret')}; "
             f"printf 'outside-canary\\n' > {quoted(root + '/outside-canary')}; "
             f"cd {quoted(root + '/project')}; git init -qb main; "
             "git config user.name RemoteE2E; git config user.email e2e@example.invalid; "
             "git config commit.gpgsign false; git config core.hooksPath /dev/null; "
             "git add .; git commit -qm fixture")
    # HOME points to a fresh directory. env -i also removes provider keys,
    # OPENCODE_CONFIG*, SSH_AUTH_SOCK, proxies and all host-specific variables.
    path = tools + "/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    server_config = (f'name = "remote-e2e"\nlisten = "127.0.0.1:8787"\n'
                     f'db = {json.dumps(provider.db)}\nharness = "opencode acp"\n'
                     'sandbox = true\nlinger_secs = 30\n')
    ssh.exec(f"cat > {quoted(root + '/relay.toml')}", input=server_config.encode())
    launch = (f"#!/bin/sh\nexec env -i HOME={quoted(root + '/host-home')} "
              f"PATH={quoted(path)} LANG=C.UTF-8 "
              f"{quoted(provider.binary)} serve --config {quoted(root + '/relay.toml')} "
              f">>{quoted(root + '/relay.log')} 2>&1\n")
    ssh.exec(f"cat > {quoted(root + '/serve.sh')}; chmod 700 {quoted(root + '/serve.sh')}", input=launch.encode())
    run.report["facts"]["opencode_version"] = ssh.text(f"{quoted(tools + '/bin/opencode')} --version")
    run.report["facts"]["relay_binary_sha256"] = ssh.text(
        f"{'shasum -a 256' if provider.platform == 'Darwin' else 'sha256sum'} {quoted(provider.binary)}").split()[0]
    start_service(provider)
    run.flush()


def start_service(provider):
    run, ssh, root = provider.run, provider.ssh, provider.root
    name = getattr(provider, "service_name", run.id)
    already_owned = any(r["kind"] == "user-service" and r.get("name") == name for r in run.ownership["resources"])
    if not already_owned:
        suffix = f".config/systemd/user/{name}.service" if provider.platform == "Linux" else f"Library/LaunchAgents/{name}.plist"
        ssh.exec("test ! -e " + quoted(provider.home + "/" + suffix))
    run.own("user-service", name=name, platform=provider.platform,
            ssh_config=str(ssh.config), target=ssh.target)
    if provider.platform == "Linux":
        unit = ("[Unit]\nDescription=Dunlin manual remote E2E\n[Service]\n"
                f"ExecStart={root}/serve.sh\nRestart=on-failure\nRestartSec=1\n"
                "[Install]\nWantedBy=default.target\n")
        ssh.exec(f"mkdir -p ~/.config/systemd/user; cat > ~/.config/systemd/user/{name}.service", input=unit.encode())
        ssh.exec(f"systemctl --user daemon-reload; systemctl --user enable {quoted(name)}.service; systemctl --user restart {quoted(name)}.service")
    else:
        import plistlib
        plist = plistlib.dumps({"Label": name, "ProgramArguments": [root + "/serve.sh"],
                               "RunAtLoad": True, "KeepAlive": True})
        ssh.exec(f"mkdir -p ~/Library/LaunchAgents; cat > ~/Library/LaunchAgents/{name}.plist", input=plist)
        ssh.exec(f"launchctl bootout gui/$(id -u)/{quoted(name)} >/dev/null 2>&1 || true; "
                 f"launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/{name}.plist")


def restart_service(provider, crash=False):
    if provider.platform == "Linux":
        action = "kill --signal=SIGKILL" if crash else "restart"
        provider.ssh.exec(f"systemctl --user {action} {quoted(getattr(provider, 'service_name', provider.run.id))}.service")
    else:
        provider.ssh.exec(f"launchctl kickstart -k gui/$(id -u)/{quoted(getattr(provider, 'service_name', provider.run.id))}")


def published_installer(provider, candidate=True):
    """Exercise the real interactive installer in a PTY, only on disposable VMs."""
    version = provider.options.release_version
    if not version and not candidate:
        raise Blocked("Published installer check requires --release-version (never latest)")
    if provider.options.provider == "ssh-host":
        raise Blocked("The public installer changes host packages; use a disposable VM")
    environment = ""
    if candidate:
        environment, version = candidate_release(provider)
    require(re.fullmatch(r"v?\d+\.\d+\.\d+(?:[-+][a-zA-Z0-9.-]+)?", version), "Invalid release version")
    provider.ssh.put(ROOT / "scripts/install-relay.sh", provider.root + "/install-relay.sh")
    # Run a real PTY inside the guest. The driver has a strict ordered dialogue,
    # strips pairing output from diagnostics, and never supplies account logins.
    driver = (ROOT / "scripts/remote_e2e/installer_pty.py").read_bytes()
    provider.ssh.exec(f"cat > {quoted(provider.root + '/installer_pty.py')}", input=driver)
    result = provider.ssh.exec(
        f"{environment} DUNLIN_VERSION={quoted(version)} python3 {quoted(provider.root + '/installer_pty.py')} "
        f"{quoted(provider.root + '/install-relay.sh')}", timeout=1200)
    provider.run.log("public-installer", result.stdout.decode(errors="replace"))
    normal = provider.home + ("/Library/Application Support/Dunlin/relay.db" if provider.platform == "Darwin"
                              else "/.local/share/dunlin/relay.db")
    installed = provider.home + "/.local/bin/dunlin-relay"
    before = provider.ssh.text(f"{quoted(installed)} user list --db {quoted(normal)}")
    # Rerun the actual PTY dialogue, not a simulated installation function.
    provider.ssh.exec(f"{environment} DUNLIN_VERSION={quoted(version)} python3 {quoted(provider.root + '/installer_pty.py')} "
                      f"{quoted(provider.root + '/install-relay.sh')}", timeout=1200)
    after = provider.ssh.text(f"{quoted(installed)} user list --db {quoted(normal)}")
    require(before == after and "admin" in after, "Installer rerun replaced relay account identities")
    # Installation twice must preserve the first database and identities.
    provider.run.report["facts"]["installer_version"] = version
    provider.run.report["facts"]["installer_source"] = "candidate HTTPS mirror" if candidate else "published release"


def candidate_release(provider):
    """Test candidate installer/recipe/archive together, with real verified HTTPS."""
    import copy
    import hashlib
    import io
    import secrets
    import tomllib
    if "relay_artifact_sha256" in provider.run.report["facts"]:
        raise Blocked("Candidate packaging requires a source-built relay; select --published-installer for release artifacts")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    directory = provider.run.directory / ("installer-mirror-" + secrets.token_hex(4))
    directory.mkdir(mode=0o700)
    command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
             "-subj", "/CN=Dunlin installer fixture", "-addext", "subjectAltName=IP:127.0.0.1",
             "-keyout", directory / "mirror.key", "-out", directory / "mirror.pem"])
    osname = "macos" if provider.platform == "Darwin" else "linux"
    arch = "arm64" if provider.arch in ("arm64", "aarch64") else "x86_64"
    asset = f"dunlin-relay-{version}-{osname}-{arch}.tar.gz"
    binary = provider.ssh.exec("cat " + quoted(provider.binary)).stdout
    with tarfile.open(directory / asset, "w:gz") as archive:
        info = tarfile.TarInfo("dunlin-relay")
        info.mode, info.size = 0o700, len(binary)
        archive.addfile(info, io.BytesIO(binary))
    (directory / "install-harness.sh").write_bytes((ROOT / "scripts/install-harness.sh").read_bytes())
    (directory / "RELAY-SHA256SUMS").write_text("".join(
        f"{sha256(directory / name)}  {name}\n" for name in (asset, "install-harness.sh")))
    remote = provider.root + "/" + directory.name
    provider.ssh.exec("umask 077; mkdir " + quoted(remote))
    for path in directory.iterdir():
        path.chmod(0o600)
        provider.ssh.put(path, remote + "/" + path.name)
    server = ("import http.server,ssl\n"
              f"server=http.server.ThreadingHTTPServer(('127.0.0.1',9444),http.server.SimpleHTTPRequestHandler)\n"
              f"ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);ctx.load_cert_chain({(remote + '/mirror.pem')!r},{(remote + '/mirror.key')!r})\n"
              "server.socket=ctx.wrap_socket(server.socket,server_side=True);server.serve_forever()\n")
    provider.ssh.exec(f"cat > {quoted(remote + '/server.py')}", input=server.encode())
    launch = f"#!/bin/sh\ncd {quoted(remote)}\nexec python3 server.py >mirror.log 2>&1\n"
    provider.ssh.exec(f"cat > {quoted(remote + '/serve.sh')}; chmod 700 {quoted(remote + '/serve.sh')}", input=launch.encode())
    mirror = copy.copy(provider)
    mirror.root, mirror.service_name = remote, provider.run.id + "-installer"
    start_service(mirror)
    from .core import wait_until
    wait_until(lambda: provider.ssh.exec(f"curl --fail --silent --cacert {quoted(remote + '/mirror.pem')} https://127.0.0.1:9444/RELAY-SHA256SUMS", check=False).returncode == 0)
    return f"DUNLIN_RELEASE_BASE_URL=https://127.0.0.1:9444 CURL_CA_BUNDLE={quoted(remote + '/mirror.pem')}", version
