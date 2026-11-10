#!/usr/bin/env python3
"""Verify reviewed local fork trees and reject obsolete dependency reintroduction.

No downloads, source mutation, advisory exemptions or legacy codec execution.
"""
import hashlib
import json
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]
OBSOLETE = {"bincode", "instant", "paste", "rustybuzz", "ttf-parser"}


def tree_digest(directory):
    entries = []
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError("Symlink in reviewed vendor tree")
        if path.is_file():
            entries.append((path.relative_to(directory).as_posix(), hashlib.sha256(path.read_bytes()).hexdigest()))
    payload = "".join(f"{name}\0{digest}\n" for name, digest in entries).encode()
    return hashlib.sha256(payload).hexdigest(), len(entries)


def verify(root=ROOT):
    records = json.loads((root / "vendor/provenance.json").read_text())
    if records.get("schema") != 1:
        raise ValueError("Unknown vendor provenance schema")
    configured = tomllib.loads((root / "Cargo.toml").read_text())["patch"]["crates-io"]
    packages = {p["name"] for p in records["packages"]}
    if packages != set(configured):
        raise ValueError("Dependency patches do not match the reviewed provenance")
    for package in records["packages"]:
        relative = Path(configured[package["name"]]["path"])
        if relative.is_absolute() or ".." in relative.parts or relative.parts[0] != "vendor":
            raise ValueError("Dependency patch is outside vendor")
        directory = root / relative
        if directory.is_symlink() or (root / "vendor").is_symlink():
            raise ValueError("Symlinked vendor dependency root")
        manifest = tomllib.loads((directory / "Cargo.toml").read_text())["package"]
        if manifest["name"] != package["name"] or manifest["version"] != package["version"]:
            raise ValueError("Vendored package identity changed")
        digest, count = tree_digest(directory)
        if digest != package["tree_sha256"] or count != package["files"]:
            raise ValueError(f"Unreviewed vendor tree change: {package['name']}")
        changed_paths = set()
        for change in package.get("changes", []):
            changed_path = Path(change["path"])
            if (changed_path.is_absolute() or ".." in changed_path.parts
                    or not changed_path.parts or change["path"] in changed_paths):
                raise ValueError(f"Invalid or duplicate vendor change path: {package['name']}")
            changed_paths.add(change["path"])
            file = directory / changed_path
            if not file.is_file() or hashlib.sha256(file.read_bytes()).hexdigest() != change["patched_sha256"]:
                raise ValueError(f"Vendor per-file provenance mismatch: {package['name']}/{change['path']}")
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    unused = {p["name"] for p in lock.get("patch", {}).get("unused", [])} & packages
    if unused:
        raise ValueError(f"Unused dependency patches: {sorted(unused)}")
    present = OBSOLETE & {p["name"] for p in lock["package"]}
    if present:
        raise ValueError(f"Obsolete dependencies returned: {sorted(present)}")
    return len(packages)


if __name__ == "__main__":
    try:
        print(f"PASS: {verify()} reviewed vendor trees match; all five obsolete packages absent from lockfile")
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(f"Vendor verification failed: {error}") from error
