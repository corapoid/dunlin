"""Run explicitly: python3 -m unittest discover -s scripts/remote_e2e/tests -v.

These infrastructure tests do not invoke a model or start VMs. Deployed-host
checks live in the manual runner, not unittest/cargo discovery or CI.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import types
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from remote_e2e import core, providers, runner
from remote_e2e.proxy import Proxy


class CoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="dunlin-manual-unit-")
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_environment_does_not_inherit_credentials_or_configuration(self):
        with patch.dict(os.environ, {"OPENAI_API_KEY": "private", "OPENCODE_API_KEY": "private",
                                     "OPENCODE_CONFIG_CONTENT": "private", "SSH_AUTH_SOCK": "/private",
                                     "HTTPS_PROXY": "private", "DUNLIN_HARNESS_COMMAND": "private"}):
            env = core.isolated_env(self.root / "home")
        for name in ("OPENAI_API_KEY", "OPENCODE_API_KEY", "OPENCODE_CONFIG_CONTENT", "SSH_AUTH_SOCK",
                     "HTTPS_PROXY", "DUNLIN_HARNESS_COMMAND"):
            self.assertNotIn(name, env)
        self.assertEqual(env["HOME"], str(self.root / "home"))

    def test_only_consented_native_probes_use_login_home_for_keychain(self):
        from remote_e2e.client import Client
        run = core.Run("unit", self.root)
        with patch.dict(os.environ, {"OPENAI_API_KEY": "private", "DUNLIN_REMOTE_TOKEN": "private",
                                     "OPENCODE_CONFIG_CONTENT": "private", "SSH_AUTH_SOCK": "/private"}):
            client = Client(run, self.root / "dunlin", "opencode/test-free")
        with patch("pwd.getpwuid", return_value=types.SimpleNamespace(pw_dir="/fixture/login")):
            for action, consent in (("ui", "keychain_consent"), ("keychain", "consent")):
                with self.assertRaises(AssertionError):
                    client.environment(action, {})
                env = client.environment(action, {consent: True})
                self.assertEqual(env["HOME"], "/fixture/login")
                self.assertEqual(env["XDG_CONFIG_HOME"], str(run.directory / "client-home/.config"))
                for key in ("OPENAI_API_KEY", "DUNLIN_REMOTE_TOKEN", "OPENCODE_CONFIG_CONTENT", "SSH_AUTH_SOCK"):
                    self.assertNotIn(key, env)
                for executable in ("opencode", "claude", "codex", "gh", "glab"):
                    result = core.command([executable, "--version"], env=env, check=False)
                    self.assertEqual(result.returncode, 1)
        self.assertEqual(client.environment("acp", {})["HOME"], str(run.directory / "client-home"))
        self.assertEqual(client.env["HOME"], str(run.directory / "client-home"))

    def test_appkit_completion_requires_all_steps_and_correct_phase(self):
        from remote_e2e.client import ui_completion
        fixture = self.root / "remote-ui-test"
        fixture.mkdir()
        (fixture / "PASS").write_text("manual-remote-e2e")
        (fixture / "first-launch.json").write_text(json.dumps({"phase": "first-launch"}))
        names = ["click remote-e2e-connect", "press cmd-s", "saved remote contents and local canary unchanged",
                 "real anonymous OpenCode reply displayed", "record real GUI session and Keychain persistence evidence"]
        def logs(steps):
            return "\n".join("GUI_STEP: " + json.dumps({"scenario": "manual-remote-e2e", "step": name, "status": status})
                              for name, status in steps)
        steps = [(name, status) for name in names for status in ("started", "passed")]
        self.assertEqual(ui_completion(fixture, logs(steps), False), {"ui_completed": True, "steps": 5})
        with self.assertRaises(AssertionError):
            ui_completion(fixture, logs(steps[:-1]), False)
        with self.assertRaises(AssertionError):
            ui_completion(fixture, logs(steps + [("late failure", "failed")]), False)
        with self.assertRaises(AssertionError):
            ui_completion(fixture, logs(steps), True)
        with self.assertRaises(AssertionError):
            ui_completion(fixture, "", False)
        (fixture / "first-launch.json").write_text(json.dumps({"phase": "restart"}))
        with self.assertRaises(AssertionError):
            ui_completion(fixture, logs(steps), False)

    def test_only_explicit_anonymous_free_models(self):
        for model in ("openai/paid", "opencode/paid", "openrouter/model-free", "opencode/foo-free#paid"):
            with self.assertRaises(AssertionError):
                core.opencode_config(model)
        config = core.opencode_config("opencode/catalog-model-free")
        self.assertEqual(config["model"], "opencode/catalog-model-free")
        self.assertEqual(config["policies"][0]["effect"], "deny")

    def test_catalog_comes_from_empty_home_and_private_server(self):
        result = subprocess.CompletedProcess([], 0, b"opencode/paid\nopencode/a-free\nopenrouter/b-free\n", b"")
        with patch.object(core, "command", return_value=result) as execute:
            models = core.free_models("opencode", self.root / "catalog")
        self.assertEqual(models, ["opencode/a-free"])
        self.assertEqual(execute.call_args.args[0], ["opencode", "--standalone", "models"])
        self.assertEqual(execute.call_args.kwargs["env"]["HOME"], str(self.root / "catalog"))

    def test_unavailable_catalog_is_blocked_not_paid_fallback(self):
        result = subprocess.CompletedProcess([], 0, b"opencode/paid\n", b"")
        with patch.object(core, "command", return_value=result):
            with self.assertRaises(core.Blocked):
                core.free_models("opencode", self.root / "catalog")

    def test_private_files_and_ownership(self):
        run = core.Run("unit", self.root)
        self.assertEqual((run.directory / "ownership.json").stat().st_mode & 0o777, 0o600)
        self.assertEqual(core.validate_run(run.directory)["run_id"], run.id)
        run.ownership["directory"] = str(self.root)
        run.flush()
        with self.assertRaises(AssertionError):
            core.validate_run(run.directory)

    def test_failed_journal_update_preserves_last_complete_manifest(self):
        path = self.root / "journal.json"
        core.private_json(path, {"complete": True})
        with self.assertRaises(TypeError):
            core.private_json(path, {"incomplete": object()})
        self.assertEqual(json.loads(path.read_text()), {"complete": True})
        self.assertEqual(list(self.root.glob(".journal.json.*")), [])

    def test_blocked_fails_junit_and_redacts_secrets(self):
        run = core.Run("unit", self.root)
        run.secrets.append("fixture-secret")
        self.assertFalse(run.check("unavailable", lambda: (_ for _ in ()).throw(core.Blocked("fixture-secret"))))
        self.assertEqual(run.report["checks"][0]["status"], "BLOCKED")
        self.assertNotIn("fixture-secret", (run.directory / "report.json").read_text())
        self.assertIn("<failure", (run.directory / "junit.xml").read_text())

    def test_previous_failures_are_archived_not_erased_on_retry(self):
        run = core.Run("unit", self.root)
        run.report["facts"]["remote_ui"] = {"previous_attempt_only": True}
        run.check("broken", lambda: (_ for _ in ()).throw(ValueError("broken")))
        resumed = core.Run.load(run.directory, "retry")
        self.assertEqual(resumed.report["checks"], [])
        self.assertNotIn("remote_ui", resumed.report["facts"])
        self.assertIn("remote_ui", json.loads((run.directory / "attempt-001.json").read_text())["facts"])
        self.assertEqual(json.loads((run.directory / "attempt-001.json").read_text())["checks"][0]["status"], "FAIL")

    def test_timeout_kills_process(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            core.command([sys.executable, "-c", "import time;time.sleep(60)"], timeout=0.1)

    def test_cleanup_refuses_foreign_vm(self):
        run = core.Run("unit", self.root)
        run.own("lima", name="unrelated-user-vm")
        with patch.object(providers, "command") as execute:
            with self.assertRaises(RuntimeError):
                providers.cleanup(run.directory)
            execute.assert_not_called()

    def test_lima_does_not_claim_a_preexisting_vm(self):
        run = core.Run("unit", self.root)
        provider = providers.Lima(run, types.SimpleNamespace())
        result = subprocess.CompletedProcess([], 0, b'{"name":"already-exists"}', b"")
        with patch("shutil.which", return_value="/limactl"), patch.object(providers, "command", return_value=result):
            with self.assertRaises(AssertionError):
                provider.prepare()
        self.assertEqual(run.ownership["resources"], [])

    def test_tart_does_not_claim_a_preexisting_vm(self):
        run = core.Run("unit", self.root)
        settings = self.root / "host.json"
        settings.write_text(json.dumps({"image": "example.invalid/test@sha256:" + "a" * 64}))
        provider = providers.Tart(run, types.SimpleNamespace(host_config=settings))
        result = subprocess.CompletedProcess([], 0, run.id.encode(), b"")
        with patch("shutil.which", return_value="/tart"), patch.object(providers, "command", return_value=result):
            with self.assertRaises(AssertionError):
                provider.prepare()
        self.assertEqual(run.ownership["resources"], [])

    def test_service_does_not_overwrite_an_unowned_unit(self):
        from remote_e2e.deployment import start_service
        from unittest.mock import Mock
        run = core.Run("unit", self.root)
        ssh = Mock()
        ssh.exec.side_effect = RuntimeError("already exists")
        provider = types.SimpleNamespace(run=run, ssh=ssh, root="/fixture", home="/home/test", platform="Linux")
        with self.assertRaises(RuntimeError):
            start_service(provider)
        self.assertEqual(run.ownership["resources"], [])

    def test_cleanup_is_idempotent(self):
        run = core.Run("unit", self.root)
        providers.cleanup(run.directory)
        providers.cleanup(run.directory)
        self.assertTrue(core.validate_run(run.directory)["cleanup_complete"])

    def test_successful_retry_cleanup_clears_previous_retention_claim(self):
        run = core.Run("unit", self.root)
        run.report["environment_retained"] = True
        runner.cleanup_environment(run)
        self.assertFalse(run.report["environment_retained"])
        self.assertTrue(run.ownership["cleanup_complete"])
        runner.cleanup_environment(run)
        self.assertFalse(run.report["environment_retained"])

    def test_active_run_cannot_be_executed_or_cleaned_concurrently(self):
        run = core.Run("unit", self.root)
        with core.lock_run(run.directory):
            with self.assertRaises(core.Blocked):
                core.lock_run(run.directory)
        with core.lock_run(run.directory):
            pass

    def test_cleanup_refuses_foreign_service_and_directory(self):
        run = core.Run("unit", self.root)
        run.own("remote-directory", path="/home/user", ssh_config="/does-not-exist", target="fixture")
        with patch.object(providers, "command") as execute:
            with self.assertRaises(RuntimeError):
                providers.cleanup(run.directory)
            execute.assert_not_called()

    def test_remote_marker_mismatch_cannot_fall_through_to_delete(self):
        run = core.Run("unit", self.root)
        config = self.root / "fixture.config"
        config.write_text("")
        (run.directory / ".owner").write_text("someone-else")
        run.own("remote-directory", path=str(run.directory), ssh_config=str(config), target="fixture")
        def local_shell(_self, script, **_kwargs):
            return core.command(["/bin/sh", "-c", script])
        with patch.object(providers.SSH, "exec", local_shell):
            with self.assertRaises(RuntimeError):
                providers.cleanup(run.directory)
        self.assertTrue(run.directory.is_dir())
        self.assertTrue((run.directory / "ownership.json").is_file())

    def test_profiles_are_explicit_and_have_no_default_all(self):
        self.assertNotIn("all", runner.PROFILES)
        options = runner.parser().parse_args(["run"])
        self.assertEqual(options.profile, "linux-core")
        self.assertFalse(options.allow_keychain)
        self.assertFalse(options.allow_reboot)
        self.assertFalse(options.allow_system_changes)

    def test_reused_environment_honors_only_explicit_catalog_free_model_override(self):
        from unittest.mock import Mock
        run = core.Run("unit", self.root)
        context = {"provider": "lima", "root": "/fixture/" + run.id, "ssh_config": "/fixture/ssh.config",
                   "target": "fixture", "model": "opencode/original-free"}
        core.private_json(run.directory / "context.json", context)
        options = types.SimpleNamespace(provider="lima", model="opencode/selected-free", opencode="/opencode")
        with patch.object(runner, "SSH", return_value=Mock()), patch.object(runner, "free_models", return_value=[options.model]):
            _, model = runner.attach(run, options)
        self.assertEqual(model, "opencode/selected-free")
        self.assertEqual(run.report["facts"]["model"], model)
        self.assertEqual(json.loads((run.directory / "context.json").read_text())["model"], "opencode/original-free")
        options.model = "opencode/paid"
        with patch.object(runner, "SSH", return_value=Mock()), patch.object(runner, "free_models", return_value=[model]):
            with self.assertRaises(AssertionError):
                runner.attach(run, options)

    def test_no_workflow_or_automatic_runner_integration(self):
        # Static guard against accidentally adding this suite to existing entry
        # points. It does not modify or police unrelated user workflow changes.
        for workflow in (core.ROOT / ".github/workflows").glob("*.yml"):
            self.assertNotIn("remote-e2e.py", workflow.read_text())
        self.assertNotIn("remote_e2e", (core.ROOT / "scripts/release_qa.py").read_text())
        cli = (core.ROOT / "apps/desktop/src/cli.rs").read_text()
        inventory = cli.split("pub(crate) const NATIVE_SMOKES", 1)[1].split("];", 1)[0]
        self.assertNotIn("remote-e2e", inventory)

    def test_host_configuration_requires_explicit_dedicated_host(self):
        run = core.Run("unit", self.root)
        with self.assertRaises(AssertionError):
                providers.configured_ssh(run, {"hostname": "example.invalid"})

    def test_harness_recipe_uses_distinct_private_empty_npm_configs(self):
        binary = self.root / "bin"
        binary.mkdir()
        node = binary / "node"
        node.write_text("#!/bin/sh\nexit 0\n")
        node.chmod(0o700)
        npm = binary / "npm"
        npm.write_text("""#!/bin/sh
