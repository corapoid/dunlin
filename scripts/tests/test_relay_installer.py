"""Offline contract checks; never install packages or contact a provider."""
import importlib.util
import io
from pathlib import Path
import struct
import tarfile
import tempfile
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("verify_relay_release", ROOT / "scripts/verify-relay-release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class RelayInstallerTests(unittest.TestCase):
    def test_help_and_invalid_harness_have_no_installation_effects(self):
        result = subprocess.run(["bash", str(ROOT / "scripts/install-relay.sh"), "--help"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("not root", result.stdout)
        result = subprocess.run(["bash", str(ROOT / "scripts/install-harness.sh"), "opencode; false"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Unknown harness", result.stderr)

    def test_catalog_is_fixed_and_uses_stable_packages(self):
        script = (ROOT / "scripts/install-harness.sh").read_text()
        for package in ["@opencode/cli@latest", "@agentclientprotocol/codex-acp@latest", "@agentclientprotocol/claude-agent-acp@latest"]:
            self.assertIn(package, script)
        self.assertNotIn("@preview", script)
        self.assertIn('Unknown harness', script)

    def test_bootstrap_confirms_before_mutating_and_checks_downloads(self):
        script = (ROOT / "scripts/install-relay.sh").read_text()
        self.assertLess(script.index("Proceed with these changes?"), script.index("TMP=$(mktemp"))
        self.assertIn("RELAY-SHA256SUMS", script)
        self.assertIn('verify "$TMP/$ASSET"', script)
        self.assertIn('pair admin --ttl 15', script)
        self.assertNotIn('tailscale funnel', script)
        self.assertIn('Existing Tailscale Serve routes found', script)

    def archive(self, directory, machine=62, name="dunlin-relay", symlink=False):
        path = Path(directory) / "relay.tar.gz"
        header = bytearray(64)
        header[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", header, 18, machine)
        with tarfile.open(path, "w:gz") as archive:
            entry = tarfile.TarInfo(name)
            entry.size = len(header)
            if symlink:
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/etc/passwd"
            archive.addfile(entry, io.BytesIO(header))
        return path

    def test_linux_architecture_and_archive_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            release.verify(self.archive(directory), "x86_64")
            for options in [{"machine":183}, {"name":"../dunlin-relay"}, {"symlink":True}]:
                with self.assertRaises(ValueError):
                    release.verify(self.archive(directory, **options), "x86_64")


if __name__ == "__main__":
    unittest.main()
