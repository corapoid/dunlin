"""Bounded subprocesses, private artifacts, explicit ownership and evidence."""
from __future__ import annotations

import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time
import uuid
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
RUN_PREFIX = "dunlin-remote-e2e-"


class Blocked(RuntimeError):
    """A selected check cannot run; never equivalent to success."""


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def private_json(path: Path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(data, stream, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def isolated_env(home: Path):
    home.mkdir(parents=True, exist_ok=True, mode=0o700)
    env = {k: v for k, v in os.environ.items()
           if k in {"PATH", "LANG", "TZ", "__CF_USER_TEXT_ENCODING"} or k.startswith("LC_")}
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"),
               XDG_DATA_HOME=str(home / ".local/share"), XDG_STATE_HOME=str(home / ".local/state"),
               XDG_CACHE_HOME=str(home / ".cache"), TMPDIR=str(home),
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
               GIT_TERMINAL_PROMPT="0", NO_COLOR="1")
    return env


def stop(process):
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
    except ProcessLookupError:
        pass


def command(argv, *, cwd=None, env=None, input=None, timeout=120, check=True):
    process = subprocess.Popen([str(a) for a in argv], cwd=cwd, env=env,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    try:
        stdout, stderr = process.communicate(input, timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        stop(process)
        process.communicate()
        raise
    finally:
        for pipe in (process.stdin, process.stdout, process.stderr):
            if pipe and not pipe.closed:
                pipe.close()
    if check and process.returncode:
        # Never include argv (it can contain a pairing command) in diagnostics.
        raise RuntimeError(f"{Path(str(argv[0])).name} exited {process.returncode}: "
                           + stderr.decode(errors="replace")[-4000:])
    return subprocess.CompletedProcess(argv, process.returncode, stdout, stderr)


def wait_until(action, timeout=30):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            value = action()
            if value:
                return value
        except (RuntimeError, OSError, AssertionError) as error:
            last = error
        time.sleep(0.15)
    raise TimeoutError(f"Readiness deadline exceeded ({type(last).__name__ if last else 'not ready'})")


def validate_run(directory):
    directory = Path(directory).resolve(strict=True)
    require(directory.name.startswith(RUN_PREFIX), "Not a remote-e2e run directory")
    manifest = json.loads((directory / "ownership.json").read_text())
    require(manifest.get("schema") == 1 and manifest.get("directory") == str(directory),
            "Ownership manifest mismatch")
    require(manifest.get("run_id") == directory.name, "Run identity mismatch")
    require((directory / "ownership.json").stat().st_uid == os.getuid(), "Run belongs to another user")
    return manifest


def lock_run(directory):
    import fcntl
    stream = (Path(directory) / "active.lock").open("a")
    try:
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        stream.close()
        raise Blocked("Another command owns this run; refusing concurrent execution/cleanup") from None
    return stream


class Run:
    @classmethod
    def load(cls, directory, profile):
        run = cls.__new__(cls)
        run.directory = Path(directory).resolve(strict=True)
        run.ownership = validate_run(run.directory)
        require(not run.ownership["cleanup_complete"], "This environment has already been cleaned")
        run.id = run.directory.name
        run.report = json.loads((run.directory / "report.json").read_text())
        attempt = len(list(run.directory.glob("attempt-*.json"))) + 1
        private_json(run.directory / f"attempt-{attempt:03d}.json", run.report)
        run.report["previous_attempts"] = attempt
        run.report["checks"] = []
        # Deployment facts survive reuse; acceptance evidence belongs to the
        # archived attempt and must not appear as this attempt's GUI success.
        run.report.get("facts", {}).pop("remote_ui", None)
        run.report["profile"] = profile
        run.report["passed"] = False
        run.secrets, run.processes = [], []
        return run

    def __init__(self, profile, parent=None):
        parent = Path(parent or os.environ.get("TMPDIR", tempfile.gettempdir())).resolve()
        parent.mkdir(parents=True, exist_ok=True)
        self.directory = Path(tempfile.mkdtemp(prefix=RUN_PREFIX, dir=parent))
        self.directory.chmod(0o700)
        self.id = self.directory.name
        self.secrets = []
        self.processes = []
        self.ownership = {"schema": 1, "directory": str(self.directory), "run_id": self.id,
                          "resources": [], "cleanup_complete": False}
        self.report = {"schema": 1, "profile": profile, "trigger": "manual", "run_id": self.id,
                       "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                       "checks": [], "facts": {}, "passed": False,
                       "not_claimed": ["physical input/IME/VoiceOver/TCC acceptance",
                                       "provider availability beyond this run"]}
        self.flush()

    def redact(self, text):
        for secret in sorted(set(self.secrets), key=len, reverse=True):
            if secret:
                text = text.replace(secret, "<redacted>")
        return re.sub(r"(?i)(authorization:\s*bearer\s+)\S+", r"\1<redacted>", text)

    def log(self, name, text):
        path = self.directory / f"{name}.log"
        path.write_text(self.redact(text))
        path.chmod(0o600)

    def own(self, kind, **fields):
        resource = {"kind": kind, "cleaned": False, **fields}
        self.ownership["resources"].append(resource)
        self.flush()
        return resource

    def check(self, name, action):
        started = time.monotonic()
        try:
            detail = action()
            status = "PASS"
        except KeyboardInterrupt:
            self.report["checks"].append({"name": name, "status": "FAIL", "detail": "Interrupted"})
            self.flush()
            raise
        except Exception as error:
            status = "BLOCKED" if isinstance(error, Blocked) else "FAIL"
            detail = f"{type(error).__name__}: {error}"
        entry = {"name": name, "status": status, "seconds": round(time.monotonic() - started, 3),
                 "detail": self.redact(str(detail or ""))}
        self.report["checks"].append(entry)
        print(f"{name}: {status}", flush=True)
        self.flush()
        return status == "PASS"

    def flush(self):
        private_json(self.directory / "ownership.json", self.ownership)
        private_json(self.directory / "report.json", json.loads(self.redact(json.dumps(self.report))))
        suite = ET.Element("testsuite", name="manual-remote-e2e", tests=str(len(self.report["checks"])))
        for check in self.report["checks"]:
            case = ET.SubElement(suite, "testcase", name=check["name"], time=str(check.get("seconds", 0)))
            if check["status"] != "PASS":
                ET.SubElement(case, "failure", type=check["status"]).text = check["detail"]
        ET.ElementTree(suite).write(self.directory / "junit.xml", encoding="unicode")
        summary = [f"# Manual remote E2E: {self.report['profile']}", "",
                   "| Check | Result |", "|---|---|"]
        summary.extend(f"| {c['name']} | {c['status']} |" for c in self.report["checks"])
        (self.directory / "summary.md").write_text("\n".join(summary) + "\n")


def opencode_config(model):
    require(model.startswith("opencode/") and model.endswith("-free"),
            "Only explicitly free anonymous OpenCode models are accepted; no paid fallback")
    return {"$schema": "https://opencode.ai/config.json", "model": model,
            "snapshots": False, "plugins": [],
            "policies": [{"action": "provider.use", "resource": "*", "effect": "deny"},
                         {"action": "provider.use", "resource": "opencode", "effect": "allow"}],
            "permissions": [{"action": "*", "resource": "*", "effect": "ask"}]}


def free_models(executable, home, cwd=None):
    result = command([executable, "--standalone", "models"], env=isolated_env(home),
                     cwd=cwd or home, timeout=90)
    models = [line.strip() for line in result.stdout.decode().splitlines()
              if re.fullmatch(r"opencode/[a-zA-Z0-9._-]+-free", line.strip())]
    if not models:
        raise Blocked("Anonymous OpenCode catalog contains no explicitly free models")
    return sorted(set(models))
