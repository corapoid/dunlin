#!/usr/bin/env python3
"""Summarize measured LLVM coverage by package, never by counting test functions."""
import argparse
import json
from pathlib import Path


METRICS = ("lines", "functions", "regions", "branches")


def summarize(document, root):
    if document.get("type") != "llvm.coverage.json.export":
        raise ValueError("Expected an LLVM coverage export")
    packages, files, seen = {}, [], set()
    for data in document.get("data", []):
        for file in data.get("files", []):
            path = Path(file["filename"])
            if path.is_absolute():
                try:
                    path = path.relative_to(root)
                except ValueError:
                    continue
            parts = path.parts
            if len(parts) < 4 or parts[0] not in {"apps", "crates"} or parts[2] != "src":
                continue
            if path.name.endswith(("_smoke.rs", "_tests.rs")) or path.name in {"smoke.rs", "tests.rs"} or "tests" in parts:
                continue
            if str(path) in seen:
                raise ValueError(f"Duplicate coverage entry: {path}")
            seen.add(str(path))
            name = "/".join(parts[:2])
            summary = file["summary"]
            totals = packages.setdefault(name, {metric: {"count": 0, "covered": 0} for metric in METRICS})
            for metric in METRICS:
                counts = summary.get(metric, {"count": 0, "covered": 0})
                count, covered = counts["count"], counts["covered"]
                if not 0 <= covered <= count:
                    raise ValueError(f"Invalid {metric} counts for {path}")
                totals[metric]["count"] += count
                totals[metric]["covered"] += covered
            files.append((str(path), summary["lines"]["count"] - summary["lines"]["covered"]))
    if not packages:
        raise ValueError("Coverage report contains no workspace source files")
    return packages, sorted(files, key=lambda item: (-item[1], item[0]))


def percent(metric):
    count, covered = metric["count"], metric["covered"]
    return f"{100 * covered / count:.2f}% ({covered}/{count})" if count else "not measured"


def check_thresholds(packages, minimum_lines=0, minimum_desktop_lines=0):
    total = {key: sum(package["lines"][key] for package in packages.values()) for key in ["count", "covered"]}
    for name, metric, minimum in [
        ("workspace", total, minimum_lines),
        ("desktop", packages.get("apps/desktop", {}).get("lines"), minimum_desktop_lines),
    ]:
        if not 0 <= minimum <= 100:
            raise ValueError("Coverage minimums must be between 0 and 100")
        if minimum and (not metric or not metric["count"] or 100 * metric["covered"] / metric["count"] < minimum):
            actual = percent(metric) if metric else "not measured"
            raise ValueError(f"{name} line coverage {actual} is below the {minimum:g}% regression floor")


def markdown(packages, files):
    lines = ["# Measured Rust unit/integration coverage", "",
             "| Package | Lines | Functions | Regions | Branches |",
             "| --- | ---: | ---: | ---: | ---: |"]
    for name, metrics in sorted(packages.items()):
        lines.append("| " + name + " | " + " | ".join(percent(metrics[m]) for m in METRICS) + " |")
    totals = {metric: {key: sum(package[metric][key] for package in packages.values())
                       for key in ["count", "covered"]} for metric in METRICS}
    lines.append("| **Total** | " + " | ".join(percent(totals[m]) for m in METRICS) + " |")
    lines += ["", "## Largest untested line counts", "", "| File | Uncovered lines |", "| --- | ---: |"]
    lines += [f"| `{name}` | {missing} |" for name, missing in files[:20]]
    lines += ["", "Scope: executed Rust unit/integration tests, not native GUI scenario coverage. "
              "Vendor, build output, dedicated test files and native UI fixtures are excluded; inline test modules "
              "and development helper binaries can still contribute to source-file totals. Stable Rust does not measure branches "
              "here; use nightly cargo-llvm-cov --branch for a separate branch report. "
              "A percentage is not evidence that every control or assertion works."]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--require-desktop", action="store_true")
    parser.add_argument("--minimum-lines", type=float, default=0)
    parser.add_argument("--minimum-desktop-lines", type=float, default=0)
    args = parser.parse_args()
    packages, files = summarize(json.loads(args.report.read_text()), args.root.resolve())
    if args.require_desktop and "apps/desktop" not in packages:
        parser.error("Report is missing desktop coverage")
    print(markdown(packages, files), end="")
    check_thresholds(packages, args.minimum_lines, args.minimum_desktop_lines)


if __name__ == "__main__":
    main()
