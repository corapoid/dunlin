#!/usr/bin/env python3
"""Run non-mutating Rust quality checks, reporting every failure before exiting."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CHECKS = (
    ("Format", ("cargo", "fmt", "--all", "--check")),
    ("Compiler", ("cargo", "check", "--workspace", "--all-targets", "--locked")),
    ("Clippy", ("cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings")),
    # Scan first-party workspace packages, not vendored forks or standalone plugins.
    ("Unused dependencies", ("cargo", "machete", "--skip-target-dir", "apps", "crates")),
)


def run_checks(root=ROOT):
    results = []
    for name, command in CHECKS:
        print(f"\n=== {name}: {' '.join(command)} ===", flush=True)
        try:
            code = subprocess.run(command, cwd=root, check=False).returncode
        except OSError as error:
            print(f"Cannot run {command[0]}: {error}", flush=True)
            code = 127
        results.append((name, code))

    print("\n=== Rust quality summary ===", flush=True)
    for name, code in results:
        print(f"{'PASS' if code == 0 else f'FAIL (exit {code})'}: {name}", flush=True)
    return int(any(code != 0 for _, code in results))


if __name__ == "__main__":
    raise SystemExit(run_checks())
