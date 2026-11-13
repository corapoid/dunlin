#!/usr/bin/env python3
"""Explicit provisioning of pinned policies for the nested bubblewrap test suite.

This does not load kernel policy, build images or run tests. Network access is
only used when --fetch is specified; test execution remains offline.
"""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request

COMMIT = "6049e40058759e8a1673ac1090ec6a7e36b9c9bd"
SOURCES = {
    "seccomp": ("seccomp/default.json", "15434cc9c9037d85b35027a60721bf45f381782b4283b01d94663366e71569f8"),
    "apparmor": ("apparmor/testdata/default.golden", "976f9817805cbc43fa4a367feb57eceba081d3055f5cd5439cea16b6cbc9f203"),
    "license": ("LICENSE", "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30"),
}
NAMESPACE_CALLS = ["clone", "unshare", "setns", "mount", "umount2", "pivot_root"]


def generate(directory, fetch=False):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    for name, (relative, expected) in SOURCES.items():
        path = directory / (name + ".upstream")
        if fetch:
            url = f"https://raw.githubusercontent.com/moby/profiles/{COMMIT}/{relative}"
            request = urllib.request.Request(url, headers={"User-Agent": "dunlin-test-provisioning"})
            with urllib.request.urlopen(request, timeout=30) as response:
                data = response.read(1024 * 1024 + 1)
            if hashlib.sha256(data).hexdigest() != expected:
                raise ValueError(f"Pinned source mismatch: {name}")
            path.write_bytes(data)
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"Pinned source mismatch: {name}")
    seccomp = json.loads((directory / "seccomp.upstream").read_text())
    if seccomp["defaultAction"] != "SCMP_ACT_ERRNO":
        raise ValueError("Expected default-deny upstream policy")
    seccomp["syscalls"].append({"names": NAMESPACE_CALLS, "action": "SCMP_ACT_ALLOW"})
    policies = {"runtime-seccomp.json": json.dumps(seccomp, sort_keys=True, indent=2) + "\n"}
    apparmor = (directory / "apparmor.upstream").read_text()
    if apparmor.count("  deny mount,") != 1:
        raise ValueError("Unexpected upstream mount rule")
    policies["runtime.apparmor"] = apparmor.replace('"default"', '"dunlin-runtime-checks"').replace(
        "  deny mount,", "  mount,\n  pivot_root,\n  userns,")
    hashes = {}
    for name, contents in policies.items():
        (directory / name).write_text(contents)
        hashes[name] = hashlib.sha256(contents.encode()).hexdigest()
    (directory / "provenance.json").write_text(json.dumps({"commit": COMMIT, "sources": SOURCES, "policies": hashes}, indent=2) + "\n")
    return hashes


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fetch", action="store_true")
    args = parser.parse_args()
    print(json.dumps(generate(args.output, args.fetch), sort_keys=True))
