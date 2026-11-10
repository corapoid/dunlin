#!/usr/bin/env python3
"""Reject UI dependencies in headless binaries and shared service libraries."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def check(metadata):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node["dependencies"] for node in metadata["resolve"]["nodes"]}
    violations = []
    for package_id in metadata["workspace_members"]:
        package = packages[package_id]
        if package["name"] in {"dunlin", "dunlin-ui"}:
            continue
        seen = set()
        pending = [package_id]
        while pending:
            dependency = pending.pop()
            if dependency in seen:
                continue
            seen.add(dependency)
            name = packages[dependency]["name"]
            if name in {"dunlin", "dunlin-ui"} or name.startswith("gpui"):
                violations.append(f"{package['name']} depends on UI package {name}")
            pending.extend(nodes.get(dependency, []))
    return sorted(set(violations))


def main():
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    violations = check(json.loads(result.stdout))
    if violations:
        raise SystemExit("\n".join(violations))
    print("Architecture boundaries verified: headless packages have no GPUI/desktop dependencies.")


if __name__ == "__main__":
    main()
