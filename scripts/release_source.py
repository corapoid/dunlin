#!/usr/bin/env python3
"""Authorize releases from a verified mainline squash merge and trusted green CI.

Uses read-only contents/check/security APIs, not an owner/admin credential. The
separate repository-policy audit verifies settings; this verifies the actual
source history, identical reviewed source trees, completed PR checks and
outstanding security alerts. CI/security run on the PR, not again after merge.
"""
import argparse
import json
import re
import subprocess
from repository_policy import CHECKS, GITHUB_ACTIONS_APP

RELEASE_CHECKS = CHECKS | {
    "relay-linux (ubuntu-24.04, x86_64-unknown-linux-musl)",
    "relay-linux (ubuntu-24.04-arm, aarch64-unknown-linux-musl)",
}


def merged_source(repo, sha, commit, comparison, pulls):
    if commit.get("sha") != sha or not commit.get("commit", {}).get("verification", {}).get("verified"):
        raise ValueError("Release source is not a GitHub-verified signed commit")
    if comparison.get("status") not in ["identical", "behind"]:
        raise ValueError("Release source is not on main's history")
    matches = [p for p in pulls if p.get("merged_at") and p.get("merge_commit_sha") == sha
               and p.get("base", {}).get("ref") == "main"
               and p.get("base", {}).get("repo", {}).get("full_name") == repo]
    if len(matches) != 1 or not re.fullmatch(r"[a-f0-9]{40}", matches[0].get("head", {}).get("sha", "")):
        raise ValueError("No unique merged mainline PR for this release source")
    return matches[0]


def trusted_checks(repo, response, required=RELEASE_CHECKS):
    runs = response.get("check_runs", [])
    if response.get("total_count") != len(runs):
        raise ValueError("Check-run result is incomplete; do not infer success")
    latest = {}
    for run in runs:
        if (run.get("app", {}).get("id") != GITHUB_ACTIONS_APP
                or not run.get("details_url", "").startswith(f"https://github.com/{repo}/actions/runs/")):
            continue
        name = run.get("name")
        if name not in latest or run.get("id", 0) > latest[name].get("id", 0):
            latest[name] = run
    failed = sorted(name for name in required if name not in latest
                    or latest[name].get("status") != "completed"
                    or latest[name].get("conclusion") != "success")
    if failed:
        raise ValueError("Required trusted PR checks are not green: " + ", ".join(failed))


def clean_alerts(alerts):
    if not isinstance(alerts, list) or len(alerts) >= 100:
        raise ValueError("Security-alert result is malformed or possibly truncated")
    if any(a.get("rule", {}).get("security_severity_level") in ["critical", "high", "medium", "low"] for a in alerts):
        raise ValueError("Open CodeQL security alerts on main require remediation")


def authorize_source(repo, sha, api):
    base = f"repos/{repo}"
    commit = api(f"{base}/commits/{sha}")
    comparison = api(f"{base}/compare/main...{sha}")
    pulls = api(f"{base}/commits/{sha}/pulls?per_page=100")
    if len(pulls) >= 100:
        raise ValueError("PR association list might be truncated")
    pull = merged_source(repo, sha, commit, comparison, pulls)
    head = pull["head"]["sha"]
    reviewed = api(f"{base}/commits/{head}")
    tree = commit.get("commit", {}).get("tree", {}).get("sha", "")
    # Squash changes the commit identity, not the reviewed files. Fail closed if
    # the base moved or the merge introduced anything absent from the green PR.
    if (not re.fullmatch(r"[a-f0-9]{40}", tree) or reviewed.get("sha") != head
            or reviewed.get("commit", {}).get("tree", {}).get("sha") != tree):
        raise ValueError("Release source tree does not match the reviewed PR head")
    trusted_checks(repo, api(f"{base}/commits/{head}/check-runs?filter=latest&per_page=100"))
    clean_alerts(api(f"{base}/code-scanning/alerts?state=open&ref=refs/heads/main&per_page=100"))
    return {"authorized": True, "source": sha, "merged_pr": pull["number"],
            "verified_signature": True, "reviewed_tree": tree,
            "required_checks": sorted(RELEASE_CHECKS), "open_security_alerts": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo) or not re.fullmatch(r"[a-f0-9]{40}", args.commit):
        parser.error("Use an explicit repository and full commit SHA")

    def api(path):
        result = subprocess.run(["gh", "api", path], capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise ValueError("GitHub source/security verification unavailable; no release authorized")
        return json.loads(result.stdout)

    try:
        print(json.dumps(authorize_source(args.repo, args.commit, api), indent=2))
    except (ValueError, OSError, KeyError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"Release source authorization failed: {error}\n")


if __name__ == "__main__":
    main()
