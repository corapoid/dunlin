#!/usr/bin/env python3
"""Run isolated automatic release checks; never claim manual acceptance.

No provider requests, GitHub writes, installed-app replacement or relay restart.
All logs/reports/fixtures use one new disposable directory. Native tests are
explicitly opt-in because hosted runners may not have an interactive desktop.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]


def run(argv, log, env, timeout):
    with log.open("w") as output:
        process = subprocess.Popen(argv, cwd=ROOT, env=env, stdout=output,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            return process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
            if isinstance(error, KeyboardInterrupt):
                raise
            return 124


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tmp-root", type=Path)
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--audit", action="store_true", help="Also run installed cargo-deny with a fresh advisory database")
    args = parser.parse_args()
    output = Path(tempfile.mkdtemp(prefix="dunlin-release-qa-", dir=args.tmp_root))
    env = {k: v for k, v in os.environ.items() if not k.startswith(("DUNLIN_", "GIT_"))}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    env["TMPDIR"] = str(output)
    cargo = shutil.which("cargo")
    if not cargo:
        cargo = subprocess.check_output(["rustup", "which", "cargo"], text=True).strip()
    env["PATH"] = str(Path(cargo).parent) + os.pathsep + env.get("PATH", "")
    checks = [
        ("release-tool-tests", ["python3", "-m", "unittest", "discover", "-s", "scripts/tests", "-v"], 60),
        ("vendor-integrity", ["python3", "scripts/verify_vendor.py"], 60),
        ("format", [cargo, "fmt", "--all", "--", "--check"], 120),
        ("clippy", [cargo, "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"], 1200),
        ("tests", [cargo, "test", "--workspace", "--locked"], 1200),
        ("build", [cargo, "build", "-p", "dunlin", "-p", "dunlin-relay", "-p", "dunlin-agent", "--bin", "dunlin", "--bin", "dunlin-relay", "--bin", "mock_acp_agent", "--locked"], 1200),
        ("relay", ["bash", "scripts/relay-smoke.sh"], 180),
        ("git-http", ["python3", "scripts/git-operations-smoke.py", "--tmp-root", str(output)], 180),
    ]
    if args.audit:
        checks.append(("dependency-audit", [cargo, "deny", "--locked", "check", "advisories", "licenses", "bans", "sources"], 180))
    if args.native:
        for kind in ["git", "markdown", "github", "workbench", "glass"]:
            for light in ([False] if kind == "glass" else [False, True]):
                fixture = "markdown" if kind == "markdown" else "glass" if kind == "glass" else "git"
                name = f"{kind}-{'light-narrow' if light else 'dark'}"
                command = [str(ROOT / "target/debug/dunlin"), f"--{kind}-smoke",
                           str(output / f"dunlin-{fixture}-smoke-{uuid.uuid4()}")]
                if light:
                    command.append("--light-narrow")
                checks.append((name, command, 100))
    report = {"scope": "automatic-only", "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
              "native_requested": args.native, "dependency_audit_requested": args.audit, "checks": [],
              "not_accepted_by_this_runner": ["fresh macOS account/TCC denial", "manual visual/keyboard/IME/bidi/a11y acceptance",
                                               "real GitHub write/merge lifecycle", "deployed-host relay", "Apple Developer ID/notarization"]}
    destination = output / "report.json"
    try:
        for name, command, timeout in checks:
            try:
                code = run(command, output / f"{name}.log", env, timeout)
            except OSError as error:
                (output / f"{name}.log").write_text(str(error))
                code = 127
            report["checks"].append({"name": name, "exit_code": code, "log": f"{name}.log"})
            print(f"{name}: {'PASS' if code == 0 else 'FAIL'} ({code})", flush=True)
            destination.write_text(json.dumps(report, indent=2))
    finally:
        report["automatic_passed"] = len(report["checks"]) == len(checks) and all(c["exit_code"] == 0 for c in report["checks"])
        destination.write_text(json.dumps(report, indent=2))
        print(f"Report: {destination}", flush=True)
    return int(not report["automatic_passed"])


if __name__ == "__main__":
    raise SystemExit(main())
