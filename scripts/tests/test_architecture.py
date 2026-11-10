import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("architecture", Path(__file__).resolve().parents[1] / "check_architecture.py")
architecture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(architecture)


class ArchitectureTests(unittest.TestCase):
    def metadata(self, dependencies):
        return {
            "packages": [{"id": "cli", "name": "dunlin-cli"}, {"id": "service", "name": "dunlin-runtime"}, {"id": "ui", "name": "gpui-kit"}],
            "workspace_members": ["cli", "service"],
            "resolve": {"nodes": [{"id": "cli", "dependencies": ["service"]}, {"id": "service", "dependencies": dependencies}, {"id": "ui", "dependencies": []}]},
        }

    def test_rejects_transitive_ui_dependency(self):
        errors = architecture.check(self.metadata(["ui"]))
        self.assertIn("dunlin-cli depends on UI package gpui-kit", errors)
        self.assertIn("dunlin-runtime depends on UI package gpui-kit", errors)

    def test_accepts_ui_free_graph(self):
        self.assertEqual(architecture.check(self.metadata([])), [])
