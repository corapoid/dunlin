from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_source as source


class ReleaseSourceTests(unittest.TestCase):
    def test_only_verified_mainline_merged_source_is_authorized(self):
        sha = "a" * 40
        commit = {"sha": sha, "commit": {"verification": {"verified": True}}}
        pulls = [{"number": 6, "merged_at": "fixture", "merge_commit_sha": sha,
                  "base": {"ref": "main", "repo": {"full_name": "owner/repo"}}, "head": {"sha": "b" * 40}}]
        self.assertEqual(source.merged_source("owner/repo", sha, commit, {"status": "behind"}, pulls)["number"], 6)
        for comparison in ["ahead", "diverged", "unknown"]:
            with self.assertRaises(ValueError):
                source.merged_source("owner/repo", sha, commit, {"status": comparison}, pulls)
        commit["commit"]["verification"]["verified"] = False
        with self.assertRaises(ValueError):
            source.merged_source("owner/repo", sha, commit, {"status": "identical"}, pulls)

    def fixture(self):
        runs = [{"name": n, "id": i, "app": {"id": source.GITHUB_ACTIONS_APP},
                 "details_url": "https://github.com/owner/repo/actions/runs/1/job/2",
                 "status": "completed", "conclusion": "success"} for i, n in enumerate(source.CHECKS)]
        return {"total_count": len(runs), "check_runs": runs}

    def test_checks_require_expected_app_repository_complete_results_and_success(self):
        source.trusted_checks("owner/repo", self.fixture())
        for change in [{"status": "in_progress"}, {"conclusion": "neutral"}, {"app": {"id": 999}},
                       {"details_url": "https://github.com/attacker/repo/actions/runs/1"}]:
            fixture = self.fixture()
            fixture["check_runs"][0].update(change)
            with self.assertRaises(ValueError):
                source.trusted_checks("owner/repo", fixture)
        fixture = self.fixture()
        fixture["total_count"] += 1
        with self.assertRaises(ValueError):
            source.trusted_checks("owner/repo", fixture)

    def test_latest_failed_retry_cannot_reuse_old_success(self):
        fixture = self.fixture()
        failed = dict(fixture["check_runs"][0], id=1000, conclusion="failure")
        fixture["check_runs"].append(failed)
        fixture["total_count"] += 1
        with self.assertRaises(ValueError):
            source.trusted_checks("owner/repo", fixture)

    def test_security_alerts_are_not_silently_ignored(self):
        source.clean_alerts([])
        for level in ["critical", "high", "medium", "low"]:
            with self.assertRaises(ValueError):
                source.clean_alerts([{"rule": {"security_severity_level": level}}])
        with self.assertRaises(ValueError):
            source.clean_alerts([{}] * 100)
