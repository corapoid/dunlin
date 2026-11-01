#!/usr/bin/env python3
"""Fail-closed release metadata, notes and downloaded-artifact verification.

Never executes an app/relay, mounts a DMG, installs anything or mutates GitHub.
Checksum verification is integrity evidence, not Apple signing/notarization.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import struct
import tarfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?")


def version(value):
    match = VERSION.fullmatch(value)
    if not match or any(x.isdigit() and len(x) > 1 and x[0] == "0"
                        for x in (match[4] or "").split(".")):
        raise ValueError("Expected a plain SemVer release version (no v, whitespace or shell syntax)")
    return value


def app_name(value):
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9 _-]{0,63}", value) or value.endswith(" "):
        raise ValueError("Invalid application name")
    return value


def workspace_version(root=ROOT):
    return version(tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"])


def changelog(root, value):
    text = (root / "CHANGELOG.md").read_text()
    heading = re.compile(r"^## \[" + re.escape(value) + r"\](?:[^\n]*)$", re.M)
    matches = list(heading.finditer(text))
    if len(matches) != 1:
        raise ValueError("Release needs exactly one matching changelog section")
    body = re.split(r"^## ", text[matches[0].end():], maxsplit=1, flags=re.M)[0].strip()
    if not body or len(body.encode()) > 128 * 1024:
        raise ValueError("Changelog section is empty or oversized")
    return body


def metadata(root, value):
    version(value)
    if workspace_version(root) != value:
        raise ValueError("Requested version does not match the workspace")
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    packages = [p for p in lock["package"] if "source" not in p]
    if not packages or any(p["version"] != value for p in packages):
        raise ValueError("Workspace Cargo.lock versions do not match")
    return changelog(root, value)


def artifact_names(value, name="Dunlin"):
    version(value)
    app_name(name)
    return {f"{name}-{value}-macos-arm64.zip", f"{name}-{value}-macos-arm64.dmg",
            f"dunlin-relay-{value}-macos-arm64.tar.gz"}


def notary_result(report):
    regular(report, 1024 * 1024)
    result = json.loads(report.read_text())
    if not isinstance(result, dict) or result.get("status") != "Accepted" or not result.get("id"):
        raise ValueError("Apple notarization is not explicitly Accepted")
    return result["id"]


def signing_configuration(values):
    certificate = bool(values.get("APPLE_CERTIFICATE"))
    password = bool(values.get("APPLE_CERTIFICATE_PASSWORD"))
    if certificate != password:
        raise ValueError("Certificate and password must be configured together")
    notary = [bool(values.get(k)) for k in ["APPLE_ID", "APPLE_TEAM_ID", "APPLE_APP_PASSWORD"]]
    if any(notary) and (not all(notary) or not certificate):
        raise ValueError("Notarization needs a certificate and all three notary credentials")
    return certificate, all(notary)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def regular(path, maximum=2 * 1024**3):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= maximum:
        raise ValueError(f"Missing, symlinked, empty or oversized release file: {path.name}")


def arm64(header):
    # Shipped archives must contain thin 64-bit ARM Mach-O executables.
    if len(header) < 8 or struct.unpack("<II", header[:8]) != (0xFEEDFACF, 0x0100000C):
        raise ValueError("Release executable is not thin macOS arm64 Mach-O")


def safe_member(name):
    path = PurePosixPath(name)
    if (not name or "\\" in name or "\0" in name or path.is_absolute() or ".." in path.parts
            or str(path) != name.rstrip("/")):
        raise ValueError("Unsafe archive member")


def verify(directory, value, name="Dunlin", bundle_id="dev.dunlin.ide"):
    expected = artifact_names(value, name)
    manifest = directory / "SHA256SUMS"
    regular(manifest, 8192)
    checksums = {}
    for line in manifest.read_text().splitlines():
        match = re.fullmatch(r"([a-f0-9]{64}) [ *](.+)", line)
        if not match:
            raise ValueError("Malformed SHA256SUMS entry")
        checksum, filename = match.groups()
        # Older beta manifests included dist/. Accept only that literal prefix,
        # never arbitrary directories, traversal or ambiguous basenames.
        filename = filename.removeprefix("dist/")
        if filename not in expected or filename in checksums:
            raise ValueError("Unexpected or duplicate SHA256SUMS entry")
        checksums[filename] = checksum
    if set(checksums) != expected:
        raise ValueError("SHA256SUMS does not cover exactly the expected three artifacts")
    for filename, checksum in checksums.items():
        path = directory / filename
        regular(path)
        if digest(path) != checksum:
            raise ValueError(f"Checksum mismatch: {filename}")

    with zipfile.ZipFile(directory / f"{name}-{value}-macos-arm64.zip") as archive:
        entries = archive.infolist()
        if len(entries) > 10000 or sum(i.file_size for i in entries) > 1024**3:
            raise ValueError("ZIP exceeds release inspection limits")
        names = set()
        for entry in entries:
            safe_member(entry.filename)
            member = PurePosixPath(entry.filename)
            if member.parts[0] not in [f"{name}.app", "__MACOSX"]:
                raise ValueError("Unexpected ZIP root member")
            canonical = str(member)
            if canonical in names or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("Duplicate or symlinked ZIP member")
            names.add(canonical)
        info = archive.getinfo(f"{name}.app/Contents/Info.plist")
        if info.file_size > 1024 * 1024:
            raise ValueError("Oversized bundle metadata")
        plist = plistlib.loads(archive.read(info))
        expected_plist = {"CFBundleShortVersionString": value, "CFBundleIdentifier": bundle_id,
                          "CFBundleExecutable": name, "LSMinimumSystemVersion": "15.0"}
        if any(plist.get(k) != v for k, v in expected_plist.items()):
            raise ValueError("Bundle identity/version/platform does not match release")
        with archive.open(f"{name}.app/Contents/MacOS/{name}") as executable:
            arm64(executable.read(8))

    with tarfile.open(directory / f"dunlin-relay-{value}-macos-arm64.tar.gz", "r:gz") as archive:
        entries = 0
        for entry in archive:
            entries += 1
            if entries != 1 or entry.name != "dunlin-relay" or not entry.isfile() or entry.size > 1024**3:
                raise ValueError("Unexpected relay tar member")
            with archive.extractfile(entry) as executable:
                arm64(executable.read(8))
        if entries != 1:
            raise ValueError("Relay tar is empty")
    with (directory / f"{name}-{value}-macos-arm64.dmg").open("rb") as dmg:
        dmg.seek(-512, 2)
        if dmg.read(4) != b"koly":
            raise ValueError("DMG is not a UDIF image")
    return sorted(expected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    show = commands.add_parser("version")
    show.add_argument("--value")
    for command in ["metadata", "notes"]:
        child = commands.add_parser(command)
        child.add_argument("--version", required=True, type=version)
        if command == "notes":
            child.add_argument("--signing", choices=["ad-hoc", "developer-id", "notarized"], required=True)
    check = commands.add_parser("verify")
    check.add_argument("--version", required=True, type=version)
    check.add_argument("--directory", required=True, type=Path)
    check.add_argument("--app-name", default="Dunlin", type=app_name)
    check.add_argument("--bundle-id", default="dev.dunlin.ide")
    notary = commands.add_parser("notary")
    notary.add_argument("--report", required=True, type=Path)
    commands.add_parser("signing-config")
    args = parser.parse_args()
    try:
        if args.command == "version":
            print(version(args.value) if args.value is not None else workspace_version())
        elif args.command in ["metadata", "notes"]:
            body = metadata(ROOT, args.version)
            if args.command == "metadata":
                print(f"PASS: workspace, lockfile and changelog match {args.version}")
            else:
                signing = {"ad-hoc": "Ad-hoc signed beta; NOT Apple-notarized. Gatekeeper approval may be required.",
                           "developer-id": "Developer ID signed; NOT Apple-notarized.",
                           "notarized": "Developer ID signed and Apple-notarized app/relay; stapled app/DMG."}[args.signing]
                print(f"Dunlin {args.version}\n\nmacOS 15+, Apple Silicon.\n\n{signing}\n\n{body}\n")
        elif args.command == "signing-config":
            certificate, notarize = signing_configuration(os.environ)
            print(f"certificate={str(certificate).lower()}\nnotarize={str(notarize).lower()}")
        elif args.command == "notary":
            notary_result(args.report)
            print("PASS: Apple notarization report explicitly Accepted")
        else:
            names = verify(args.directory, args.version, args.app_name, args.bundle_id)
            print("PASS: SHA256SUMS, bundle version/identity, arm64 app/relay and DMG header: " + ", ".join(names))
    except (ValueError, OSError, KeyError, zipfile.BadZipFile, tarfile.TarError, plistlib.InvalidFileException) as error:
        parser.exit(1, f"Release gate failed: {error}\n")


if __name__ == "__main__":
    main()
