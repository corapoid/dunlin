"""Manual CLI. Nothing here is wired into CI, release QA or default Rust tests."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys

from .client import Client
from .core import ROOT, Blocked, Run, command, free_models, private_json, require, sha256, stop, validate_run, lock_run
from .deployment import deploy
from .providers import Lima, Provider, RemoteHost, SSH, Tart, cleanup
from .scenarios import Scenarios
from .tls import TLS


CORE = ["pairing", "grants", "files", "git-mutation", "transfers", "history-transfer", "acp", "resume",
        "permission-allow", "permission-deny", "cancel", "concurrent", "terminal", "pairing-expiry"]
TLS_CHECKS = ["tls-setup", "tls-positive", "tls-untrusted", "tls-wrong-name", "tls-expired", "tls-upgrade-rejected"]
RESILIENCE = ["network", "uncertain-write", "acp-reset", "service-restart", "service-crash", "reboot", "namespace-failure"]
PROFILES = {
    "linux-core": CORE,
    "linux-ssh": ["bootstrap", "ssh-host-key", "ssh-bad-key", "ssh-port-conflict"],
    "linux-tls": TLS_CHECKS,
    "linux-ui": ["ui"],
    "linux-resilience": ["acp", *RESILIENCE],
    "linux-keyring": ["keyring"],
    "linux-install": ["installer"],
    "linux-upgrade": ["upgrade"],
    "linux-full": [*CORE, "bootstrap", "ssh-host-key", "ssh-bad-key", "ssh-port-conflict",
                   *TLS_CHECKS, "ui", *RESILIENCE, "keyring", "installer", "upgrade"],
    "macos-full": [*CORE, "bootstrap", *TLS_CHECKS, "ui", "network", "acp-reset",
                   "service-restart", "service-crash", "reboot", "keyring", "installer", "upgrade"],
    "remote-host": [*CORE, "public-endpoint", "service-restart"],
}
# Full server/transport acceptance without the OS-login-only native UI step.
# This remains explicit and manual; it is not the default profile.
PROFILES["linux-headless"] = [name for name in PROFILES["linux-full"] if name != "ui"]

class Prepared(Exception):
    """Finish preparation through the same evidence/exit-status finalizer."""


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    sub = cli.add_subparsers(dest="command", required=True)
    listing = sub.add_parser("list", help="List manual profiles without starting anything")
    listing.add_argument("--json", action="store_true")
    for name in ("doctor", "prepare", "run"):
        command_parser = sub.add_parser(name)
        command_parser.add_argument("--profile", choices=PROFILES, default="linux-core")
        command_parser.add_argument("--provider", choices=("lima", "tart", "ssh-host"))
        command_parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/dunlin")
        command_parser.add_argument("--build", action="store_true", help="Explicitly build the host debug client")
        command_parser.add_argument("--opencode", default=shutil.which("opencode") or "opencode")
        command_parser.add_argument("--opencode-version", default="2.2.6")
        command_parser.add_argument("--rust-version", default="1.99.1")
        command_parser.add_argument("--model", help="Anonymous opencode/*-free ID present in an empty-home catalog")
        command_parser.add_argument("--output-root", type=Path)
        command_parser.add_argument("--run-directory", type=Path, help="Explicitly reuse an owned prepared environment")
        command_parser.add_argument("--vm-template", type=Path)
        command_parser.add_argument("--host-config", type=Path)
        command_parser.add_argument("--relay-artifact", type=Path)
        command_parser.add_argument("--relay-sha256")
        command_parser.add_argument("--previous-relay-artifact", type=Path)
        command_parser.add_argument("--previous-relay-sha256")
        command_parser.add_argument("--release-version")
        command_parser.add_argument("--published-installer", action="store_true",
                                    help="Use immutable published harness assets instead of the candidate HTTPS fixture")
        command_parser.add_argument("--endpoint", help="Real HTTPS endpoint forwarding to this disposable relay")
        command_parser.add_argument("--allow-keychain", action="store_true")
        command_parser.add_argument("--allow-reboot", action="store_true")
        command_parser.add_argument("--allow-system-changes", action="store_true")
        command_parser.add_argument("--keep-on-failure", action="store_true")
        command_parser.add_argument("--keep-environment", action="store_true", help="Retain owned VM for an explicit later run")
        command_parser.add_argument("--only", action="append", help="Select named checks from the profile; repeatable")
    for name in ("inspect", "cleanup"):
        inspect = sub.add_parser(name)
        inspect.add_argument("--run-directory", type=Path, required=True)
    return cli


def doctor(options):
    provider = options.provider or ("tart" if options.profile == "macos-full" else
                                    "ssh-host" if options.profile == "remote-host" else "lima")
    issues = []
    for executable in (["limactl"] if provider == "lima" else ["tart"] if provider == "tart" else []) + ["ssh", "openssl", "git"]:
        if not shutil.which(executable):
            issues.append(f"Missing executable: {executable}")
    if not shutil.which(options.opencode):
        issues.append("OpenCode V2 must be installed locally for isolated free-model discovery")
    if provider in ("tart", "ssh-host") and not options.host_config:
        issues.append("This provider requires --host-config (no host, key, or image is inferred)")
    if not options.binary.is_file() and not options.build:
        issues.append("Build the debug client or select --build")
    return {"provider": provider, "issues": issues, "manual_only": True}


def attach(run, options):
    context = json.loads((run.directory / "context.json").read_text())
    require(context["provider"] == options.provider, "Prepared provider does not match requested provider")
    require(Path(context["root"]).name == run.id, "Prepared root ownership mismatch")
    provider = Provider(run, options)
    provider.ssh = SSH(context["ssh_config"], context["target"])
    for field in ("root", "home", "platform", "arch", "binary", "db", "tls_port"):
        if field in context:
            setattr(provider, field, context[field])
    from .providers import quoted
    provider.ssh.exec(f"test \"$(cat {quoted(provider.root + '/.owner')})\" = {quoted(run.id)}")
    provider.relay = lambda *args: provider.ssh.text(" ".join(quoted(a) for a in [provider.binary, *args, "--db", provider.db]))
    model = context["model"]
    if options.model:
        catalog = free_models(options.opencode, run.directory / "catalog-home")
        require(options.model in catalog, "Requested model is not available in the anonymous free-model catalog")
        model = options.model
        run.report["facts"]["anonymous_catalog"] = catalog
    run.report["facts"]["model"] = model
    return provider, model


def prepare(run, options):
    catalog = free_models(options.opencode, run.directory / "catalog-home")
    model = options.model or next((m for m in catalog if "deepseek" in m and "flash" in m), catalog[0])
    require(model in catalog, "Requested model is not available in the anonymous free-model catalog")
    run.report["facts"].update(model=model, anonymous_catalog=catalog)
    provider = {"lima": Lima, "tart": Tart, "ssh-host": RemoteHost}[options.provider](run, options)
    if not run.check("environment.create", provider.prepare):
        return provider, model, False
    if not run.check("environment.deploy", lambda: deploy(provider, model)):
        return provider, model, False
    context = {field: getattr(provider, field) for field in ("root", "home", "platform", "arch", "binary", "db")}
    context.update(provider=options.provider, ssh_config=str(provider.ssh.config), target=provider.ssh.target, model=model)
    if hasattr(provider, "tls_port"):
        context["tls_port"] = provider.tls_port
    private_json(run.directory / "context.json", context)
    return provider, model, True


def native_cleanup(run, binary):
    # Only delete freshly generated IDs recorded before the production Keychain
    # write. No token values are passed through the security command's argv.
    import re
    probe = run.directory / "keychain-probe.json"
    if probe.is_file():
        host = json.loads(probe.read_text())["id"]
        require(host == run.directory.name and host.startswith("dunlin-remote-e2e-"), "Foreign Keychain probe")
        result = command(["/usr/bin/security", "delete-generic-password", "-s", "dev.dunlin.remote", "-a", host], check=False, timeout=15)
        require(result.returncode in (0, 44), "Cannot clean owned Keychain probe")
    for journal in run.directory.glob("remote-ui-*/keychain-ids"):
        require(journal.resolve().is_relative_to(run.directory.resolve()), "Foreign Keychain journal")
        for host in set(journal.read_text().splitlines()):
            require(re.fullmatch(r"host-[0-9a-f-]{36}", host), "Malformed owned Keychain id")
            result = command(["/usr/bin/security", "delete-generic-password", "-s", "dev.dunlin.remote", "-a", host], check=False)
            require(result.returncode in (0, 44), "Cannot remove fixture Keychain item")


def cleanup_environment(run):
    # A reused report may still say the previous attempt retained its VM.
    # Do not carry that claim into a successful final cleanup.
    run.report["environment_retained"] = True
    cleanup(run.directory)
    run.ownership = validate_run(run.directory)
    run.report["environment_retained"] = False


def main(argv=None):
    options = parser().parse_args(argv)
    if options.command == "list":
        print(json.dumps(PROFILES, indent=2) if options.json else "\n".join(
            f"{name}: {', '.join(checks)}" for name, checks in PROFILES.items()))
        return 0
    if options.command in ("inspect", "cleanup"):
        validate_run(options.run_directory)
        if options.command == "inspect":
            print((options.run_directory / "report.json").read_text())
        else:
            with lock_run(options.run_directory):
                cleanup(options.run_directory)
                # Read only this run's recorded IDs, never enumerate user's Keychain.
                run = Run.__new__(Run)
                run.directory = options.run_directory
                native_cleanup(run, None)
                print("Owned environment cleaned; private evidence retained")
        return 0
    diagnostics = doctor(options)
    options.provider = diagnostics["provider"]
    if options.command == "doctor":
        print(json.dumps(diagnostics, indent=2))
        return int(bool(diagnostics["issues"]))
    selected = options.only or PROFILES[options.profile]
    require(set(selected) <= set(PROFILES[options.profile]), "--only contains a check outside the selected profile")
    lock = lock_run(options.run_directory) if options.run_directory else None
    run = Run.load(options.run_directory, options.profile) if options.run_directory else Run(options.profile, options.output_root)
    if lock is None:
        lock = lock_run(run.directory)
    print(f"Run directory: {run.directory}", flush=True)
    run.report["selected"] = selected
    provider, tls = None, None
    keep = options.command == "prepare" or options.keep_environment
    try:
        def preflight():
            if diagnostics["issues"]:
                raise Blocked("; ".join(diagnostics["issues"]))
            if options.build:
                env = os.environ.copy()
                rustup = Path("/opt/homebrew/opt/rustup/bin")
                if rustup.is_dir():
                    env["PATH"] = str(rustup) + os.pathsep + env.get("PATH", "")
                result = command(["cargo", "build", "-p", "dunlin", "--locked"], env=env, cwd=ROOT, timeout=1800)
                run.log("host-build", (result.stdout + result.stderr).decode(errors="replace"))
            run.report["facts"].update(
                source_commit=command(["git", "rev-parse", "HEAD"], cwd=ROOT).stdout.decode().strip(),
                source_dirty=bool(command(["git", "status", "--porcelain"], cwd=ROOT).stdout),
                client_sha256=sha256(options.binary))
            registry = command([options.binary, "--list-native-smokes"]).stdout.decode()
            require("remote-e2e" not in registry, "Manual remote checks leaked into automatic native discovery")
        if not run.check("preflight", preflight):
            raise Blocked("Preflight failed")
        if options.run_directory:
            provider, model = attach(run, options)
        else:
            provider, model, prepared = prepare(run, options)
            if not prepared:
                raise Blocked("Server preparation did not complete")
        if options.command == "prepare":
            run.report["passed"] = True
            run.report["environment_retained"] = True
            raise Prepared()
        client = Client(run, options.binary, model)
        scenarios = Scenarios(provider, client)
        if not run.check("connection.initialize", scenarios.initialize):
            raise Blocked("Cannot run scenarios without an authenticated deployed server")
        tls = TLS(scenarios)
        from . import ssh_checks, upgrades
        checks = {
            "pairing": scenarios.pairing, "pairing-expiry": scenarios.expiry, "grants": scenarios.grants,
            "files": scenarios.files, "transfers": scenarios.transfers, "history-transfer": scenarios.history_transfer,
            "git-mutation": scenarios.git_mutation,
            "acp": scenarios.acp, "resume": scenarios.resume,
            "permission-allow": lambda: scenarios.permissions(True), "permission-deny": lambda: scenarios.permissions(False),
            "cancel": scenarios.cancel, "concurrent": scenarios.concurrent, "terminal": scenarios.terminal,
            "bootstrap": scenarios.bootstrap, "network": scenarios.network, "acp-reset": scenarios.acp_reset,
            "uncertain-write": scenarios.uncertain_write,
            "service-restart": scenarios.restart, "service-crash": lambda: scenarios.restart(True),
            "reboot": scenarios.reboot, "namespace-failure": scenarios.namespace_failure,
            "keyring": scenarios.keyring, "ui": scenarios.ui, "installer": scenarios.installer,
            "public-endpoint": scenarios.public_endpoint, "upgrade": lambda: upgrades.run(scenarios),
            "tls-setup": tls.certificates, "tls-positive": tls.positive,
            "tls-upgrade-rejected": tls.rejected_upgrade,
            **{f"tls-{variant}": (lambda variant=variant: tls.negative(variant)) for variant in ("untrusted", "wrong-name", "expired")},
            **{f"ssh-{name}": (lambda name=name: ssh_checks.run(scenarios, name)) for name in ("host-key", "bad-key", "port-conflict")},
        }
        tls_ready = False
        for name in selected:
            if name.startswith("tls-") and name != "tls-setup" and not tls_ready:
                run.check(name, lambda: (_ for _ in ()).throw(Blocked("TLS setup was not successful/selected")))
                continue
            if not name.startswith("tls-"):
                tls.close()
            passed = run.check(name, checks[name])
            if name == "tls-setup":
                tls_ready = passed
        run.report["passed"] = all(check["status"] == "PASS" for check in run.report["checks"])
    except Prepared:
        pass
    except KeyboardInterrupt:
        run.report["interrupted"] = True
    except Exception as error:
        run.check("execution", lambda: (_ for _ in ()).throw(error))
        completed = {check["name"] for check in run.report["checks"]}
        for name in selected:
            if name not in completed:
                run.check(name, lambda: (_ for _ in ()).throw(Blocked("A prerequisite failed")))
    finally:
        if tls:
            tls.close()
        if provider and provider.ssh:
            run.check("evidence.collect", provider.collect)
        for process in reversed(run.processes):
            stop(process)
        run.check("keychain.cleanup", lambda: native_cleanup(run, options.binary))
        keep = keep or (options.keep_on_failure and not run.report["passed"])
        if not keep:
            run.check("environment.cleanup", lambda: cleanup_environment(run))
        else:
            run.report["environment_retained"] = True
        run.report["passed"] = bool(run.report["passed"]) and all(c["status"] == "PASS" for c in run.report["checks"])
        run.flush()
        print(f"Report: {run.directory / 'report.json'}", flush=True)
        lock.close()
    return int(not run.report["passed"])
