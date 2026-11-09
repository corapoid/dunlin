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
import re
import shutil
import signal
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]


def native_environment(directory, parent):
    """Do not inherit account catalogs, provider keys, SSH agents or app settings."""
    directory.mkdir(parents=True, exist_ok=False)
    home = directory / "home"
    home.mkdir()
    blocked = directory / "bin"
    blocked.mkdir()
    # gh/glab can use the OS Keychain even with an empty HOME. Individual
    # fixtures prepend their own fake CLIs when testing hosted imports.
    for command in ["gh", "glab"]:
        executable = blocked / command
        executable.write_text("#!/bin/sh\nexit 1\n")
        executable.chmod(0o700)
    env = {key: value for key, value in parent.items()
           if key in {"PATH", "LANG", "TZ", "__CF_USER_TEXT_ENCODING"} or key.startswith("LC_")}
    env.update(
        HOME=str(home), TMPDIR=str(directory),
        PATH=os.pathsep.join([str(blocked)] + [p for p in parent.get("PATH", os.defpath).split(os.pathsep)
                                             if Path(p).is_absolute()]),
        XDG_CONFIG_HOME=str(home / ".config"), XDG_DATA_HOME=str(home / ".local/share"),
        XDG_CACHE_HOME=str(home / ".cache"), CODEX_HOME=str(home / ".codex"),
        CLAUDE_CONFIG_DIR=str(home / ".claude"),
        DUNLIN_DATA_DIR=str(directory / "fallback-data"),
        DUNLIN_LOG_FILE=str(directory / "dunlin.log"),
        GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0",
    )
    return env


def native_checks(binary, output, env):
    """Discover supported scenarios without opening a window or application data."""
    result = subprocess.run([str(binary), "--list-native-smokes"], env=env,
                            capture_output=True, text=True, check=True, timeout=10)
    registry = json.loads(result.stdout)
    if not isinstance(registry, list) or not registry:
        raise ValueError("Native smoke registry must be a nonempty list")
    checks, flags = [], set()
    for entry in registry:
        if not isinstance(entry, dict):
            raise ValueError("Invalid native smoke entry")
        flag, fixture = entry.get("flag"), entry.get("fixture")
        light, timeout = entry.get("light_narrow"), entry.get("timeout_secs")
        if (not isinstance(flag, str) or not re.fullmatch(r"--[a-z-]+-smoke", flag)
                or flag in flags or type(light) is not bool
                or type(timeout) is not int or not 1 <= timeout <= 300
                or (fixture is not None and (not isinstance(fixture, str)
                                            or not re.fullmatch(r"[a-z]+(?:-[a-z]+)*", fixture)))):
            raise ValueError("Invalid or duplicate native smoke registration")
        flags.add(flag)
        for narrow in ([False, True] if light else [False]):
            name = flag[2:] + ("-light-narrow" if narrow else "")
            command = [str(binary), flag]
            if fixture is not None:
                command.append(str(output / f"dunlin-{fixture}-smoke-{uuid.uuid4()}"))
            if narrow:
                command.append("--light-narrow")
            checks.append((name, command, timeout))
    return checks


def run_native(command, log, env, timeout):
    code = run(command, log, env, timeout)
    # Closing a fixture window early may exit successfully without completing
    # its assertions. Exit zero alone is not acceptance evidence.
    if code == 0 and not any(line.startswith("PASS:") for line in log.read_text(errors="replace").splitlines()):
        with log.open("a") as output:
            output.write("\nFAIL: native scenario exited without its completion marker\n")
        return 1
    return code


def verify_native_cli(binary, output, env):
    """Test the real entry point, not just its argument-validation helper."""
    for index, flag in enumerate(["--github-smoke", "--unknown-smoke", "--save-smoke"]):
        code = run([str(binary), flag], output / f"native-cli-{index}.log", env, 5)
        if code != 2:
            raise ValueError(f"Invalid CLI mode {flag} returned {code}, expected 2 before GUI startup")
        if Path(env["DUNLIN_DATA_DIR"]).exists() or Path(env["DUNLIN_LOG_FILE"]).exists():
            raise ValueError("Invalid CLI mode initialized application data or logging")


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
    parser.add_argument("--native-only", action="store_true", help="Run every native UI scenario using an already built debug binary")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/dunlin")
    parser.add_argument("--audit", action="store_true", help="Also run installed cargo-deny with a fresh advisory database")
    args = parser.parse_args()
    if args.native_only and args.audit:
        parser.error("--audit cannot be combined with --native-only")
    output = Path(tempfile.mkdtemp(prefix="dunlin-release-qa-", dir=args.tmp_root)).resolve()
    env = {k: v for k, v in os.environ.items() if not k.startswith(("DUNLIN_", "GIT_"))}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    env["TMPDIR"] = str(output)
    cargo = "cargo" if args.native_only else shutil.which("cargo")
    if not cargo:
        cargo = subprocess.check_output(["rustup", "which", "cargo"], text=True).strip()
    if Path(cargo).is_absolute():
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
    if args.native_only:
        checks = []
    native = args.native or args.native_only
    report = {"scope": "automatic-only", "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
               "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
               "native_requested": native, "native_only": args.native_only,
               "dependency_audit_requested": args.audit and not args.native_only, "checks": [],
              "not_accepted_by_this_runner": ["fresh macOS account/TCC denial", "manual visual/keyboard/IME/bidi/a11y acceptance",
                                               "real GitHub write/merge lifecycle", "deployed-host relay", "Apple Developer ID/notarization"]}
    destination = output / "report.json"

    def execute(checks, native=False):
        for name, command, timeout in checks:
            try:
                check_env = native_environment(output / name, env) if native else env
                code = (run_native if native else run)(command, output / f"{name}.log", check_env, timeout)
            except OSError as error:
                (output / f"{name}.log").write_text(str(error))
                code = 127
            report["checks"].append({"name": name, "exit_code": code, "log": f"{name}.log"})
            print(f"{name}: {'PASS' if code == 0 else 'FAIL'} ({code})", flush=True)
            destination.write_text(json.dumps(report, indent=2))

    expected = len(checks)
    completed = False
    try:
        execute(checks)
        if native:
            try:
                if any(c["name"] == "build" and c["exit_code"] != 0 for c in report["checks"]):
                    raise ValueError("Cannot run native tests after a failed build")
                discovery_env = native_environment(output / "native-discovery", env)
                scenarios = native_checks(args.binary.resolve(), output, discovery_env)
                verify_native_cli(args.binary.resolve(), output, discovery_env)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                (output / "native-discovery.log").write_text(str(error))
                report["checks"].append({"name": "native-discovery", "exit_code": 1, "log": "native-discovery.log"})
                expected += 1
            else:
                expected += len(scenarios)
                execute(scenarios, native=True)
        completed = True
    finally:
        report["automatic_passed"] = completed and expected > 0 and len(report["checks"]) == expected and all(c["exit_code"] == 0 for c in report["checks"])
        destination.write_text(json.dumps(report, indent=2))
        print(f"Report: {destination}", flush=True)
    return int(not report["automatic_passed"])


if __name__ == "__main__":
    raise SystemExit(main())
