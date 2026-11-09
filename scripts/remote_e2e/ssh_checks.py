"""Negative cases use the same production OpenSSH entry points as the UI."""
import hashlib
import os
from pathlib import Path
import re
import secrets
import socket

from .core import command, require


def run(scenarios, name):
    run, client, provider = scenarios.run, scenarios.c, scenarios.p
    original = provider.ssh.config.read_text()
    config = run.directory / ("ssh-negative-" + name + ".config")
    control = Path(os.environ.get("TMPDIR", "/tmp")) / ("de-negative-" + secrets.token_hex(6) + ".sock")
    # Never reuse a successful ControlMaster: that would bypass authentication
    # and known_hosts and make the negative test a false positive.
    host = {"id": run.id + "-negative", "name": "Negative fixture", "target": provider.ssh.target,
            "user": "admin", "port": 0, "identity": None, "relay_port": 8787, "local_port": 0,
            "host_key": "", "created_at": 0}
    if name == "host-key":
        # A config-level option overrides included Lima settings, without
        # modifying Lima's own trusted key file or the user's ~/.ssh.
        known = run.directory / "empty-known-hosts"
        known.write_text("")
        text = (f'Host *\n  UserKnownHostsFile "{known}"\n  GlobalKnownHostsFile /dev/null\n'
                '  StrictHostKeyChecking yes\n  ControlMaster no\n' + original)
    elif name == "bad-key":
        key = run.directory / ("wrong-identity-" + secrets.token_hex(4))
        command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", key])
        # Remove all configured identity files; OpenSSH otherwise tries both.
        text = re.sub(r"(?im)^\s*IdentityFile\s+.*$", "", original)
        text = (f'Host *\n  IdentityFile "{key}"\n  IdentitiesOnly yes\n  IdentityAgent none\n'
                '  ControlMaster no\n' + text)
    elif name == "port-conflict":
        config.write_text(original)
        config.chmod(0o600)
        # The master is checked separately; preserve the live conflicting socket
        # throughout the production start_tunnel call.
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            reservation.listen(1)
            host["local_port"] = reservation.getsockname()[1]
            payload = {"host": host, "ssh_config": str(config), "control_socket": str(control)}
            try:
                client.call("tunnel", payload=payload, timeout=20, expect_failure=True)
            finally:
                command(["/usr/bin/ssh", "-F", config, "-S", control, "-O", "exit", provider.ssh.target], check=False)
        return
    else:
        raise ValueError("Unknown SSH negative check")
    config.write_text(text)
    config.chmod(0o600)
    payload = {"host": host, "ssh_config": str(config), "control_socket": str(control)}
    try:
        client.call("ssh-check", payload=payload, expect_failure=True, timeout=30)
    finally:
        command(["/usr/bin/ssh", "-F", config, "-S", control, "-O", "exit", provider.ssh.target], check=False)
