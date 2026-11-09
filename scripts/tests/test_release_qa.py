"""Exercise QA orchestration with disposable processes, never the real app."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_qa as qa


class NativeQaTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.entry = {"flag": "--example-smoke", "fixture": "example", "light_narrow": True, "timeout_secs": 5}

    def discover(self, entries):
        result = subprocess.CompletedProcess([], 0, json.dumps(entries), "")
        with patch.object(qa.subprocess, "run", return_value=result) as run:
            checks = qa.native_checks(Path("/fixture/dunlin"), self.root, {})
        self.assertEqual(run.call_args.args[0], ["/fixture/dunlin", "--list-native-smokes"])
        return checks

    def test_registry_builds_both_variants_with_fresh_absolute_fixtures(self):
        checks = self.discover([self.entry])
        self.assertEqual(len(checks), 2)
        fixtures = []
        for name, command, timeout in checks:
            self.assertEqual(timeout, 5)
            self.assertEqual(command[:2], ["/fixture/dunlin", "--example-smoke"])
            fixture = Path(command[2])
            self.assertEqual(fixture.parent, self.root)
            self.assertTrue(fixture.name.startswith("dunlin-example-smoke-"))
            self.assertFalse(fixture.exists())
            fixtures.append(fixture)
        self.assertNotEqual(*fixtures)
        self.assertEqual(checks[1][1][-1], "--light-narrow")

    def test_controls_can_run_without_a_fixture_argument(self):
        checks = self.discover([dict(self.entry, fixture=None, light_narrow=False)])
        self.assertEqual(checks[0][1], ["/fixture/dunlin", "--example-smoke"])

    def test_malformed_or_duplicate_registry_is_not_a_successful_empty_suite(self):
        for entries in [[], {}, [None], [self.entry, self.entry],
                        [dict(self.entry, fixture="../escape")], [dict(self.entry, flag="../../escape")],
                        [dict(self.entry, timeout_secs=0)], [dict(self.entry, light_narrow="yes")]]:
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                self.discover(entries)

    def test_native_environment_does_not_inherit_account_or_agent_credentials(self):
        parent = dict(os.environ, OPENAI_API_KEY="do-not-forward", SSH_AUTH_SOCK="/real/agent",
                      DUNLIN_AUTOPROMPT="do-not-send", GH_TOKEN="do-not-forward", CODEX_HOME="/real/codex",
                      CLAUDE_CONFIG_DIR="/real/claude", HOME="/real/home")
        env = qa.native_environment(self.root / "sandbox", parent)
        for key in ["OPENAI_API_KEY", "SSH_AUTH_SOCK", "DUNLIN_AUTOPROMPT", "GH_TOKEN"]:
            self.assertNotIn(key, env)
        for key in ["HOME", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "XDG_CONFIG_HOME", "DUNLIN_DATA_DIR"]:
            self.assertTrue(Path(env[key]).is_relative_to(self.root))
        for command in ["gh", "glab"]:
            result = subprocess.run([command, "auth", "status"], env=env, capture_output=True)
            self.assertEqual(result.returncode, 1)
        self.assertTrue(all(Path(p).is_absolute() for p in env["PATH"].split(os.pathsep)))

    def fake_binary(self, success, reject_unknown=True):
        binary = self.root / "fake-dunlin"
        entry = dict(self.entry, light_narrow=False)
        binary.write_text(f"#!{sys.executable}\nimport json, os, sys\n"
                          f"if sys.argv[1] == '--list-native-smokes':\n print({json.dumps(json.dumps([entry]))})\n"
                          f"elif {reject_unknown!r} and sys.argv[1] != '--example-smoke':\n raise SystemExit(2)\n"
                          "else:\n"
                          " assert os.environ['HOME'] != '/real/home'\n"
                          " assert 'OPENAI_API_KEY' not in os.environ\n"
                          f" print({'PASS: fixture completed' if success else 'window closed early'!r})\n")
        binary.chmod(0o700)
        return binary

    def test_real_cli_entrypoint_must_reject_unknown_modes_before_app_initialization(self):
        env = qa.native_environment(self.root / "cli-environment", os.environ)
        qa.verify_native_cli(self.fake_binary(True), self.root, env)
        with self.assertRaises(ValueError):
            qa.verify_native_cli(self.fake_binary(True, reject_unknown=False), self.root, env)
        Path(env["DUNLIN_DATA_DIR"]).mkdir()
        with self.assertRaises(ValueError):
            qa.verify_native_cli(self.fake_binary(True), self.root, env)

    def test_native_only_writes_a_truthful_report_for_real_child_processes(self):
        for success in [True, False]:
            with self.subTest(success=success):
                output = self.root / str(success)
                output.mkdir()
                result = subprocess.run([
                    sys.executable, str(Path(qa.__file__)), '--native-only',
                    '--binary', str(self.fake_binary(success)), '--tmp-root', str(output),
                ], env=dict(os.environ, HOME='/real/home', OPENAI_API_KEY='must-not-forward'),
                    capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0 if success else 1, result.stderr)
                report = json.loads(next(output.glob('*/report.json')).read_text())
                self.assertEqual(report['automatic_passed'], success)
                self.assertTrue(report['native_only'])
                self.assertEqual(len(report['checks']), 1)
                self.assertEqual(report['checks'][0]['exit_code'], 0 if success else 1)

    def test_failed_registry_discovery_is_recorded_as_a_failed_check(self):
        binary = self.root / "missing-binary"
        result = subprocess.run([
            sys.executable, str(Path(qa.__file__)), '--native-only', '--binary', str(binary),
            '--tmp-root', str(self.root),
        ], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        report = json.loads(next(self.root.glob('*/report.json')).read_text())
        self.assertFalse(report['automatic_passed'])
        self.assertEqual(report['checks'][0]['name'], 'native-discovery')

    def test_success_requires_both_exit_zero_and_completion_marker(self):
        for script, expected in [("print('PASS: finished')", 0), ("print('closed early')", 1),
                                 ("print('PASS: not enough'); raise SystemExit(7)", 7)]:
            with self.subTest(script=script):
                code = qa.run_native([sys.executable, "-c", script], self.root / "test.log", os.environ.copy(), 5)
                self.assertEqual(code, expected)

    def test_timed_out_native_process_cannot_pass(self):
        code = qa.run_native([sys.executable, "-c", "import time; time.sleep(60)"],
                             self.root / "timeout.log", os.environ.copy(), 0.05)
        self.assertEqual(code, 124)


if __name__ == "__main__":
    unittest.main()
