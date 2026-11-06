#!/usr/bin/env python3
"""Inspect, never execute or extract, a Linux relay release archive."""
import argparse
import struct
import tarfile


def verify(path, arch):
    with tarfile.open(path, "r:gz") as archive:
        member = archive.next()
        if member is None or member.name != "dunlin-relay" or not member.isfile() or not 64 <= member.size <= 1024**3:
            raise ValueError("Invalid relay archive member")
        with archive.extractfile(member) as executable:
            header = executable.read(64)
        if header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != {"x86_64": 62, "arm64": 183}[arch]:
            raise ValueError("Wrong Linux executable architecture")
        if archive.next() is not None:
            raise ValueError("Expected exactly one relay executable")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive")
    parser.add_argument("--arch", choices=["x86_64", "arm64"], required=True)
    args = parser.parse_args()
    verify(args.archive, args.arch)
