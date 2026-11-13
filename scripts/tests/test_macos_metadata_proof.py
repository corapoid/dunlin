"""Pure bundle-contract tests; no Docker, sandbox adapter or probe is executed."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("macos_profile_driver_under_test", Path(__file__).parents[1] / "prove_macos_metadata.py")
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


class BundleContractTests(unittest.TestCase):
    def bundle(self, root):
        paths = driver.fixture_paths(root)
        code = driver.shared_probe_programs()
        profiles = []
        for mode in ["read", "write"]:
            # Deliberately not a policy: these tests validate byte and scope
            # contracts only and never hand their fixture string to a kernel.
            raw = "opaque production-byte contract fixture\n"
            profiles.append({"permission": mode, "profile": raw, "profile_sha256": driver.digest(raw.encode()),
                             "cwd": paths["checkout"], "args": [paths["common"], paths["git"],
                             f"{paths['source']}/source-only-secret", f"{paths['other']}/other-project-secret", paths["db"], mode]})
        bundle = {"schema_version": 1, "generator": "production_macos_harness_profile", "authority_platform": "linux-fixture-only",
                  "fixture_root": str(root), "profile_home": paths["home"], "process_home": str(root / "server/agent-homes/usr-fixture"),
                  "program": "/usr/bin/python3", "python_args": ["-I", "-S"], "probe": code["PROBE"], "control": code["CONTROL"], "profiles": profiles}
        return paths, bundle

    def test_exact_shared_code_and_policy_bytes_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            paths, bundle = self.bundle(Path(directory).resolve())
            decoded = driver.validate_bundle(json.dumps(bundle).encode(), paths)
            self.assertEqual(decoded, bundle)
            self.assertEqual(driver.shared_probe_programs()["PROBE"], bundle["probe"])

    def test_code_executable_home_paths_modes_and_policy_hashes_cannot_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            paths, bundle = self.bundle(Path(directory).resolve())
            mutations = [
                lambda value: value.update(program="/bin/sh"),
                lambda value: value.update(process_home="/outside/home"),
                lambda value: value.update(probe="unexpected program"),
                lambda value: value.update(authority_platform="macos-authority"),
                lambda value: value["profiles"][0].update(profile="changed policy bytes"),
                lambda value: value["profiles"][0].update(cwd="/outside"),
                lambda value: value["profiles"][0]["args"].__setitem__(0, "/outside"),
                lambda value: value["profiles"][0].update(permission="write"),
                lambda value: value["profiles"].append(copy.deepcopy(value["profiles"][0])),
            ]
            for mutation in mutations:
                changed = copy.deepcopy(bundle)
                mutation(changed)
                with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                    driver.validate_bundle(json.dumps(changed).encode(), paths)

    def test_oversized_bundle_is_rejected_before_json_parsing(self):
        with self.assertRaises(ValueError):
            driver.validate_bundle(b"x" * (driver.BUNDLE_LIMIT + 1), {})


if __name__ == "__main__":
    unittest.main()
