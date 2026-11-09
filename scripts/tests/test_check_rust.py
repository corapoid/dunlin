"""The quality runner must report all checks without changing source files."""
import contextlib
import io
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_rust


class RustQualityTests(unittest.TestCase):
    def run_checks(self, outcomes):
        output = io.StringIO()
        with patch.object(check_rust.subprocess, "run", side_effect=outcomes) as run:
            with contextlib.redirect_stdout(output):
                code = check_rust.run_checks()
        return code, run, output.getvalue()

    def test_success_runs_every_check_from_repository_root(self):
        outcomes = [subprocess.CompletedProcess(command, 0) for _, command in check_rust.CHECKS]
        code, run, output = self.run_checks(outcomes)
        self.assertEqual(code, 0)
        self.assertEqual(run.call_count, 4)
        for call, (_, command) in zip(run.call_args_list, check_rust.CHECKS):
            self.assertEqual(call.args, (command,))
            self.assertEqual(call.kwargs, {"cwd": check_rust.ROOT, "check": False})
            self.assertNotIn("--fix", command)
        self.assertEqual(output.count("PASS:"), 4)

    def test_failures_and_missing_tools_do_not_skip_later_checks(self):
        outcomes = [subprocess.CompletedProcess((), 1), FileNotFoundError("missing cargo"),
                    subprocess.CompletedProcess((), 0), subprocess.CompletedProcess((), 2)]
        code, run, output = self.run_checks(outcomes)
        self.assertEqual(code, 1)
        self.assertEqual(run.call_count, 4)
        for expected in ["FAIL (exit 1): Format", "FAIL (exit 127): Compiler",
                         "PASS: Clippy", "FAIL (exit 2): Unused dependencies"]:
            self.assertIn(expected, output)

    def test_ci_keeps_the_same_pinned_dependency_check(self):
        ci = (check_rust.ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("cargo install cargo-machete --version 0.9.2 --locked", ci)
        self.assertIn(" ".join(check_rust.CHECKS[-1][1]), ci)


if __name__ == "__main__":
    unittest.main()
