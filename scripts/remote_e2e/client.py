"""Invoke Dunlin's production transports, not Python reimplementations."""
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess

from .core import Blocked, command, isolated_env, require, stop


def ui_completion(fixture, stderr, restart):
    # AppKit may terminate the process from cx.quit() rather than return to
    # main's JSON printer. Accept only the real named-step/fixture protocol,
    # never exit zero or a stale PASS file on its own.
    require((fixture / "PASS").read_text() == "manual-remote-e2e", "Missing exact native completion marker")
    steps = [json.loads(line.removeprefix("GUI_STEP: ")) for line in stderr.splitlines()
             if line.startswith("GUI_STEP: ")]
    require(steps and all(step["scenario"] == "manual-remote-e2e" for step in steps), "Missing native step evidence")
    require(all(step["status"] in ("started", "passed") for step in steps), "Native workflow has a failed step")
    started = [step["step"] for step in steps if step["status"] == "started"]
    passed = [step["step"] for step in steps if step["status"] == "passed"]
    require(started == passed, "Native workflow ended with incomplete steps")
    required = {"saved remote contents and local canary unchanged", "real anonymous OpenCode reply displayed",
                "record real GUI session and Keychain persistence evidence"}
    if restart:
        required.update({"same remote conversation selected after fresh app launch", "earlier remote reply restored"})
    else:
        required.update({"click remote-e2e-connect", "press cmd-s"})
    require(required <= set(passed), "Native workflow omitted a required assertion")
    evidence = json.loads((fixture / ("restart.json" if restart else "first-launch.json")).read_text())
    require(evidence["phase"] == ("restart" if restart else "first-launch"), "Wrong native completion phase")
    return {"ui_completed": True, "steps": len(passed)}


class Client:
    def __init__(self, run, binary, model):
        self.run, self.binary, self.model = run, Path(binary).resolve(), model
        self.env = isolated_env(run.directory / "client-home")
        blocked = run.directory / "blocked-host-clis"
        blocked.mkdir(exist_ok=True, mode=0o700)
        # The SUT's agent is server-side. Never start a personal local agent or
        # account-discovery CLI when the native app first shows its local canary.
        for name in ("gh", "glab", "opencode", "claude", "codex", "claude-agent-acp", "codex-acp"):
            executable = blocked / name
            executable.write_text("#!/bin/sh\nexit 1\n")
            executable.chmod(0o700)
        self.env["PATH"] = str(blocked) + os.pathsep + self.env.get("PATH", os.defpath)
        self.url, self.token, self.project_id, self.cwd = "", "", "", ""
        self.ca = None
        self.count = 0

    def manifest(self, action, **kwargs):
        return {"action": action, "url": self.url, "token": self.token,
                "project_id": self.project_id, "cwd": self.cwd, "model": self.model,
                "tls_ca_pem": self.ca, **kwargs}

    def environment(self, action, payload):
        env = self.env.copy()
        if action in ("ui", "keychain"):
            consent = "keychain_consent" if action == "ui" else "consent"
            require(payload.get(consent) is True, "Native Keychain access requires explicit consent")
            # Security.framework resolves the default file Keychain through
            # HOME. An empty HOME has no default Keychain and produces a
            # misleading authorization error. Only native credential-store/UI
            # probes use the OS login home; XDG/app data remain run-private and
            # no credential-bearing environment variables are restored.
            import pwd
            env["HOME"] = pwd.getpwuid(os.getuid()).pw_dir
        return env

    def call(self, action, *, timeout=240, expect_failure=False, **kwargs):
        self.count += 1
        index = self.count
        if action == "ui":
            require(not (Path(kwargs["fixture"]) / "PASS").exists(), "Refuse stale native completion marker")
        result = command([self.binary, "--remote-e2e"], env=self.environment(action, kwargs.get("payload", {})),
                         input=(json.dumps(self.manifest(action, **kwargs)) + "\n").encode(),
                         timeout=timeout, check=False)
        text = result.stderr.decode(errors="replace")
        self.run.log(f"client-{index:03d}-{action}", text)
        if expect_failure:
            require(result.returncode != 0, f"{action} unexpectedly succeeded")
            return {"failed": True}
        if action == "ui" and result.returncode and "keychain" in text.lower() and any(
                phrase in text.lower() for phrase in ("unable to obtain authorization", "interaction is not allowed", "authorization was canceled")):
            raise Blocked("macOS denied the production Keychain write. Run native pairing in an authorized disposable login; no credential-store bypass was attempted")
        require(result.returncode == 0, f"Production client {action} failed: {self.run.redact(text[-3000:])}")
        if action == "ui":
            return ui_completion(Path(kwargs["fixture"]), text, kwargs.get("payload", {}).get("restart") is True)
        lines = result.stdout.decode().splitlines()
        require(bool(lines), f"Missing structured {action} evidence")
        return json.loads(lines[-1])

    def request(self, path, method="GET", body=None, expected=200, **kwargs):
        payload = {"method": method}
        if body is not None:
            payload["body"] = body
        result = self.call("request", path=path, payload=payload, **kwargs)
        require(result["status"] == expected, f"{method} {path.split('?')[0]}: expected {expected}, got {result['status']}")
        return result["body"]

    def pair(self, code):
        self.run.secrets.append(code)
        result = self.call("pair", code=code)
        self.run.secrets.append(result["token"])
        return result

    def acp(self, prompt=None, expected="DUNLIN_REMOTE_OK", **options):
        return self.call("acp", payload={"prompt": prompt or f"Reply with exactly {expected}. Do not use tools.",
                                        "expected": expected, **options})

    def tunnel(self, provider, *, port=8787, bootstrap=None, config=None):
        config = str(config or provider.ssh.config)
        # Keep the Unix socket short enough for macOS sockaddr_un. Identity is
        # journaled before starting; cleanup never guesses a socket pathname.
        digest = hashlib.sha256(self.run.id.encode()).hexdigest()[:16]
        socket = str(Path(os.environ.get("TMPDIR", "/tmp")) / f"de-{digest}.sock")
        self.run.own("ssh-master", config=config, socket=socket, target=provider.ssh.target)
        payload = {"ssh_config": config, "control_socket": socket,
                   "host": {"id": self.run.id, "name": "Manual fixture", "target": provider.ssh.target,
                            "user": "admin", "port": 0, "identity": None, "relay_port": port,
                            "local_port": 0, "host_key": "", "created_at": 0}}
        if bootstrap:
            payload["relay_binary"] = str(bootstrap)
        log_path = self.run.directory / f"tunnel-{len(self.run.processes)}.log"
        log = log_path.open("wb")
        process = subprocess.Popen([self.binary, "--remote-e2e"], env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log,
                                   start_new_session=True)
        log.close()
        self.run.processes.append(process)
        process.stdin.write((json.dumps(self.manifest("bootstrap" if bootstrap else "tunnel", payload=payload)) + "\n").encode())
        process.stdin.flush()
        if not select.select([process.stdout], [], [], 60)[0]:
            stop(process)
            raise TimeoutError("Production SSH tunnel/bootstrap did not report readiness")
        line = process.stdout.readline()
        if not line:
            process.wait(timeout=5)
            raise RuntimeError("Production SSH tunnel/bootstrap failed: " + self.run.redact(log_path.read_text()[-3000:]))
        result = json.loads(line)
        self.url = result["url"]
        return result
