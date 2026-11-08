import copy
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
                 "status": "completed", "conclusion": "success"} for i, n in enumerate(source.RELEASE_CHECKS)]
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

    def authorization_fixture(self):
        sha, head, tree = "a" * 40, "b" * 40, "c" * 40
        base = "repos/owner/repo"
        return {
            f"{base}/commits/{sha}": {
                "sha": sha, "commit": {"verification": {"verified": True}, "tree": {"sha": tree}}},
            f"{base}/compare/main...{sha}": {"status": "identical"},
            f"{base}/commits/{sha}/pulls?per_page=100": [{
                "number": 24, "merged_at": "fixture", "merge_commit_sha": sha,
                "base": {"ref": "main", "repo": {"full_name": "owner/repo"}}, "head": {"sha": head}}],
            f"{base}/commits/{head}": {"sha": head, "commit": {"tree": {"sha": tree}}},
            f"{base}/commits/{head}/check-runs?filter=latest&per_page=100": self.fixture(),
            f"{base}/code-scanning/alerts?state=open&ref=refs/heads/main&per_page=100": [],
        }

    def test_authorization_reuses_pr_checks_without_requesting_post_merge_checks(self):
        fixture = self.authorization_fixture()
        calls = []

        def api(path):
            calls.append(path)
            return fixture[path]

        result = source.authorize_source("owner/repo", "a" * 40, api)
        self.assertTrue(result["authorized"])
        self.assertEqual(result["reviewed_tree"], "c" * 40)
        self.assertEqual(result["required_checks"], sorted(source.RELEASE_CHECKS))
        self.assertEqual(set(calls), set(fixture))

    def test_authorization_rejects_mismatched_or_missing_reviewed_trees(self):
        head_path = f"repos/owner/repo/commits/{'b' * 40}"
        commit_path = f"repos/owner/repo/commits/{'a' * 40}"
        for path, replacement in [
            (head_path, {"sha": "b" * 40, "commit": {"tree": {"sha": "d" * 40}}}),
            (head_path, {"sha": "d" * 40, "commit": {"tree": {"sha": "c" * 40}}}),
            (head_path, {"sha": "b" * 40}),
            (commit_path, {"sha": "a" * 40, "commit": {"verification": {"verified": True}}}),
        ]:
            with self.subTest(path=path, replacement=replacement):
                fixture = self.authorization_fixture()
                fixture[path] = replacement
                with self.assertRaisesRegex(ValueError, "tree does not match"):
                    source.authorize_source("owner/repo", "a" * 40, fixture.__getitem__)

    def test_authorization_requires_all_pr_checks_including_both_linux_architectures(self):
        original = self.authorization_fixture()
        path = f"repos/owner/repo/commits/{'b' * 40}/check-runs?filter=latest&per_page=100"
        for name in source.RELEASE_CHECKS:
            for conclusion in ["failure", "cancelled", "skipped", None]:
                with self.subTest(name=name, conclusion=conclusion):
                    fixture = copy.deepcopy(original)
                    run = next(r for r in fixture[path]["check_runs"] if r["name"] == name)
                    run["conclusion"] = conclusion
                    with self.assertRaisesRegex(ValueError, "not green"):
                        source.authorize_source("owner/repo", "a" * 40, fixture.__getitem__)

    def test_missing_required_checks_cannot_authorize_source(self):
        for name in source.RELEASE_CHECKS:
            with self.subTest(name=name):
                fixture = self.fixture()
                fixture["check_runs"] = [r for r in fixture["check_runs"] if r["name"] != name]
                fixture["total_count"] -= 1
                with self.assertRaisesRegex(ValueError, "not green"):
                    source.trusted_checks("owner/repo", fixture)
