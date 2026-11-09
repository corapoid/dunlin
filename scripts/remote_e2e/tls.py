"""Per-run PKI. Never installs a root into the user's trust stores."""
import json
from pathlib import Path
import secrets

from .core import ROOT, command, require, wait_until
from .providers import quoted


class TLS:
    def __init__(self, scenarios):
        self.s, self.p, self.c = scenarios, scenarios.p, scenarios.c
        self.directory = scenarios.run.directory / "pki"
        self.directory.mkdir(mode=0o700, exist_ok=True)
        self.name = self.p.run.id + "-tls"
        self.original_url, self.original_ca = self.c.url, self.c.ca

    def openssl(self, *args):
        command(["openssl", *args], cwd=self.directory)

    def certificates(self):
        self.openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca.key",
                     "-out", "ca.pem", "-subj", "/CN=Dunlin disposable acceptance CA", "-days", "2",
                     "-addext", "basicConstraints=critical,CA:TRUE")
        self.openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", "server.key",
                     "-out", "server.csr", "-subj", "/CN=localhost")
        for name, san, days in [("valid", "IP:127.0.0.1,DNS:localhost", "1"),
                                 ("wrong-name", "DNS:not-the-relay.invalid", "1")]:
            (self.directory / "leaf.ext").write_text(
                "basicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\n"
                "extendedKeyUsage=serverAuth\nsubjectAltName=" + san + "\n")
            self.openssl("x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key",
                         "-set_serial", str(int.from_bytes(secrets.token_bytes(8), "big")),
                         "-days", days, "-extfile", "leaf.ext", "-out", name + ".pem")
        # OpenSSL 3 rejects non-positive -days. Sign with explicit historical
        # dates instead; do not change the machine clock or sleep for expiry.
        (self.directory / "index.txt").write_text("")
        (self.directory / "serial").write_text("1000\n")
        (self.directory / "ca.cnf").write_text(
            "[ca]\ndefault_ca=fixture\n[fixture]\ndatabase=index.txt\nnew_certs_dir=.\n"
            "certificate=ca.pem\nprivate_key=ca.key\nserial=serial\ndefault_md=sha256\n"
            "policy=policy\ndefault_days=1\nx509_extensions=leaf\n"
            "[policy]\ncommonName=supplied\n[leaf]\nbasicConstraints=critical,CA:FALSE\n"
            "keyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n"
            "subjectAltName=IP:127.0.0.1,DNS:localhost\n")
        self.openssl("ca", "-batch", "-notext", "-config", "ca.cnf", "-in", "server.csr",
                     "-startdate", "20000101000000Z", "-enddate", "20010101000000Z", "-out", "expired.pem")
        for path in self.directory.iterdir():
            path.chmod(0o600)
        root = self.p.root
        for name in ["server.key", "valid.pem", "wrong-name.pem", "expired.pem"]:
            self.p.ssh.put(self.directory / name, root + "/" + name)
        self.p.ssh.put(ROOT / "scripts/remote_e2e/proxy.py", root + "/tls_proxy.py")
        self.p.run.own("user-service", name=self.name, platform=self.p.platform,
                       ssh_config=str(self.p.ssh.config), target=self.p.ssh.target)
        self.activate("valid", initial=True)
        if self.p.options.provider == "lima":
            self.url = f"https://127.0.0.1:{self.p.tls_port}"
            self.p.run.report["facts"]["tls_transport"] = "lima-tls-only-port-forward-no-ssh-tunnel"
        else:
            tunnel = self.c.tunnel(self.p, port=9443)
            self.url = tunnel["url"].replace("http://", "https://")
            self.p.run.report["facts"]["tls_transport"] = "verified-tls-over-ssh-tcp-forward"
        self.c.url, self.c.ca = self.url, (self.directory / "ca.pem").read_text()
        wait_until(lambda: self.c.request("/health") == "ok", timeout=30)

    def activate(self, variant, initial=False, reject_websocket=False):
        root = self.p.root
        launch = (f"#!/bin/sh\nexec python3 {quoted(root + '/tls_proxy.py')} "
                  f"--cert {quoted(root + '/' + variant + '.pem')} --key {quoted(root + '/server.key')} "
                  f"{'--reject-websocket ' if reject_websocket else ''}"
                  f">>{quoted(root + '/tls.log')} 2>&1\n")
        self.p.ssh.exec(f"cat > {quoted(root + '/tls.sh')}; chmod 700 {quoted(root + '/tls.sh')}", input=launch.encode())
        if self.p.platform == "Linux":
            if initial:
                unit = f"[Service]\nExecStart={root}/tls.sh\nRestart=on-failure\n"
                self.p.ssh.exec(f"cat > ~/.config/systemd/user/{self.name}.service", input=unit.encode())
                self.p.ssh.exec("systemctl --user daemon-reload")
            self.p.ssh.exec(f"systemctl --user reset-failed {quoted(self.name)}.service; systemctl --user restart {quoted(self.name)}.service")
        else:
            import plistlib
            plist = plistlib.dumps({"Label": self.name, "ProgramArguments": [root + "/tls.sh"], "RunAtLoad": True})
            self.p.ssh.exec(f"cat > ~/Library/LaunchAgents/{self.name}.plist", input=plist)
            if not initial:
                self.p.ssh.exec(f"launchctl bootout gui/$(id -u)/{quoted(self.name)}", check=False)
            self.p.ssh.exec(f"launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/{self.name}.plist")
        # Test socket readiness without accepting any invalid certificate.
        script = "python3 -c \"import socket;s=socket.create_connection(('127.0.0.1',9443),2);s.close()\""
        wait_until(lambda: self.p.ssh.exec(script, check=False).returncode == 0, timeout=15)

    def positive(self):
        self.c.url, self.c.ca = self.url, (self.directory / "ca.pem").read_text()
        self.c.request("/v1/me")
        self.c.call("read", path="file.txt")
        self.c.acp(expected="DUNLIN_WSS_OK")
        self.c.call("terminal")

    def negative(self, variant):
        self.c.url, self.c.ca = self.url, (self.directory / "ca.pem").read_text()
        previous = self.c.ca
        try:
            if variant == "untrusted":
                self.c.ca = None
            else:
                self.activate(variant)
            self.c.call("read", path="file.txt", expect_failure=True)
            self.c.call("acp", payload={"prompt": "Hello", "expected": "Hello", "timeout_secs": 20}, expect_failure=True)
            self.c.call("terminal", expect_failure=True)
        finally:
            self.c.ca = previous
            if variant != "untrusted":
                self.activate("valid")
        self.c.request("/v1/me")

    def close(self):
        # Reboot may have opened a new production tunnel. Never restore its
        # dead pre-reboot predecessor when TLS scope was already closed.
        if self.c.url == getattr(self, "url", None):
            self.c.url, self.c.ca = self.original_url, self.original_ca

    def rejected_upgrade(self):
        self.c.url, self.c.ca = self.url, (self.directory / "ca.pem").read_text()
        try:
            self.activate("valid", reject_websocket=True)
            self.c.request("/v1/me")
            self.c.call("acp", payload={"prompt": "Hello", "expected": "Hello", "timeout_secs": 20}, expect_failure=True)
            self.c.call("terminal", expect_failure=True)
        finally:
            self.activate("valid")
