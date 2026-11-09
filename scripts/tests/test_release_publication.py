"""Exercise the actual publication shell with a local GitHub CLI stub."""
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ReleasePublicationTests(unittest.TestCase):
    def test_regular_and_beta_versions_are_published_as_latest_regular_releases(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        publish = workflow.split("      - name: Publish only verified existing tag\n", 1)[1]
        command = textwrap.dedent(publish.split("        run: |\n", 1)[1])
        stub = 'gh() { printf "%s\\0" "$@" > "$GH_ARGS"; }\n'
        for version in ["0.2.0", "0.2.1-beta.1"]:
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                arguments = Path(directory) / "gh-args"
                commit = "a" * 40
                result = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", stub + command],
                    cwd=directory,
                    env=dict(os.environ, VERSION=version, TAG=f"v{version}", APP_NAME="Dunlin",
                             GITHUB_SHA=commit, GH_ARGS=str(arguments)),
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(arguments.read_bytes().decode().split("\0")[:-1], [
                    "release", "create", f"v{version}",
                    f"dist/dunlin-relay-{version}-macos-arm64.tar.gz",
                    f"dist/dunlin-relay-{version}-linux-x86_64.tar.gz",
                    f"dist/dunlin-relay-{version}-linux-arm64.tar.gz",
                    "dist/install-relay.sh", "dist/install-harness.sh", "dist/RELAY-SHA256SUMS",
                    "--title", f"Dunlin {version}", "--notes-file", "dist/release-notes.md",
                    "--verify-tag", "--target", commit, "--prerelease=false", "--latest",
                ])


if __name__ == "__main__":
    unittest.main()
