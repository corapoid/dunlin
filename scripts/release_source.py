#!/usr/bin/env python3
"""Authorize releases from a verified mainline squash merge and trusted green CI.

Uses read-only contents/check/security APIs, not an owner/admin credential. The
separate repository-policy audit verifies settings; this verifies the actual
source history, completed PR checks and outstanding security alerts.
"""
import argparse
import json
import re
import subprocess
from repository_policy import CHECKS, GITHUB_ACTIONS_APP


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


def trusted_checks(repo, response, required=CHECKS):
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
        base = f"repos/{args.repo}"
        commit = api(f"{base}/commits/{args.commit}")
        comparison = api(f"{base}/compare/main...{args.commit}")
        pulls = api(f"{base}/commits/{args.commit}/pulls?per_page=100")
        if len(pulls) >= 100:
            raise ValueError("PR association list might be truncated")
        pull = merged_source(args.repo, args.commit, commit, comparison, pulls)
        trusted_checks(args.repo, api(f"{base}/commits/{pull['head']['sha']}/check-runs?filter=latest&per_page=100"))
        # Squash produces a new signed source identity; require its own push CI
        # (including provenance) and security scans as well as the reviewed PR.
        trusted_checks(args.repo, api(f"{base}/commits/{args.commit}/check-runs?filter=latest&per_page=100"), CHECKS - {"dependency-review"})
        clean_alerts(api(f"{base}/code-scanning/alerts?state=open&ref=refs/heads/main&per_page=100"))
        print(json.dumps({"authorized": True, "source": args.commit, "merged_pr": pull["number"],
                          "verified_signature": True, "required_checks": sorted(CHECKS),
                          "open_security_alerts": 0}, indent=2))
    except (ValueError, OSError, KeyError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"Release source authorization failed: {error}\n")


if __name__ == "__main__":
    main()
