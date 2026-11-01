#!/usr/bin/env python3
"""Read-only audit of release repository rules. Never applies/weakens rules.

A one-maintainer repository needs an independent reviewer/signing setup before
requiring approval; this tool deliberately does not lock out the owner for them.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path

CHECKS = {"test", "dependency-review", "web", "dependency-audit",
          "codeql (rust)", "codeql (javascript-typescript)", "codeql (python)", "codeql (actions)"}
GITHUB_ACTIONS_APP = 15368  # Verified GitHub.com app identity, not caller-supplied statuses.


def audit(rulesets, security, review_mode="independent"):
    if review_mode not in ["independent", "solo-maintainer"]:
        return ["Unknown review policy"]
    errors = []
    active = [r for r in rulesets if r.get("enforcement") == "active"
              and r.get("target") == "branch"
              and "~DEFAULT_BRANCH" in r.get("conditions", {}).get("ref_name", {}).get("include", [])]
    if not active:
        errors.append("No active default-branch ruleset")
    rules = {}
    for ruleset in active:
        if "bypass_actors" not in ruleset:
            errors.append("Bypass actors are not visible with this credential; policy is unverified")
        if ruleset.get("conditions", {}).get("ref_name", {}).get("exclude"):
            errors.append("Default-branch ruleset contains exclusions; review them explicitly")
        if ruleset.get("bypass_actors"):
            errors.append("Default-branch ruleset has bypass actors")
        for rule in ruleset.get("rules", []):
            rules.setdefault(rule.get("type"), []).append(rule.get("parameters") or {})
    for required in ["deletion", "non_fast_forward", "required_signatures"]:
        if required not in rules:
            errors.append(f"Missing rule: {required}")
    pr = rules.get("pull_request", [])
    def review_matches(p):
        if not p.get("dismiss_stale_reviews_on_push") or not p.get("required_review_thread_resolution"):
            return False
        if review_mode == "solo-maintainer":
            return (p.get("required_approving_review_count") == 0
                    and p.get("require_code_owner_review") is False
                    and p.get("require_last_push_approval") is False
                    and p.get("allowed_merge_methods") == ["squash"])
        return (p.get("required_approving_review_count", 0) >= 1
                and p.get("require_code_owner_review") and p.get("require_last_push_approval"))
    if not any(review_matches(p) for p in pr):
        errors.append(f"PR review rules do not match explicitly configured {review_mode} policy")
    required_checks = rules.get("required_status_checks", [])
    if not any(p.get("strict_required_status_checks_policy") and not p.get("do_not_enforce_on_create")
               and CHECKS <= {c.get("context") for c in p.get("required_status_checks", [])
                              if c.get("integration_id") == GITHUB_ACTIONS_APP}
               for p in required_checks):
        errors.append("Required status checks do not include the complete strict CI/security gate")
    for name in ["secret_scanning", "secret_scanning_push_protection"]:
        if security.get(name, {}).get("status") != "enabled":
            errors.append(f"Not verified enabled: {name}")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo):
        parser.error("Use an explicit owner/repository identity")

    def api(path):
        result = subprocess.run(["gh", "api", path], capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise ValueError("GitHub policy could not be read with current permissions")
        return json.loads(result.stdout)

    try:
        config = json.loads((Path(__file__).resolve().parents[1] / ".github/release-policy.json").read_text())
        if config.get("schema") != 1:
            raise ValueError("Unrecognized repository policy schema")
        index = api(f"repos/{args.repo}/rulesets")
        rulesets = [api(f"repos/{args.repo}/rulesets/{r['id']}") for r in index]
        security = api(f"repos/{args.repo}").get("security_and_analysis") or {}
        mode = config.get("review_mode")
        errors = audit(rulesets, security, mode)
        print(json.dumps({"repository": args.repo, "read_only": True, "passed": not errors,
                          "review_mode": mode, "remaining": errors}, indent=2))
        return int(bool(errors))
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"Policy is unverified: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
