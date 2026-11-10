import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify_vendor as vendor


class VendorIntegrityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.directory = self.root / "vendor/fixture-1.0"
        self.directory.mkdir(parents=True)
        (self.directory / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="1.0"\n')
        (self.directory / "source.rs").write_text("// Reviewed\n")
        (self.root / "Cargo.toml").write_text('[patch.crates-io]\nfixture={path="vendor/fixture-1.0"}\n')
        (self.root / "Cargo.lock").write_text('[[package]]\nname="fixture"\nversion="1.0"\n')
        digest, count = vendor.tree_digest(self.directory)
        (self.root / "vendor/provenance.json").write_text(json.dumps({"schema": 1, "packages": [
            {"name": "fixture", "version": "1.0", "tree_sha256": digest, "files": count}]}))

    def test_reviewed_source_passes_but_modified_or_added_source_does_not(self):
        self.assertEqual(vendor.verify(self.root), 1)
        (self.directory / "source.rs").write_text("// Changed\n")
        with self.assertRaises(ValueError):
            vendor.verify(self.root)
        (self.directory / "source.rs").write_text("// Reviewed\n")
        (self.directory / "extra.rs").write_text("// Unreviewed\n")
        with self.assertRaises(ValueError):
            vendor.verify(self.root)

    def test_obsolete_dependency_cannot_return_even_if_the_current_target_skips_it(self):
        with (self.root / "Cargo.lock").open("a") as lock:
            lock.write('[[package]]\nname="ttf-parser"\nversion="0.25.1"\n')
        with self.assertRaisesRegex(ValueError, "Obsolete dependencies returned"):
            vendor.verify(self.root)

    def test_unused_reviewed_patches_are_rejected(self):
        with (self.root / "Cargo.lock").open("a") as lock:
            lock.write('[[patch.unused]]\nname="fixture"\nversion="1.0"\n')
        with self.assertRaisesRegex(ValueError, "Unused dependency patches"):
            vendor.verify(self.root)

    def test_tree_hash_does_not_hide_incorrect_per_file_provenance(self):
        path = self.root / "vendor/provenance.json"
        records = json.loads(path.read_text())
        record = {"path": "source.rs", "upstream_sha256": None,
                  "patched_sha256": vendor.hashlib.sha256((self.directory / "source.rs").read_bytes()).hexdigest()}
        records["packages"][0]["changes"] = [record]
        path.write_text(json.dumps(records))
        self.assertEqual(vendor.verify(self.root), 1)
        record["patched_sha256"] = "0" * 64
        path.write_text(json.dumps(records))
        with self.assertRaisesRegex(ValueError, "per-file provenance mismatch"):
            vendor.verify(self.root)

    def test_duplicate_and_escaping_change_paths_are_rejected(self):
        path = self.root / "vendor/provenance.json"
        records = json.loads(path.read_text())
        record = {"path": "source.rs", "patched_sha256": vendor.hashlib.sha256((self.directory / "source.rs").read_bytes()).hexdigest()}
        for changes in [[record, record], [{**record, "path": "../outside"}], [{**record, "path": str(self.directory / "source.rs")}]]:
            records["packages"][0]["changes"] = changes
            path.write_text(json.dumps(records))
            with self.assertRaisesRegex(ValueError, "Invalid or duplicate vendor change path"):
                vendor.verify(self.root)

    def test_symlinks_and_unregistered_patches_fail_closed(self):
        (self.directory / "link").symlink_to("source.rs")
        with self.assertRaisesRegex(ValueError, "Symlink"):
            vendor.verify(self.root)
        (self.directory / "link").unlink()
        with (self.root / "Cargo.toml").open("a") as manifest:
            manifest.write('other={path="../outside"}\n')
        with self.assertRaises(ValueError):
            vendor.verify(self.root)


if __name__ == "__main__":
    unittest.main()
