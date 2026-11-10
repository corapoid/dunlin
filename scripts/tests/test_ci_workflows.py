"""Offline workflow contracts; actionlint validates the complete YAML separately."""
import itertools
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]


def job(workflow, name):
    text = (ROOT / ".github/workflows" / workflow).read_text()
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:\n|\Z)", text, re.M | re.S)
    if match is None:
        raise AssertionError(f"Missing job {name} in {workflow}")
    return match.group(1)


class WorkflowTests(unittest.TestCase):
    def test_required_test_gate_rejects_failed_cancelled_skipped_or_missing_results(self):
        gate = job("ci.yml", "test")
        self.assertIn("needs: [macos-checks, macos-build, headless]", gate)
        self.assertIn("if: ${{ always() }}", gate)
        self.assertIn("CHECKS_RESULT: ${{ needs.macos-checks.result }}", gate)
        self.assertIn("BUILD_RESULT: ${{ needs.macos-build.result }}", gate)
        self.assertIn("HEADLESS_RESULT: ${{ needs.headless.result }}", gate)
        command = textwrap.dedent(gate.split("        run: |\n", 1)[1])
        results = ["success", "failure", "cancelled", "skipped", ""]
        for checks, build, headless in itertools.product(results, repeat=3):
            with self.subTest(checks=checks, build=build, headless=headless):
                result = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", command],
                    env=dict(os.environ, CHECKS_RESULT=checks, BUILD_RESULT=build, HEADLESS_RESULT=headless),
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode == 0, checks == build == headless == "success")

    def test_macos_checks_and_build_are_independent_and_keep_full_checks(self):
        checks = job("ci.yml", "macos-checks")
        build = job("ci.yml", "macos-build")
        for independent in [checks, build]:
            self.assertNotRegex(independent, r"(?m)^    (needs|if):")
        for command in ["cargo fmt --all --check", "cargo clippy --workspace --all-targets --locked -- -D warnings",
                        "cargo test --workspace --locked", "python3 -m unittest discover -s scripts/tests -v",
                        "python3 scripts/verify_vendor.py", "./scripts/relay-smoke.sh",
                        "python3 scripts/git-operations-smoke.py"]:
            self.assertIn(command, checks)
        self.assertIn("cargo build --release -p dunlin -p dunlin-cli -p dunlin-knowledge-mcp -p dunlin-relay --locked", build)
        self.assertIn("./scripts/bundle-relay.sh --no-build", build)

    def test_headless_checks_are_required_and_publish_separate_cli_artifacts(self):
        headless = job("ci.yml", "headless")
        self.assertIn("cargo test --workspace --exclude dunlin --exclude dunlin-ui --locked", headless)
        self.assertIn("python3 scripts/test_headless.py", headless)
        self.assertIn("python3 scripts/check_architecture.py", job("ci.yml", "macos-checks"))
        self.assertIn("name: dunlin-cli-linux-x86_64", headless)
        self.assertIn("name: dunlin-cli-macos", job("ci.yml", "macos-build"))

    def test_platform_releases_wait_for_authorization_not_each_other(self):
        authorization = job("release.yml", "source-authorization")
        for command in ["python3 scripts/release_source.py", "release_gate.py metadata",
                        "release_gate.py acceptance", "refs/tags/v$VERSION^{commit}"]:
            self.assertIn(command, authorization)
        for name in ["release", "relay-linux"]:
            platform = job("release.yml", name)
            self.assertRegex(platform, r"(?m)^    needs: source-authorization$")
            self.assertIn("${{ needs.source-authorization.outputs.version }}", platform)
        self.assertIn("run: ./scripts/bundle-relay.sh", job("release.yml", "release"))
        self.assertIn("needs: [release, relay-linux]", job("release.yml", "publish"))

    def test_desktop_packages_are_not_generated_or_published(self):
        self.assertFalse((ROOT / ".github/workflows/package.yml").exists())
        self.assertFalse((ROOT / "scripts/publish-oci.sh").exists())
        self.assertFalse((ROOT / "scripts/bundle-pkg.sh").exists())
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            text = path.read_text()
            with self.subTest(workflow=path.name):
                for forbidden in ["./scripts/bundle-macos.sh", "bundle-pkg.sh", "publish-oci.sh",
                                  "packages: write", "name: dunlin-app", "dist/*.app", "dist/*.pkg",
                                  "dist/*.dmg", "dist/*.zip", "hdiutil create"]:
                    self.assertNotIn(forbidden, text)
        release = job("release.yml", "release")
        publish = job("release.yml", "publish")
        self.assertIn('--platform macos --arch arm64', release)
        self.assertIn('--platform macos --arch arm64', publish)
        self.assertIn("shasum -a 256 -c MACOS-RELAY-SHA256SUMS", publish)
        self.assertIn("dist/RELAY-SHA256SUMS", publish)

    def test_installation_docs_recommend_source_without_package_commands(self):
        for name in ["README.md", "docs/getting-started/installation.md", "CONTRIBUTING.md"]:
            text = (ROOT / name).read_text()
            with self.subTest(path=name):
                self.assertIn("beta", text)
                self.assertIn("source", text)
                self.assertNotIn("brew install --cask", text)
                self.assertNotIn("oras pull", text)

    def test_website_does_not_link_to_withdrawn_desktop_packages(self):
        for path in (ROOT / "web/src").rglob("*"):
            if path.suffix not in {".astro", ".ts", ".tsx"}:
                continue
            with self.subTest(path=str(path.relative_to(ROOT))):
                text = path.read_text()
                for forbidden in ["downloadDmg", "downloadPkg", "downloadZip", "site.packages",
                                  "brew install --cask", "oras pull", "Download the beta"]:
                    self.assertNotIn(forbidden, text)

    def test_only_main_warmer_writes_shared_macos_caches_without_running_tests(self):
        for workflow, name, profile in [("ci.yml", "macos-checks", "checks"),
                                        ("ci.yml", "macos-build", "release"),
                                        ("release.yml", "release", "release")]:
            consumer = job(workflow, name)
            self.assertIn(f"shared-key: macos-{profile}-v1", consumer)
            self.assertIn('save-if: "false"', consumer)
        warmer = job("cache.yml", "warm")
        self.assertIn("if: github.ref == 'refs/heads/main'", warmer)
        self.assertIn("shared-key: macos-${{ matrix.profile }}-v1", warmer)
        self.assertIn("persist-credentials: false", warmer)
        test_commands = re.findall(r"^\s*cargo test .+$", warmer, re.M)
        self.assertTrue(test_commands)
        self.assertTrue(all("--no-run" in command for command in test_commands))
        self.assertNotIn("pull_request_target:", (ROOT / ".github/workflows/cache.yml").read_text())


class RelayBundleTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/bundle-relay.sh", self.root / "scripts/bundle-relay.sh")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "cargo.log"
        cargo = self.bin / "cargo"
        cargo.write_text('#!/bin/bash\nprintf "%s\\n" "$*" >> "$CARGO_LOG"\n')
        cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}",
                        CARGO_LOG=str(self.log), DUNLIN_DIST_DIR=str(self.root / "output"))
        self.binary = self.root / "target/release/dunlin-relay"

    def build_fixture(self):
        self.binary.parent.mkdir(parents=True)
        self.binary.write_text("#!/bin/sh\necho relay-fixture\n")
        self.binary.chmod(0o755)

    def bundle(self, *args):
        return subprocess.run(["bash", str(self.root / "scripts/bundle-relay.sh"), *args],
                              env=self.env, capture_output=True, text=True)

    def test_no_build_copies_existing_binary_without_invoking_cargo(self):
        self.build_fixture()
        result = self.bundle("--no-build")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.log.exists())
        output = self.root / "output/dunlin-relay"
        self.assertEqual(output.read_bytes(), self.binary.read_bytes())
        self.assertTrue(os.access(output, os.X_OK))

    def test_default_still_builds_a_standalone_release(self):
        self.build_fixture()
        result = self.bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text(), "build --release -p dunlin-relay --locked\n")

    def test_no_build_rejects_missing_or_non_executable_binary(self):
        for present in [False, True]:
            with self.subTest(present=present):
                if present:
                    self.build_fixture()
                    self.binary.chmod(0o644)
                result = self.bundle("--no-build")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Missing release relay binary", result.stderr)
                self.assertFalse(self.log.exists())
                self.assertFalse((self.root / "output").exists())

    def test_invalid_arguments_fail_before_building(self):
        for args in [("--unknown",), ("--no-build", "extra")]:
            with self.subTest(args=args):
                result = self.bundle(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Usage:", result.stderr)
                self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
