"""Explicit providers. A resource is journaled *before* creating it."""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tarfile
import time
import socket

from .core import ROOT, Blocked, command, private_json, require, sha256, stop, wait_until


def quoted(value):
    return shlex.quote(str(value))


class SSH:
    def __init__(self, config, target):
        self.config = Path(config).resolve(strict=True)
        require(re.fullmatch(r"[a-zA-Z0-9_.-]+", target), "Expected an SSH config alias, not options")
        self.target = target

    def exec(self, script, *, input=None, timeout=120, check=True):
        return command(["/usr/bin/ssh", "-F", self.config, "-o", "BatchMode=yes",
                        "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10",
                        "--", self.target, script], input=input, timeout=timeout, check=check)

    def put(self, local, remote):
        self.exec("umask 077; cat > " + quoted(remote), input=Path(local).read_bytes())

    def text(self, script, **kwargs):
        return self.exec(script, **kwargs).stdout.decode().strip()


class Provider:
    def __init__(self, run, options):
        self.run, self.options = run, options
        self.ssh = None
        self.resource = None

    def prepare(self):
        raise NotImplementedError

    def initialize(self):
        platform = self.ssh.text("uname -s; uname -m").splitlines()
        require(len(platform) == 2 and platform[0] in ("Linux", "Darwin"), "Unsupported guest platform")
        self.platform, self.arch = platform
        self.home = self.ssh.text('printf %s "$HOME"')
        require(self.home.startswith("/") and "\n" not in self.home, "Invalid guest home")
        self.root = self.home + "/" + self.run.id
        self.remote_resource = self.run.own("remote-directory", path=self.root,
                                           ssh_config=str(self.ssh.config), target=self.ssh.target)
        self.ssh.exec(f"set -eu; umask 077; mkdir {quoted(self.root)}; "
                      f"printf %s {quoted(self.run.id)} > {quoted(self.root + '/.owner')}")
        self.run.report["facts"].update(server_os=self.platform, server_arch=self.arch)
        self.run.flush()

    def reboot(self):
        if self.options.provider == "ssh-host" and not self.options.allow_reboot:
            raise Blocked("Reboot of a remote host requires --allow-reboot")
        old = self.ssh.text("sysctl -n kern.boottime" if self.platform == "Darwin"
                            else "cat /proc/sys/kernel/random/boot_id")
        # SSH may close before a successful response is sent.
        self.ssh.exec("sudo -n shutdown -r now", check=False)
        def changed_boot():
            value = self.ssh.text("sysctl -n kern.boottime" if self.platform == "Darwin"
                                  else "cat /proc/sys/kernel/random/boot_id", timeout=15)
            return value != old
        wait_until(changed_boot, timeout=180)

    def collect(self):
        result = self.ssh.exec(f"find {quoted(self.root)} -maxdepth 2 -name '*.log' -type f "
                               "-exec tail -c 65536 {} \\;", check=False)
        self.run.log("server", result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace"))
        if self.platform == "Linux":
            result = self.ssh.exec("journalctl --user --no-pager -n 150", check=False)
            self.run.log("journal", result.stdout.decode(errors="replace"))


class Lima(Provider):
    def prepare(self):
        if not shutil.which("limactl"):
            raise Blocked("Install Lima before selecting a Lima profile")
        name = self.run.id
        existing = command(["limactl", "list", name, "--json"], check=False)
        require(not existing.stdout.strip(), "Refusing to adopt or delete an existing Lima VM")
        self.resource = self.run.own("lima", name=name)
        template = self.options.vm_template or ROOT / "tests/remote-e2e/vm/lima/debian-arm64.yaml"
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.tls_port = reservation.getsockname()[1]
        forwards = [{"guestPort": 9443, "hostPort": self.tls_port, "hostIP": "127.0.0.1"},
                    {"guestPortRange": [1, 65535], "ignore": True}]
        result = command(["limactl", "start", "--name", name, "--tty=false", "--set",
                          ".portForwards = " + json.dumps(forwards), template], timeout=900)
        self.run.log("vm-start", (result.stdout + result.stderr).decode(errors="replace"))
        listing = command(["limactl", "list", name, "--json"]).stdout.decode()
        info = json.loads(listing)
        require(not info["config"].get("mounts"), "VM must not mount host directories")
        require(not info["config"]["ssh"].get("forwardAgent"), "VM must not inherit SSH agent")
        self.ssh = pin_lima_ssh(self.run, info)
        self.run.report["facts"]["vm_config_sha256"] = sha256(template)
        self.initialize()


def pin_lima_ssh(run, info):
    """Lima's generated config disables host checking: NEVER use it for the SUT.

    Obtain the guest public key through the owned VM's management channel, and
    authorize a fresh run-only client key. No personal SSH key or agent is used.
    """
    name = run.id
    require(info["name"] == name, "Foreign VM management identity")
    host_key = command(["limactl", "shell", name, "cat", "/etc/ssh/ssh_host_ed25519_key.pub"]).stdout.decode().strip()
    require(host_key.startswith("ssh-ed25519 "), "Unexpected VM host key")
    identity = run.directory / "identity"
    if not identity.exists():
        command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", identity])
        command(["limactl", "shell", name, "sh", "-c",
                 "umask 077; mkdir -p ~/.ssh; cat >> ~/.ssh/authorized_keys"], input=identity.with_suffix(".pub").read_bytes())
    known = run.directory / "known_hosts"
    known.write_text(f"[127.0.0.1]:{info['sshLocalPort']} {host_key}\n")
    settings = {"dedicated_test_host": True, "hostname": "127.0.0.1", "port": info["sshLocalPort"],
                "user": info["config"]["user"]["name"], "identity": str(identity), "known_hosts": str(known)}
    return configured_ssh(run, settings)


class Tart(Provider):
    def prepare(self):
        if not shutil.which("tart"):
            raise Blocked("Tart is required for a macOS VM; it is never installed implicitly")
        config = self.options.host_config
        if not config:
            raise Blocked("Tart needs --host-config with a pinned image, SSH key and known_hosts")
        settings = json.loads(Path(config).read_text())
        image = settings.get("image", "")
        require(re.fullmatch(r".+@sha256:[a-f0-9]{64}", image), "Pin the Tart image by OCI SHA-256 digest")
        listing = command(["tart", "list"]).stdout.decode()
        require(self.run.id not in listing.split(), "Refusing to adopt or delete an existing Tart VM")
        self.resource = self.run.own("tart", name=self.run.id)
        command(["tart", "clone", image, self.run.id], timeout=900)
        log = (self.run.directory / "tart.log").open("wb")
        process = subprocess.Popen(["tart", "run", "--no-graphics", self.run.id],
                                   stdout=log, stderr=log, start_new_session=True)
        log.close()
        self.run.processes.append(process)
        address = wait_until(lambda: command(["tart", "ip", self.run.id], check=False).stdout.decode().strip(), 180)
        self.ssh = configured_ssh(self.run, settings, address)
        wait_until(lambda: self.ssh.text("true", timeout=15) == "", 90)
        self.initialize()


def configured_ssh(run, settings, address=None):
    require(settings.get("dedicated_test_host") is True, "A dedicated test host must be explicitly declared")
    host = address or settings["hostname"]
    require(re.fullmatch(r"[a-zA-Z0-9.:-]+", host), "Invalid hostname")
    user = settings["user"]
    require(re.fullmatch(r"[a-zA-Z0-9_.-]+", user), "Invalid SSH user")
    identity = Path(settings["identity"]).expanduser().resolve(strict=True)
    known = Path(settings["known_hosts"]).expanduser().resolve(strict=True)
    require(not any(c in str(identity) + str(known) for c in ('\n', '\r', '"')), "Invalid SSH path")
    port = int(settings.get("port", 22))
    require(1 <= port <= 65535, "Invalid SSH port")
    # No Include, ProxyCommand, agent, or user-global config. Paths use SSH quoting.
    text = (f'Host fixture\n  HostName {host}\n  User {user}\n  Port {port}\n'
            f'  IdentityFile "{identity}"\n  UserKnownHostsFile "{known}"\n'
            '  GlobalKnownHostsFile /dev/null\n  StrictHostKeyChecking yes\n'
            '  NoHostAuthenticationForLocalhost no\n'
            '  IdentitiesOnly yes\n  IdentityAgent none\n  ForwardAgent no\n')
    config = run.directory / "ssh.config"
    config.write_text(text)
    config.chmod(0o600)
    return SSH(config, "fixture")


class RemoteHost(Provider):
    def prepare(self):
        if not self.options.host_config:
            raise Blocked("Remote host requires --host-config; no host is inferred")
        settings = json.loads(Path(self.options.host_config).read_text())
        self.ssh = configured_ssh(self.run, settings)
        self.initialize()


def source_archive(run):
    archive = run.directory / "source.tar.gz"
    files = command(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT).stdout
    with tarfile.open(archive, "w:gz") as tar:
        for name in sorted(set(files.decode().split("\0"))):
            path = ROOT / name
            if (not name or not path.is_file() or path.is_symlink()
                    or Path(name).parts[0] not in {"Cargo.toml", "Cargo.lock", "crates", "apps", "vendor", "assets", "scripts"}):
                continue
            tar.add(path, arcname=name, recursive=False)
    run.report["facts"]["source_archive_sha256"] = sha256(archive)
    return archive


def cleanup(directory):
    from .core import validate_run, RUN_PREFIX
    manifest = validate_run(directory)
    errors = []
    completed = set()
    for resource in reversed(manifest["resources"]):
        key = (resource["kind"], resource.get("name") or resource.get("path") or resource.get("socket") or resource.get("target"))
        if key in completed:
            resource["cleaned"] = True
            continue
        if resource["cleaned"]:
            completed.add(key)
            continue
        try:
            kind = resource["kind"]
            if kind in ("lima", "tart"):
                require(resource["name"] == manifest["run_id"], "Refuse deletion of a foreign VM")
                if kind == "lima":
                    listing = command(["limactl", "list", resource["name"], "--json"], check=False)
                    if listing.stdout.strip():
                        command(["limactl", "stop", "--force", resource["name"]], timeout=90)
                        command(["limactl", "delete", "--force", resource["name"]], timeout=90)
                else:
                    command(["tart", "stop", resource["name"]], check=False, timeout=60)
                    command(["tart", "delete", resource["name"]], timeout=60)
            elif kind == "remote-directory":
                path = resource["path"]
                require(Path(path).name == manifest["run_id"] and Path(path).name.startswith(RUN_PREFIX),
                        "Foreign remote directory")
                ssh = SSH(resource["ssh_config"], resource["target"])
                # Exact ownership marker and exact path; no parent-directory deletes.
                script = (f"set -eu; if [ -d {quoted(path)} ]; then "
                          f"test \"$(cat {quoted(path + '/.owner')})\" = {quoted(manifest['run_id'])}; "
                          f"rm -rf -- {quoted(path)}; fi")
                # A VM may already have been manually removed. Only tolerate that if
                # the VM itself is absent, never for a real host.
                ssh.exec(script, timeout=60)
            elif kind == "user-service":
                ssh = SSH(resource["ssh_config"], resource["target"])
                name = resource["name"]
                require(name in (manifest["run_id"], manifest["run_id"] + "-tls", manifest["run_id"] + "-upgrade", manifest["run_id"] + "-installer"), "Foreign service")
                if resource["platform"] == "Linux":
                    ssh.exec(f"set -eu; STATE=$(systemctl --user show {quoted(name)}.service -p LoadState --value); "
                             f"if [ \"$STATE\" != not-found ]; then systemctl --user disable --now {quoted(name)}.service; fi; "
                             f"rm -f -- \"$HOME/.config/systemd/user/{name}.service\"; "
                             "systemctl --user daemon-reload")
                else:
                    ssh.exec(f"set -eu; if launchctl print gui/$(id -u)/{quoted(name)} >/dev/null 2>&1; then "
                             f"launchctl bootout gui/$(id -u)/{quoted(name)}; fi; "
                             f"rm -f -- \"$HOME/Library/LaunchAgents/{name}.plist\"")
            elif kind == "ssh-master":
                import hashlib
                digest = hashlib.sha256(manifest["run_id"].encode()).hexdigest()[:16]
                require(Path(resource["socket"]).name == f"de-{digest}.sock", "Foreign SSH socket")
                command(["/usr/bin/ssh", "-F", resource["config"], "-S", resource["socket"],
                         "-O", "exit", resource["target"]], check=False, timeout=15)
            elif kind == "linger":
                SSH(resource["ssh_config"], resource["target"]).exec('sudo -n loginctl disable-linger "$(id -un)"')
            else:
                raise RuntimeError(f"Unknown owned resource kind: {kind}")
            resource["cleaned"] = True
            completed.add(key)
        except Exception as error:
            errors.append(f"{resource['kind']}: {type(error).__name__}: {error}")
        private_json(Path(directory) / "ownership.json", manifest)
    manifest["cleanup_complete"] = not errors
    private_json(Path(directory) / "ownership.json", manifest)
    if errors:
        raise RuntimeError("; ".join(errors))