set -eu
user=''; global=''
for arg; do
  case "$arg" in --userconfig=*) user=${arg#*=};; --globalconfig=*) global=${arg#*=};; esac
done
test "$user" != "$global"
test -f "$user"; test -f "$global"
test ! -s "$user"; test ! -s "$global"
printf ok > "$RECIPE_LOG"
""")
        npm.chmod(0o700)
        env = core.isolated_env(self.root / "home")
        env.update(PATH=str(binary) + ":/usr/bin:/bin", DUNLIN_TOOLS=str(self.root / "tools"),
                   RECIPE_LOG=str(self.root / "recipe.log"))
        core.command(["/bin/bash", core.ROOT / "scripts/install-harness.sh", "opencode"], env=env)
        self.assertEqual((self.root / "recipe.log").read_text(), "ok")
        self.assertFalse(list((self.root / "home").glob("dunlin-npm-config.*")))

    def test_upgrade_backup_includes_uncheckpointed_wal(self):
        import sqlite3
        from contextlib import closing
        from remote_e2e.upgrades import sqlite_backup_script
        source, destination = self.root / "source.db", self.root / "backup.db"
        connection = sqlite3.connect(source)
        try:
            connection.execute("PRAGMA journal_mode=WAL")
            connection.execute("PRAGMA wal_autocheckpoint=0")
            connection.execute("CREATE TABLE evidence (value TEXT)")
            connection.execute("INSERT INTO evidence VALUES ('committed-only-in-wal')")
            connection.commit()
            self.assertTrue(source.with_name("source.db-wal").is_file())
            core.command(["/bin/sh", "-c", sqlite_backup_script(source, destination)], timeout=10)
            with closing(sqlite3.connect(destination)) as backup:
                self.assertEqual(backup.execute("SELECT value FROM evidence").fetchone()[0], "committed-only-in-wal")
                self.assertEqual(backup.execute("PRAGMA quick_check").fetchone()[0], "ok")
        finally:
            connection.close()


class ProxyTests(unittest.TestCase):
    def setUp(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(4)
        self.listener.settimeout(0.1)
        self.done = threading.Event()
        def serve():
            while not self.done.is_set():
                try:
                    client, _ = self.listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    break
                def echo(stream):
                    with stream:
                        try:
                            while data := stream.recv(16384):
                                stream.sendall(data)
                        except OSError:
                            pass
                threading.Thread(target=echo, args=(client,), daemon=True).start()
        self.thread = threading.Thread(target=serve, daemon=True)
        self.thread.start()
        self.proxy = Proxy(self.listener.getsockname())

    def tearDown(self):
        self.proxy.close()
        self.done.set()
        self.listener.close()
        self.thread.join(timeout=1)

    def test_forward_delay_and_recovery_after_reset(self):
        self.proxy.delay = 0.03
        with socket.create_connection(("127.0.0.1", self.proxy.port), timeout=2) as client:
            before = time.monotonic()
            client.sendall(b"hello")
            self.assertEqual(client.recv(5), b"hello")
            self.assertGreaterEqual(time.monotonic() - before, 0.05)
            self.proxy.reset()
        with socket.create_connection(("127.0.0.1", self.proxy.port), timeout=2) as client:
            client.sendall(b"again")
            self.assertEqual(client.recv(5), b"again")

    def test_blackhole_is_bounded_and_reversible(self):
        self.proxy.blackhole = True
        with socket.create_connection(("127.0.0.1", self.proxy.port), timeout=0.2) as client:
            client.sendall(b"held")
            with self.assertRaises(socket.timeout):
                client.recv(4)
            self.proxy.blackhole = False
            client.settimeout(2)
            self.assertEqual(client.recv(4), b"held")

    def test_websocket_upgrade_can_be_rejected_without_disabling_http(self):
        proxy = Proxy(self.listener.getsockname(), reject_websocket=True)
        try:
            with socket.create_connection(("127.0.0.1", proxy.port), timeout=2) as client:
                client.sendall(b"GET /acp HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\r\n")
                self.assertIn(b"426 Upgrade Required", client.recv(4096))
            self.assertEqual(proxy.request_counts, {"GET": 1})
        finally:
            proxy.close()

    def test_response_loss_does_not_invent_another_request(self):
        proxy = Proxy(self.listener.getsockname(), inspect_http=True)
        proxy.drop_responses = True
        try:
            with socket.create_connection(("127.0.0.1", proxy.port), timeout=0.2) as client:
                client.sendall(b"PUT /file HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n")
                with self.assertRaises(socket.timeout):
                    client.recv(4096)
            self.assertEqual(proxy.request_counts, {"PUT": 1})
        finally:
            proxy.close()


class PkiTests(unittest.TestCase):
    def test_generated_fixtures_have_real_trust_name_and_expiry_failures(self):
        from remote_e2e.tls import TLS
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory(prefix="dunlin-pki-unit-") as directory:
            run = core.Run("pki", directory)
            provider = types.SimpleNamespace(run=run, root="/fixture", ssh=Mock(), platform="Linux",
                                              options=types.SimpleNamespace(provider="lima"), tls_port=12345)
            provider.ssh.target = "fixture"
            provider.ssh.config = Path(directory) / "ssh.config"
            client = types.SimpleNamespace(url="http://127.0.0.1:1", ca=None, request=Mock())
            fixture = TLS(types.SimpleNamespace(p=provider, c=client, run=run))
            with patch.object(fixture, "activate"), patch("remote_e2e.tls.wait_until"):
                fixture.certificates()
            command = ["openssl", "verify", "-CAfile", "ca.pem", "-verify_ip", "127.0.0.1"]
            valid = core.command(command + ["valid.pem"], cwd=fixture.directory, check=False)
            wrong = core.command(command + ["wrong-name.pem"], cwd=fixture.directory, check=False)
            expired = core.command(command + ["expired.pem"], cwd=fixture.directory, check=False)
            self.assertEqual(valid.returncode, 0)
            self.assertNotEqual(wrong.returncode, 0)
            self.assertIn(b"IP address mismatch", wrong.stderr)
            self.assertNotEqual(expired.returncode, 0)
            self.assertIn(b"expired", expired.stderr)


if __name__ == "__main__":
    unittest.main()
