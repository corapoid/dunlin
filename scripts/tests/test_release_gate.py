import io
import itertools
import json
import plistlib
from pathlib import Path
import struct
import sys
import tarfile
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_gate as gate
import repository_policy as policy


class ReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.version = "0.1.0-beta.3"

    def metadata(self):
        (self.root / "Cargo.toml").write_text(f'[workspace.package]\nversion="{self.version}"\n')
        (self.root / "Cargo.lock").write_text(f'[[package]]\nname="dunlin"\nversion="{self.version}"\n')
        (self.root / "CHANGELOG.md").write_text(f'## [Unreleased]\n\n## [{self.version}] — 2026-11-01\n\n### Fixed\n\n- Fixture\n\n## [0.1.0-beta.2]\nOld\n')

    def archives(self, bundle_version=None, member=None, cpu=0x0100000C, relay_type=None):
        app = self.root / f"Dunlin-{self.version}-macos-arm64.zip"
        with zipfile.ZipFile(app, "w") as archive:
            archive.writestr("Dunlin.app/Contents/Info.plist", plistlib.dumps({
                "CFBundleShortVersionString": bundle_version or self.version,
                "CFBundleIdentifier": "dev.dunlin.ide", "CFBundleExecutable": "Dunlin",
                "LSMinimumSystemVersion": "15.0"}))
            archive.writestr("Dunlin.app/Contents/MacOS/Dunlin", struct.pack("<II", 0xFEEDFACF, cpu))
            if member:
                archive.writestr(member, "extra")
        with tarfile.open(self.root / f"dunlin-relay-{self.version}-macos-arm64.tar.gz", "w:gz") as archive:
            entry = tarfile.TarInfo("dunlin-relay")
            if relay_type:
                entry.type = relay_type
                entry.linkname = "/outside"
                archive.addfile(entry)
            else:
                data = struct.pack("<II", 0xFEEDFACF, cpu)
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
        (self.root / f"Dunlin-{self.version}-macos-arm64.dmg").write_bytes(b"koly" + b"\0" * 508)
        self.checksums()

    def checksums(self, prefix=""):
        lines = [f"{gate.digest(self.root / name)}  {prefix}{name}\n"
                 for name in sorted(gate.artifact_names(self.version))]
        (self.root / "SHA256SUMS").write_text("".join(lines))

    def test_versions_are_plain_semver_not_shell_or_paths(self):
        for text in ["0.1.0-beta.3", "1.2.3", "1.2.3-rc.1"]:
            self.assertEqual(gate.version(text), text)
        for text in ["", "v1.2.3", "01.2.3", "1.2.3-01", "1.2.3\n", "../1.2.3", "1.2.3;echo test", "$(touch file)", "1.2.3+metadata"]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                gate.version(text)

    def test_notarization_requires_explicit_accepted_status_not_just_successful_submission(self):
        report = self.root / "notary.json"
        for result in [{}, {"status": "Invalid", "id": "job"}, {"status": "In Progress", "id": "job"},
                       {"status": "Accepted"}, []]:
            report.write_text(json.dumps(result))
            with self.subTest(result=result), self.assertRaises(ValueError):
                gate.notary_result(report)
        report.write_text(json.dumps({"status": "Accepted", "id": "job"}))
        self.assertEqual(gate.notary_result(report), "job")

    def test_all_signing_configurations_fail_closed_without_echoing_credentials(self):
        keys = ["APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_TEAM_ID", "APPLE_APP_PASSWORD"]
        for flags in itertools.product([False, True], repeat=5):
            env = {k: "secret-fixture-value" if flag else "" for k, flag in zip(keys, flags)}
            valid = flags[:2] in [(False, False), (True, True)] and (not any(flags[2:]) or (all(flags[2:]) and flags[0]))
            if valid:
                self.assertEqual(gate.signing_configuration(env), (flags[0], all(flags[2:])))
            else:
                with self.assertRaises(ValueError) as error:
                    gate.signing_configuration(env)
                self.assertNotIn("secret-fixture-value", str(error.exception))

    def test_names_cannot_escape_packaging_directory(self):
        for text in ["../App", "-App", "/Applications/App", "App<xml>", "App\n", "App "]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                gate.app_name(text)

    def test_metadata_and_changelog_pin_the_workspace(self):
        self.metadata()
        self.assertIn("Fixture", gate.metadata(self.root, self.version))
        with self.assertRaises(ValueError):
            gate.metadata(self.root, "0.1.0-beta.4")
        (self.root / "Cargo.lock").write_text('[[package]]\nname="dunlin"\nversion="0.0.0"\n')
        with self.assertRaises(ValueError):
            gate.metadata(self.root, self.version)

    def test_empty_duplicate_or_missing_changelog_is_rejected(self):
        self.metadata()
        for text in ["", f"## [{self.version}]\n", f"## [{self.version}]\nbody\n## [{self.version}]\nagain\n"]:
            (self.root / "CHANGELOG.md").write_text(text)
            with self.assertRaises(ValueError):
                gate.changelog(self.root, self.version)

    def test_valid_artifacts_and_legacy_manifest_prefix(self):
        self.archives()
        self.assertEqual(len(gate.verify(self.root, self.version)), 3)
        self.checksums("dist/")
        self.assertEqual(len(gate.verify(self.root, self.version)), 3)

    def test_modified_artifact_is_not_integrity_success(self):
        self.archives()
        (self.root / f"Dunlin-{self.version}-macos-arm64.dmg").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "Checksum"):
            gate.verify(self.root, self.version)

    def test_manifest_cannot_omit_duplicate_or_escape(self):
        self.archives()
        manifest = self.root / "SHA256SUMS"
        original = manifest.read_text()
        for text in ["", original.splitlines()[0], original + original.splitlines()[0],
                     original.replace("  Dunlin-", "  ../Dunlin-"), original.replace("  Dunlin-", "  dist/../Dunlin-")]:
            manifest.write_text(text)
            with self.subTest(text=text), self.assertRaises(ValueError):
                gate.verify(self.root, self.version)

    def test_symlink_artifact_is_rejected(self):
        self.archives()
        path = self.root / f"Dunlin-{self.version}-macos-arm64.dmg"
        path.rename(self.root / "elsewhere")
        path.symlink_to(self.root / "elsewhere")
        with self.assertRaisesRegex(ValueError, "symlinked"):
            gate.verify(self.root, self.version)

    def test_archive_identity_architecture_and_traversal_are_checked_after_hash(self):
        for kwargs in [{"bundle_version": "0.0.0"}, {"cpu": 0x01000007},
                       {"member": "../outside"}, {"member": "/outside"},
                       {"member": "Other.app/payload"}, {"member": "Dunlin.app//alias"},
                       {"relay_type": tarfile.SYMTYPE}]:
            self.archives(**kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                gate.verify(self.root, self.version)

    def test_zip_symlink_is_rejected_without_extracting(self):
        self.archives()
        path = self.root / f"Dunlin-{self.version}-macos-arm64.zip"
        with zipfile.ZipFile(path, "a") as archive:
            entry = zipfile.ZipInfo("Dunlin.app/link")
            entry.external_attr = 0o120777 << 16
            archive.writestr(entry, "/outside")
        self.checksums()
        with self.assertRaisesRegex(ValueError, "symlinked"):
            gate.verify(self.root, self.version)


class RepositoryPolicyTests(unittest.TestCase):
    def fixture(self):
        return [{"enforcement": "active", "target": "branch", "bypass_actors": [],
                 "conditions": {"ref_name": {"include": ["~DEFAULT_BRANCH"], "exclude": []}},
                 "rules": [{"type": k} for k in ["deletion", "non_fast_forward", "required_signatures"]] + [
                     {"type": "pull_request", "parameters": {"required_approving_review_count": 1,
                      "require_code_owner_review": True, "dismiss_stale_reviews_on_push": True,
                      "require_last_push_approval": True, "required_review_thread_resolution": True}},
                     {"type": "required_status_checks", "parameters": {"strict_required_status_checks_policy": True,
                      "required_status_checks": [{"context": c} for c in policy.CHECKS]}}]}]

    def test_complete_policy_passes_without_mutation(self):
        self.assertEqual(policy.audit(self.fixture(), {k: {"status": "enabled"} for k in
                         ["secret_scanning", "secret_scanning_push_protection"]}), [])

    def test_missing_rules_reviews_signatures_or_bypass_are_not_success(self):
        self.assertTrue(policy.audit([], {}))
        fixture = self.fixture()
        fixture[0]["bypass_actors"] = [{"bypass_mode": "always"}]
        fixture[0]["rules"][3]["parameters"]["required_approving_review_count"] = 0
        fixture[0]["rules"] = [r for r in fixture[0]["rules"] if r["type"] != "required_signatures"]
        errors = policy.audit(fixture, {})
        self.assertTrue(any("bypass" in e for e in errors))
        self.assertTrue(any("PRs" in e for e in errors))
        self.assertTrue(any("required_signatures" in e for e in errors))
        self.assertTrue(any("secret_scanning" in e for e in errors))


if __name__ == "__main__":
    unittest.main()
