"""Architecture contracts complement the runner's behavioral subprocess tests."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
from test_ci_workflows import job


class TestQualityWorkflowTests(unittest.TestCase):
    def test_native_suite_is_part_of_required_checks_without_a_headless_skip(self):
        checks = job("ci.yml", "macos-checks")
        self.assertIn("cargo build -p dunlin -p dunlin-agent --bin dunlin --bin mock_acp_agent --locked", checks)
        self.assertIn('python3 scripts/release_qa.py --native-only --tmp-root "$RUNNER_TEMP"', checks)
        self.assertNotIn("continue-on-error", checks)
        self.assertIn("name: native-ui-qa", checks)
        self.assertIn("dunlin-release-qa-*/report.json", checks)
        self.assertIn("dunlin-gui-smoke-*/failure.json", checks)
        self.assertIn("dunlin-gui-smoke-*/project/requests.jsonl", checks)

    def test_coverage_reports_the_desktop_not_just_the_backend(self):
        coverage = job("coverage.yml", "workspace")
        self.assertIn("llvm-tools-preview", coverage)
        self.assertIn("cargo-llvm-cov --version 0.9.1 --locked", coverage)
        self.assertIn("cargo llvm-cov --workspace --locked", coverage)
        self.assertNotIn("--exclude dunlin", coverage)
        self.assertNotIn("--ignore-run-fail", coverage)
        self.assertIn("--require-desktop", coverage)
        self.assertIn("--minimum-lines 39 --minimum-desktop-lines 23", coverage)
        self.assertIn("--branch", coverage)
        self.assertIn("target/coverage/workspace.json", coverage)
        self.assertIn("name: workspace-coverage", coverage)


if __name__ == "__main__":
    unittest.main()
