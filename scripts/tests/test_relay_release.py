"""Offline checks for relay-only distribution on macOS and Linux."""
import importlib.util
import io
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "verify-relay-release.py"
SPEC = importlib.util.spec_from_file_location("verify_relay_release", SCRIPT)
relay = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(relay)


class RelayReleaseTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.archive = Path(temp.name) / "relay.tar.gz"

    def fixture(self, platform="linux", arch="arm64", name="dunlin-relay", extra=False, symlink=False):
        header = bytearray(64)
        if platform == "macos":
            struct.pack_into("<II", header, 0, 0xFEEDFACF,
                             {"arm64": 0x0100000C, "x86_64": 0x01000007}[arch])
        else:
            header[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", header, 18, {"arm64": 183, "x86_64": 62}[arch])
        with tarfile.open(self.archive, "w:gz") as archive:
            entry = tarfile.TarInfo(name)
            if symlink:
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/outside"
                archive.addfile(entry)
            else:
                entry.size = len(header)
                archive.addfile(entry, io.BytesIO(header))
            if extra:
                archive.addfile(tarfile.TarInfo("extra"))

    def test_supported_platforms_and_architectures(self):
        for platform, arch in [("macos", "arm64"), ("linux", "arm64"), ("linux", "x86_64")]:
            with self.subTest(platform=platform, arch=arch):
                self.fixture(platform, arch)
                relay.verify(self.archive, arch, platform)

    def test_linux_remains_the_default_platform(self):
        self.fixture()
        relay.verify(self.archive, "arm64")

    def test_wrong_platform_or_architecture_is_rejected(self):
        for platform, arch in [("linux", "arm64"), ("macos", "x86_64")]:
            self.fixture(platform, arch)
            with self.subTest(platform=platform, arch=arch), self.assertRaises(ValueError):
                relay.verify(self.archive, "arm64", "macos")
        self.fixture("macos", "arm64")
        with self.assertRaises(ValueError):
            relay.verify(self.archive, "arm64", "linux")
        with self.assertRaises(ValueError):
            relay.verify(self.archive, "x86_64", "macos")

    def test_unsafe_or_additional_archive_members_are_rejected(self):
        for kwargs in [{"name": "../dunlin-relay"}, {"name": "/dunlin-relay"},
                       {"extra": True}, {"symlink": True}]:
            self.fixture("macos", **kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                relay.verify(self.archive, "arm64", "macos")

    def test_empty_archive_is_rejected(self):
        with tarfile.open(self.archive, "w:gz"):
            pass
        with self.assertRaises(ValueError):
            relay.verify(self.archive, "arm64", "macos")


if __name__ == "__main__":
    unittest.main()
