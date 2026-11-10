import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
HELPERS = ("dunlin-knowledge-mcp", "dunlin-relay")


class BundleMacosTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        for name in ("bundle-macos.sh", "release_gate.py"):
            shutil.copy(ROOT / "scripts" / name, self.root / "scripts" / name)
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.0"\n')
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.md", "licenses/Apache-2.0.txt",
                     "assets/fonts/OFL.txt", "assets/fonts/Nerd-Fonts-LICENSE.txt",
                     "assets/Simple-Icons-LICENSE.md", "assets/harnesses/LICENSE.md"):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture\n")
        for profile in ("debug", "release"):
            for binary in ("dunlin", *HELPERS):
                self.executable(f"target/{profile}/{binary}",
                                f"#!/bin/sh\necho {profile}-{binary}\n")
        # Stub native tools and Cargo: tests must not compile, launch the GUI,
        # sign a real bundle, or depend on macOS being the test host.
        self.executable("bin/cargo", f'#!{sys.executable}\n'
                        'import json, os, sys\n'
                        'from pathlib import Path\n'
                        'Path(os.environ["CARGO_LOG"]).write_text(json.dumps(sys.argv[1:]))\n'
                        'sys.exit(int(os.environ.get("CARGO_EXIT", "0")))\n')
        for tool in ("codesign", "qlmanage", "iconutil"):
            self.executable(f"bin/{tool}", "#!/bin/sh\nexit 0\n")
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("DUNLIN_")}
        self.env.update(PATH=f'{self.root / "bin"}{os.pathsep}{os.environ["PATH"]}',
                        CARGO_LOG=str(self.root / "cargo.json"))

    def executable(self, name, contents):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)
        path.chmod(0o755)

    def bundle(self, *args, success=True):
        result = subprocess.run(["/bin/bash", str(self.root / "scripts/bundle-macos.sh"), *args],
                                env=self.env, capture_output=True, text=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def assert_profile(self, profile):
        macos = self.root / "dist/Dunlin.app/Contents/MacOS"
        self.assertIn(f"echo {profile}-dunlin", (macos / "Dunlin").read_text())
        for helper in HELPERS:
            self.assertIn(f"echo {profile}-{helper}", (macos / helper).read_text())
        notices = self.root / "dist/Dunlin.app/Contents/Resources/Syntax-Notices.md"
        self.assertEqual(notices.read_text().strip(), f"{profile}-dunlin")

    def test_dev_build_uses_debug_app_and_helpers_even_with_release_present(self):
        self.bundle("--dev")
        args = json.loads((self.root / "cargo.json").read_text())
        self.assertEqual(args, ["build", "-p", "dunlin", "-p", "dunlin-knowledge-mcp",
                                "-p", "dunlin-relay", "--locked"])
        self.assert_profile("debug")

    def test_build_preserves_release_profile(self):
        self.bundle("--build")
        args = json.loads((self.root / "cargo.json").read_text())
        self.assertEqual(args[:2], ["build", "--release"])
        self.assert_profile("release")

    def test_no_flags_packages_release_without_building(self):
        self.bundle()
        self.assertFalse((self.root / "cargo.json").exists())
        self.assert_profile("release")

    def test_debug_fallback_does_not_mix_in_release_helpers(self):
        (self.root / "target/release/dunlin").unlink()
        self.bundle()
        self.assert_profile("debug")

    def test_dev_and_build_flags_are_order_independent(self):
        for args in (("--build", "--dev"), ("--dev", "--build")):
            with self.subTest(args=args):
                self.bundle(*args)
                self.assertNotIn("--release", json.loads((self.root / "cargo.json").read_text()))
                self.assert_profile("debug")

    def test_explicit_binary_uses_sibling_helpers(self):
        self.env["DUNLIN_BIN"] = str(self.root / "target/debug/dunlin")
        self.bundle()
        self.assert_profile("debug")

    def test_failed_build_leaves_existing_bundle_untouched(self):
        marker = self.root / "dist/Dunlin.app/keep"
        marker.parent.mkdir(parents=True)
        marker.write_text("existing bundle")
        self.env["CARGO_EXIT"] = "1"
        self.bundle("--dev", success=False)
        self.assertEqual(marker.read_text(), "existing bundle")

    def test_unknown_option_is_rejected_before_build_or_packaging(self):
        result = self.bundle("--deev", success=False)
        self.assertIn("Unknown argument", result.stderr)
        self.assertFalse((self.root / "cargo.json").exists())
        self.assertFalse((self.root / "dist").exists())

    def test_help_does_not_build_or_package(self):
        result = self.bundle("--help")
        self.assertIn("--dev", result.stdout)
        self.assertFalse((self.root / "cargo.json").exists())
        self.assertFalse((self.root / "dist").exists())


if __name__ == "__main__":
    unittest.main()
