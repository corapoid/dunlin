import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import coverage_summary as coverage


def source(path, count, covered):
    return {"filename": path, "summary": {
        metric: {"count": count, "covered": covered} for metric in ["lines", "functions", "regions"]}}


class CoverageTests(unittest.TestCase):
    def report(self, files):
        return {"type": "llvm.coverage.json.export", "data": [{"files": files}]}

    def test_aggregates_measured_counts_not_averages_of_percentages(self):
        report = self.report([source("/repo/apps/desktop/src/app.rs", 90, 45),
                              source("/repo/apps/desktop/src/main.rs", 10, 10),
                              source("/repo/crates/ssh/src/service.rs", 20, 15)])
        packages, files = coverage.summarize(report, Path("/repo"))
        self.assertEqual(packages["apps/desktop"]["lines"], {"count": 100, "covered": 55})
        text = coverage.markdown(packages, files)
        self.assertIn("55.00% (55/100)", text)
        self.assertIn("not measured", text)
        self.assertEqual(files[0], ("apps/desktop/src/app.rs", 45))

    def test_excludes_vendor_external_sources_and_dedicated_test_fixtures(self):
        files = [source(name, 100, 100) for name in [
            "/repo/vendor/gpui/src/lib.rs", "/external/crates/ssh/src/lib.rs",
            "/repo/apps/desktop/src/save_smoke.rs", "/repo/apps/desktop/src/git_history/smoke.rs",
            "/repo/apps/desktop/src/app/tests.rs", "/repo/crates/ssh/tests/network.rs"]]
        files.append(source("/repo/crates/ssh/src/service.rs", 20, 5))
        packages, included = coverage.summarize(self.report(files), Path("/repo"))
        self.assertEqual(list(packages), ["crates/ssh"])
        self.assertEqual(included, [("crates/ssh/src/service.rs", 15)])

    def test_empty_invalid_and_duplicate_exports_fail_instead_of_claiming_coverage(self):
        file = source("crates/ssh/src/service.rs", 20, 5)
        invalid = copy.deepcopy(file)
        invalid["summary"]["lines"]["covered"] = 21
        for report in [{}, self.report([]), self.report([invalid]), self.report([file, file])]:
            with self.subTest(report=report), self.assertRaises(ValueError):
                coverage.summarize(report, Path("/repo"))

    def test_regression_floors_apply_to_weighted_workspace_and_desktop_separately(self):
        packages, _ = coverage.summarize(self.report([
            source("apps/desktop/src/app.rs", 90, 18), source("crates/ssh/src/service.rs", 10, 10),
        ]), Path("/repo"))
        coverage.check_thresholds(packages, 28, 20)
        for total, desktop in [(29, 20), (28, 21), (-1, 0), (0, 101)]:
            with self.subTest(total=total, desktop=desktop), self.assertRaises(ValueError):
                coverage.check_thresholds(packages, total, desktop)
        del packages["apps/desktop"]
        with self.assertRaises(ValueError):
            coverage.check_thresholds(packages, 0, 1)


if __name__ == "__main__":
    unittest.main()
