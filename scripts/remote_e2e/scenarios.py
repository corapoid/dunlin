"""Manual deployed-host scenarios. Each function contains behavioral assertions."""
import base64
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import select
import socket
import subprocess
import time
from urllib.parse import quote, urlparse

from .core import ROOT, Blocked, command, private_json, require, stop, wait_until
from .deployment import restart_service, published_installer
from .providers import quoted
from .proxy import Proxy


class Scenarios:
    def __init__(self, provider, client):
        self.p, self.c, self.run = provider, client, provider.run
        self.expiring = None
        self.session = None
        self.direct_url = None

    def code(self, user="admin", ttl="10"):
        text = self.p.relay("pair", user, "--ttl", ttl)
        match = re.search(r"Pairing code for .*?: ([A-Z0-9]+)", text)
        require(match, "Relay CLI did not issue a pairing code")
        code = match.group(1)
        self.run.secrets.append(code)
        return code

    def ready(self):
        wait_until(lambda: self.c.request("/health") == "ok", timeout=30)

    def initialize(self):
        endpoint = urlparse(self.c.url)
        if endpoint.scheme != "https" or not (endpoint.hostname or "").endswith(".ts.net"):
            raise Blocked("Supply --endpoint with this disposable relay's private Tailscale Serve HTTPS address. Product SSH tunnels/bootstrap were removed; provider SSH is for test provisioning only.")
        self.ready()
        self.expiring = (self.code(ttl="1"), time.monotonic())
        pairing = self.c.pair(self.code())
        self.c.token = pairing["token"]
        projects = self.c.call("projects")
        project = next(p for p in projects if p["name"] == "fixture")
        self.c.project_id, self.c.cwd = project["id"], project["path"]
        # Pin the UI's initial model as well as the headless ACP requests.
        self.c.request("/v1/project/settings", "PUT", {
            "project": self.c.project_id,
            "settings": {"harness": "OpenCode", "default_models": {"OpenCode": self.c.model}}})
        self.run.report["facts"]["transport"] = "private-tailscale-https"

    def pairing(self):
        code = self.code()
        paired = self.c.pair(code)
        require(paired["role"] == "admin", "Unexpected paired role")
        self.c.call("pair", code=code, expect_failure=True)
        self.c.call("pair", code="INVALID-PAIRING-CODE", expect_failure=True)
        self.p.relay("revoke", paired["device_id"])
        original = self.c.token
        try:
            self.c.token = paired["token"]
            self.c.request("/v1/me", expected=401)
        finally:
            self.c.token = original

    def expiry(self):
        code, issued = self.expiring
        remaining = 62 - (time.monotonic() - issued)
        if remaining > 0:
            time.sleep(remaining)
        self.c.call("pair", code=code, expect_failure=True)
        # Expiry of the *code* must not expire the already issued device token.
        self.c.request("/v1/me")

    def grants(self):
        original = self.c.token
        try:
            member = self.c.pair(self.code("member"))
            self.c.token = member["token"]
            require(any(p["id"] == self.c.project_id for p in self.c.call("projects")), "Granted project missing")
            self.c.request("/v1/server", "PUT", {"name": "must-not-change"}, expected=403)
            outsider = self.c.pair(self.code("outsider"))
            self.c.token = outsider["token"]
            require(self.c.call("projects") == [], "Ungranted user sees projects")
            self.c.request(f"/v1/sessions?project={self.c.project_id}", expected=403)
            self.c.call("read", path="file.txt", expect_failure=True)
            self.c.call("acp", payload={"prompt": "Hello", "expected": "Hello", "timeout_secs": 20}, expect_failure=True)
        finally:
            self.c.token = original

    def files(self):
        before = self.c.call("read", path="file.txt")
        require(before["contents"], "Fixture file is empty")
        self.c.call("write", path="file.txt", contents="remote-client-written\n")
        require(self.c.call("read", path="file.txt")["contents"] == "remote-client-written\n", "Read-after-write mismatch")
        require(self.p.ssh.text(f"cat {quoted(self.c.cwd + '/file.txt')}") == "remote-client-written", "Write was not remote")
        # A stale precondition must be rejected, with the newer contents retained.
        self.c.call("write", path="file.txt", contents="must-not-win", payload={"expected_mtime_ms": 1}, expect_failure=True)
        require(self.c.call("read", path="file.txt")["contents"] == "remote-client-written\n", "Stale write overwrote contents")
        self.c.call("read", path="../outside-canary", expect_failure=True)
        # Also exercise server rejection, not only the client's traversal guard.
        self.c.request(f"/v1/fs/file?project={self.c.project_id}&path=..%2Foutside-canary", expected=400)
        self.c.call("git")
        patch = self.c.call("git-diff", path="file.txt")["patch"]
        require("remote-client-written" in patch, "Git diff did not use the remote worktree")

    def transfers(self):
        contents = b"portable remote fixture\n"
        name = "copy-" + secrets.token_hex(5)
        manifest = {"name": name, "files": [{"path": "copied.txt", "size": len(contents),
                    "sha256": hashlib.sha256(contents).hexdigest(), "executable": False}],
                    "history": False, "branch": None}
        base = "/v1/server/copies"
        transfer = self.c.request(base, "POST", manifest)["id"]
        self.c.request(f"{base}/{transfer}/chunk", "POST", {"path": "copied.txt", "offset": 1,
                       "contents": base64.b64encode(contents).decode()}, expected=409)
        self.c.request(f"{base}/{transfer}/chunk", "POST", {"path": "copied.txt", "offset": 0,
                       "contents": base64.b64encode(contents).decode()}, expected=204)
        project = self.c.request(f"{base}/{transfer}/finish", "POST")
        require(self.p.ssh.text(f"cat {quoted(project['path'] + '/copied.txt')}") == contents.decode().strip(), "Transfer changed contents")
        self.c.request(base, "POST", manifest, expected=409)
        cancelled = self.c.request(base, "POST", {**manifest, "name": name + "-cancel"})["id"]
        self.c.request(f"{base}/{cancelled}", "DELETE", expected=204)
        require(self.p.ssh.text(f"test ! -d {quoted(self.p.root + '/server/projects/.transfer-' + cancelled)}; printf ok") == "ok", "Cancelled staging retained")
        bad = {**manifest, "name": name + "-bad", "files": [{**manifest["files"][0], "sha256": "0" * 64}]}
        transfer = self.c.request(base, "POST", bad)["id"]
        self.c.request(f"{base}/{transfer}/chunk", "POST", {"path": "copied.txt", "offset": 0,
                       "contents": base64.b64encode(contents).decode()}, expected=204)
        self.c.request(f"{base}/{transfer}/finish", "POST", expected=400)
        require(self.p.ssh.text(f"test ! -d {quoted(self.p.root + '/server/projects/' + bad['name'])}; printf ok") == "ok", "Bad checksum project was published")

    def git_mutation(self):
        before = self.p.ssh.text(f"git -C {quoted(self.c.cwd)} rev-parse HEAD")
        self.c.call("write", path="file.txt", contents="git-mutation-" + secrets.token_hex(8))
        self.c.call("git-stage", path="file.txt")
        self.c.call("git-commit", contents="Manual remote E2E commit")
        after = self.p.ssh.text(f"git -C {quoted(self.c.cwd)} rev-parse HEAD")
        require(before != after, "Remote commit did not change HEAD")
        require(self.p.ssh.text(f"git -C {quoted(self.c.cwd)} show -s --format=%s HEAD") == "Manual remote E2E commit",
                "Wrong commit produced on server")

    def history_transfer(self):
        local = self.run.directory / ("history-" + secrets.token_hex(4))
        local.mkdir()
        def git(*args):
            return command(["git", "-C", local, *args], env=self.c.env).stdout.decode().strip()
        git("init", "-qb", "main")
        git("config", "user.name", "Remote E2E")
        git("config", "user.email", "e2e@example.invalid")
        (local / "file.txt").write_text("committed\n")
        git("add", ".")
        git("-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture")
        oid = git("rev-parse", "HEAD")
        bundle = self.run.directory / (local.name + ".bundle")
        git("bundle", "create", str(bundle), "--all")
        files = {"file.txt": b"uncommitted snapshot\n", ".dunlin-transfer.bundle": bundle.read_bytes()}
        manifest = {"name": local.name, "history": True, "branch": "main", "files": [
            {"path": path, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(), "executable": False}
            for path, data in files.items()]}
        base = "/v1/server/copies"
        ident = self.c.request(base, "POST", manifest)["id"]
        for path, data in files.items():
            for offset in range(0, len(data), 1024 * 1024):
                self.c.request(f"{base}/{ident}/chunk", "POST", {"path": path, "offset": offset,
                    "contents": base64.b64encode(data[offset:offset + 1024 * 1024]).decode()}, expected=204)
        project = self.c.request(f"{base}/{ident}/finish", "POST")
        require(self.p.ssh.text(f"git -C {quoted(project['path'])} rev-parse HEAD") == oid, "History changed during transfer")
        require(self.p.ssh.text(f"cat {quoted(project['path'] + '/file.txt')}") == "uncommitted snapshot", "Worktree snapshot changed")
        require((local / "file.txt").read_text() == "committed\n", "Local source was modified")

    def acp(self):
        evidence = self.c.acp()
        self.session = evidence["session_id"]
        self.run.report["facts"]["agent"] = {"name": evidence["agent"], "version": evidence["version"], "model": evidence["model"]}
        require(evidence["chunks"] > 0 and evidence["stop_reason"] == "end_turn", "Incomplete streamed ACP response")
        evidence_path = self.p.root + "/project/.sandbox-evidence.json"
        sandbox = json.loads(self.p.ssh.text("cat " + quoted(evidence_path)))
        require(sandbox == {"allowed_read": True, "host_secret_denied": True, "credentials_absent": True}, "Sandbox evidence missing")
        sessions = self.c.request(f"/v1/sessions?project={self.c.project_id}")
        require(any(row["id"] == self.session for row in sessions), "ACP session not indexed by relay")

    def resume(self):
        if not self.session:
            raise Blocked("A successfully created real ACP session is required")
        # A new client process and a new relay-owned OpenCode process load the
        # real persisted OpenCode session; the mock cannot satisfy this check.
        self.c.acp(expected="DUNLIN_RESUMED_OK", resume_session_id=self.session)

    def permissions(self, allow):
        filename = "agent-allowed.txt" if allow else "agent-denied.txt"
        self.p.ssh.exec("rm -f -- " + quoted(self.c.cwd + "/" + filename))
        marker = "DUNLIN_ALLOWED" if allow else "DUNLIN_DENIED"
        evidence = self.c.acp(
            prompt=(f"Use a file-writing tool to create {filename} containing exactly agent-written. "
                    f"If permission is {'granted' if allow else 'denied'}, reply with exactly {marker}. "
                    "If denied, stop immediately and do not try another tool. Do not just describe writing the file."),
            expected=marker, allow_permissions=allow, expect_denied=not allow)
        require(evidence["permissions"] > 0, "No actual ACP permission request was observed")
        path = self.c.cwd + "/" + filename
        if allow:
            require(self.p.ssh.text("cat " + quoted(path)) == "agent-written", "Approved tool did not write the remote file")
        else:
            self.p.ssh.exec("test ! -e " + quoted(path))

    def cancel(self):
        self.c.acp(prompt="Write a very long numbered explanation of TCP, at least 10000 words. Do not use tools.",
                   expected="cancel", cancel_after_ms=100, timeout_secs=60)

    def concurrent(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            jobs = [executor.submit(self.c.acp, expected=f"DUNLIN_CONCURRENT_{i}") for i in range(2)]
            evidence = [job.result(timeout=240) for job in jobs]
        require(len({row["session_id"] for row in evidence}) == 2, "Concurrent conversations share a session")

    def terminal(self):
        result = self.c.call("terminal")
        require(result["executed"] and result["resized"], "Remote PTY execution/resize missing")

    def bootstrap(self):
        if self.p.options.provider == "ssh-host":
            raise Blocked("Production bootstrap installs the normal service; select a disposable VM")
        binary = self.run.directory / "target-relay"
        result = self.p.ssh.exec("cat " + quoted(self.p.binary))
        binary.write_bytes(result.stdout)
        binary.chmod(0o700)
        original = self.c.url
        try:
            invalid = self.run.directory / "invalid-relay"
            invalid.write_bytes(b"not an executable for this server")
            try:
                self.c.tunnel(self.p, port=8877, bootstrap=invalid)
            except RuntimeError as error:
                require("does not match" in str(error), "Invalid artifact failed for an unrelated reason")
            else:
                raise AssertionError("Bootstrap accepted an incompatible executable")
            boot = self.c.tunnel(self.p, port=8877, bootstrap=binary)
            self.ready()
            pair = self.c.pair(boot["code"])
            token = self.c.token
            self.c.token = pair["token"]
            try:
                self.c.request("/v1/server")
                self.c.tunnel(self.p, port=8877, bootstrap=binary)
                self.c.request("/v1/me")
            finally:
                self.c.token = token
        finally:
            self.c.url = original

    def restart(self, crash=False):
        expected = self.c.call("read", path="file.txt")["contents"]
        restart_service(self.p, crash=crash)
        self.ready()
        require(self.c.call("read", path="file.txt")["contents"] == expected, "Confirmed file changed after relay restart")
        self.c.request("/v1/me")
        if self.session:
            self.resume()

    def reboot(self):
        # Be explicit about Linux user service start semantics. This check uses
        # lingering; ordinary login-only behavior is recorded separately.
        if self.p.platform == "Linux":
            if self.p.options.provider == "ssh-host" and not self.p.options.allow_system_changes:
                raise Blocked("Enabling lingering needs --allow-system-changes on a remote host")
            lingering = self.p.ssh.text('loginctl show-user "$(id -un)" -p Linger --value')
            if lingering != "yes":
                self.run.own("linger", ssh_config=str(self.p.ssh.config), target=self.p.ssh.target)
                self.p.ssh.exec('sudo -n loginctl enable-linger "$(id -un)"')
        self.p.reboot()
        self.ready()
        self.c.request("/v1/me")
        if self.session:
            self.resume()

    def network(self):
        original = self.c.url
        parsed = urlparse(original)
        proxy = Proxy((parsed.hostname, parsed.port))
        self.c.url = f"http://127.0.0.1:{proxy.port}"
        try:
            proxy.delay = 0.1
            proxy.bytes_per_second = 32768
            self.c.call("read", path="file.txt")
            proxy.delay, proxy.bytes_per_second = 0, 0
            proxy.blackhole = True
            self.c.call("read", path="file.txt", timeout=45, expect_failure=True)
            proxy.blackhole = False
            proxy.reset()
            self.c.call("read", path="file.txt")
            self.c.acp(expected="DUNLIN_NETWORK_RECOVERED")
        finally:
            proxy.close()
            self.c.url = original

    def acp_reset(self):
        original = self.c.url
        parsed = urlparse(original)
        proxy = Proxy((parsed.hostname, parsed.port))
        self.c.url = f"http://127.0.0.1:{proxy.port}"
        process = None
        try:
            spec = self.c.manifest("acp", payload={"prompt": "Explain TCP in at least 10000 words.",
                                                   "expected": "should-not-complete", "timeout_secs": 30})
            process = subprocess.Popen([self.c.binary, "--remote-e2e"], env=self.c.env,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            self.run.processes.append(process)
            process.stdin.write((json.dumps(spec) + "\n").encode())
            process.stdin.close()
            deadline = time.monotonic() + 25
            saw_prompt = False
            while time.monotonic() < deadline:
                if select.select([process.stderr], [], [], 0.2)[0]:
                    line = process.stderr.readline()
                    if b"ACP_ACCEPTANCE: prompt_started" in line:
                        saw_prompt = True
                        break
                    if not line:
                        break
            require(saw_prompt, "Did not observe actual ACP prompt before fault")
            proxy.reset()
            require(process.wait(timeout=35) != 0, "Interrupted ACP falsely reported success")
            self.c.acp(expected="DUNLIN_AFTER_RESET")
        finally:
            if process:
                stop(process)
            proxy.close()
            self.c.url = original

    def uncertain_write(self):
        original = self.c.url
        parsed = urlparse(original)
        proxy = Proxy((parsed.hostname, parsed.port), inspect_http=True)
        self.c.url = f"http://127.0.0.1:{proxy.port}"
        marker = "confirmed-on-server-" + secrets.token_hex(8)
        try:
            proxy.drop_responses = True
            self.c.call("write", path="file.txt", contents=marker, expect_failure=True, timeout=45)
            require(self.p.ssh.text("cat " + quoted(self.c.cwd + "/file.txt")) == marker,
                    "Fault was not injected after the server mutation")
            require(proxy.request_counts.get("PUT") == 1, "A response-lost mutation was automatically retried")
            proxy.drop_responses = False
            require(self.c.call("read", path="file.txt")["contents"] == marker, "Cannot reconcile uncertain save")
        finally:
            proxy.close()
            self.c.url = original

    def ui(self):
        if not self.p.options.allow_keychain:
            raise Blocked("Native pairing needs --allow-keychain; use a disposable macOS login")
        # Exercise the actual credential store first. A headless/unauthorized
        # login must not spend four minutes waiting on an OS authorization UI,
        # nor be "fixed" by replacing production token storage with a mock.
        private_json(self.run.directory / "keychain-probe.json", {"id": self.run.id})
        probe_token = secrets.token_hex(24)
        self.run.secrets.append(probe_token)
        try:
            self.c.call("keychain", token=probe_token,
                        payload={"consent": True, "host_id": self.run.id}, timeout=15)
        except (AssertionError, subprocess.TimeoutExpired) as error:
            raise Blocked(f"Production macOS Keychain prerequisite unavailable: {type(error).__name__}; use an authorized disposable login") from None
        fixture = self.run.directory / ("remote-ui-client-" + secrets.token_hex(4))
        self.c.call("write", path="file.txt", contents="before-gui-save-" + secrets.token_hex(8))
        code = self.code()
        self.c.call("ui", fixture=str(fixture), code=code,
                    payload={"keychain_consent": True}, timeout=600)
        require((fixture / "PASS").is_file(), "Native UI did not complete its assertions")
        (fixture / "PASS").unlink()
        restart_service(self.p)
        self.ready()
        self.c.call("ui", fixture=str(fixture), payload={"keychain_consent": True, "restart": True}, timeout=300)
        require((fixture / "PASS").is_file(), "Fresh app instance did not restore the remote workspace")
        first = json.loads((fixture / "first-launch.json").read_text())
        resumed = json.loads((fixture / "restart.json").read_text())
        require(first["pid"] != resumed["pid"], "Restart reused the first app process")
        require(first["session_id"] == resumed["session_id"], "Restart silently created a new ACP conversation")
        require(first["host_id"] == resumed["host_id"], "Restart silently re-paired the client")
        require(first["keychain_read"] and resumed["keychain_read"], "Production Keychain persistence not exercised")
        for evidence in (first, resumed):
            require(evidence["model"] == self.c.model and "opencode" in evidence["agent"].lower(),
                    "GUI did not use the selected real anonymous OpenCode model")
        require(self.p.ssh.text("cat " + quoted(self.c.cwd + "/file.txt")) == "Remote UI persisted",
                "GUI save is not present on the separate server")
        require((fixture / "local-project/file.txt").read_text() == "local-canary", "GUI overwrote the local project")
        self.run.report["facts"]["remote_ui"] = {"fixture": fixture.name, "first_launch": first, "restart": resumed,
            "client_home": "OS login home for production Keychain; private XDG and Dunlin DB",
            "server_credentials": "isolated; anonymous free model only"}

    def keyring(self):
        helper = self.p.root + "/bin/vault_acceptance"
        if self.p.ssh.exec("test -x " + quoted(helper), check=False).returncode:
            raise Blocked("Build the manual vault_acceptance helper in the guest")
        if self.p.options.provider == "ssh-host" and not self.p.options.allow_keychain:
            raise Blocked("Remote OS keyring acceptance needs --allow-keychain")
        secret = secrets.token_hex(24)
        self.run.secrets.append(secret)
        if self.p.platform == "Linux" and self.p.options.provider != "ssh-host":
            password = secrets.token_hex(24)
            self.run.secrets.append(password)
            home = self.p.root + "/private-keyring-" + secrets.token_hex(6)
            self.p.ssh.exec("umask 077; mkdir " + quoted(home))
            self.p.ssh.put(ROOT / "scripts/remote_e2e/keyring_driver.py", self.p.root + "/keyring_driver.py")
            result = self.p.ssh.exec(f"dbus-run-session -- env HOME={quoted(home)} XDG_DATA_HOME={quoted(home + '/data')} "
                f"XDG_CONFIG_HOME={quoted(home + '/config')} python3 {quoted(self.p.root + '/keyring_driver.py')}",
                input=json.dumps({"password": password, "id": self.run.id, "secret": secret, "helper": helper}).encode(), timeout=90)
            evidence = json.loads(result.stdout)
            require(evidence.get("deleted") and evidence.get("failed_closed"), "Private OS keyring evidence incomplete")
            self.run.report["facts"]["linux_keyring_scope"] = "private D-Bus/Secret Service; no login-keyring unlock claim"
            return
        elif self.p.platform == "Linux":
            # Never initialize/change an existing real host's login-keyring
            # password. The operator must supply an already unlocked session.
            prefix = 'DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus" '
        else:
            prefix = ""
        payload = {"id": self.run.id, "secret": secret}
        result = self.p.ssh.exec(prefix + quoted(helper), input=json.dumps(payload).encode(), timeout=40)
        require(json.loads(result.stdout)["deleted"], "OS keyring roundtrip incomplete")
        if self.p.platform == "Linux":
            result = self.p.ssh.exec("DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/dunlin-bus " + quoted(helper),
                    input=json.dumps({**payload, "expect_unavailable": True}).encode(), timeout=40)
            require(json.loads(result.stdout)["failed_closed"], "OS keyring silently fell back")

    def installer(self):
        if self.p.options.provider == "ssh-host":
            raise Blocked("The public installer may replace a normal service; select a disposable VM")
        # The installer owns the real default port, service and tool locations.
        # Quiesce our separate fixture rather than accepting somebody else's
        # /health response as proof the newly installed server started.
        if self.p.platform == "Linux":
            self.p.ssh.exec(f"systemctl --user stop {quoted(self.run.id)}.service")
        else:
            self.p.ssh.exec(f"launchctl bootout gui/$(id -u)/{quoted(self.run.id)}")
        try:
            published_installer(self.p, candidate=not self.p.options.published_installer)
            installed = self.p.home + "/.local/bin/dunlin-relay"
            db = self.p.home + ("/Library/Application Support/Dunlin/relay.db" if self.p.platform == "Darwin"
                               else "/.local/share/dunlin/relay.db")
            tools = str(Path(db).parent) + "/server/tools/bin/opencode"
            self.run.report["facts"]["installed_harness_version"] = self.p.ssh.text(quoted(tools) + " --version")
            name = "installed-" + secrets.token_hex(4)
            self.p.ssh.exec(" ".join(quoted(a) for a in [installed, "project", "add", name, self.p.root + "/project",
                                                       "--user", "admin", "--db", db]))
            output = self.p.ssh.text(" ".join(quoted(a) for a in [installed, "pair", "admin", "--db", db]))
            code = re.search(r"Pairing code for .*?: ([A-Z0-9]+)", output).group(1)
            previous = self.c.token, self.c.project_id, self.c.cwd
            try:
                self.c.token = self.c.pair(code)["token"]
                project = next(p for p in self.c.call("projects") if p["name"] == name)
                self.c.project_id, self.c.cwd = project["id"], project["path"]
                self.c.acp(expected="DUNLIN_INSTALLED_OPENCODE_OK")
            finally:
                self.c.token, self.c.project_id, self.c.cwd = previous
        finally:
            if self.p.platform == "Linux":
                try:
                    self.p.ssh.exec("set -eu; STATE=$(systemctl --user show dunlin-relay.service -p LoadState --value); "
                                    'if [ "$STATE" != not-found ]; then systemctl --user disable --now dunlin-relay.service; fi')
                finally:
                    restart_service(self.p)
            else:
                self.p.ssh.exec("launchctl bootout gui/$(id -u)/dev.dunlin.relay >/dev/null 2>&1 || true; "
                               "rm -f ~/Library/LaunchAgents/dev.dunlin.relay.plist; "
                               f"launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/{self.run.id}.plist")
            self.ready()

    def public_endpoint(self):
        if not self.p.options.endpoint:
            raise Blocked("Real-network profile requires an explicit --endpoint HTTPS URL")
        require(self.p.options.endpoint.startswith("https://"), "External endpoint must be HTTPS")
        original, ca = self.c.url, self.c.ca
        try:
            self.c.url, self.c.ca = self.p.options.endpoint, None
            self.c.request("/v1/me")
            self.c.acp(expected="DUNLIN_PUBLIC_TLS_OK")
        finally:
            self.c.url, self.c.ca = original, ca

    def namespace_failure(self):
        if self.p.platform != "Linux":
            raise Blocked("The namespace-startup check is Linux-specific")
        wrapper = self.p.root + "/server/tools/bin/bwrap"
        self.p.ssh.exec(f"test ! -e {quoted(wrapper)}; cat > {quoted(wrapper)}; chmod 700 {quoted(wrapper)}",
                        input=b"#!/bin/sh\necho 'fixture: namespaces unavailable' >&2\nexit 78\n")
        try:
            self.c.call("acp", payload={"prompt": "Hello", "expected": "Hello", "timeout_secs": 20}, expect_failure=True)
        finally:
            self.p.ssh.exec("rm -f -- " + quoted(wrapper))
        self.c.acp(expected="DUNLIN_SANDBOX_RESTORED")
