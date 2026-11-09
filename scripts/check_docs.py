#!/usr/bin/env python3
"""Offline local-link, heading and navigation checks for first-party Markdown.

No dependencies, network access, source rewrites or generated reports. This is a
repository documentation checker, not a complete CommonMark renderer.
"""
import argparse
from dataclasses import dataclass
from html import unescape
from html.parser import HTMLParser
from pathlib import Path
import re
import unicodedata
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
SECTIONS = (
    "getting-started", "user-guide", "administration", "reference",
    "architecture", "development", "quality", "project", "archive",
)


@dataclass(frozen=True)
class Link:
    target: str
    line: int


def blank(match):
    return "".join("\n" if char == "\n" else " " for char in match[0])


def prose(text):
    """Hide comments/fenced code while preserving offsets and line numbers."""
    text = re.sub(r"<!--.*?-->", blank, text, flags=re.S)
    lines = []
    fence = None
    for line in text.splitlines(keepends=True):
        marker = re.match(r"^ {0,3}(`{3,}|~{3,})(.*)$", line.rstrip("\r\n"))
        if fence:
            if (marker and marker[1][0] == fence[0]
                    and len(marker[1]) >= len(fence) and not marker[2].strip()):
                fence = None
            lines.append("".join("\n" if char == "\n" else " " for char in line))
        elif marker:
            fence = marker[1]
            lines.append("".join("\n" if char == "\n" else " " for char in line))
        else:
            lines.append(line)
    return "".join(lines)


class HTMLLinks(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.links = []
        self.anchors = set()

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if value is None:
                continue
            if key in {"href", "src"}:
                self.links.append(Link(value, self.getpos()[0]))
            if key == "id" or (tag == "a" and key == "name"):
                self.anchors.add(value)

    handle_startendtag = handle_starttag


def destination(text, start):
    """Read an angle-wrapped or balanced/escaped inline destination."""
    if start >= len(text):
        return ""
    if text[start] == "<":
        end = text.find(">", start + 1)
        return text[start + 1:end] if end >= 0 else ""
    chars = []
    depth = 0
    index = start
    while index < len(text):
        char = text[index]
        if char == "\\" and index + 1 < len(text):
            index += 1
            chars.append(text[index])
        elif char == "(":
            depth += 1
            chars.append(char)
        elif char == ")":
            if depth == 0:
                break
            depth -= 1
            chars.append(char)
        elif char.isspace() and depth == 0:
            break
        else:
            chars.append(char)
        index += 1
    return "".join(chars)


def links(text):
    visible = prose(text)
    # Backticks in code spans must not create fake Markdown or HTML links.
    visible = re.sub(r"(`+)(?!`)(.+?)(?<!`)\1(?!`)", blank, visible, flags=re.S)
    result = []
    # Starting at the closing bracket also handles a linked image inside a label.
    for match in re.finditer(r"\]\(\s*", visible):
        target = destination(visible, match.end())
        if target:
            result.append(Link(unescape(target), visible.count("\n", 0, match.start()) + 1))
    # Definitions are validated even when unused. Full/collapsed/shortcut uses
    # share that destination; undefined full references get an explicit error.
    definitions = {}
    for match in re.finditer(r"^ {0,3}\[([^\]\n]+)\]:[ \t]*", visible, re.M):
        target = destination(visible, match.end())
        definitions[" ".join(match[1].split()).casefold()] = target
        if target:
            result.append(Link(unescape(target), visible.count("\n", 0, match.start()) + 1))
    for match in re.finditer(r"!?\[([^\]\n]+)\]\[([^\]\n]*)\]", visible):
        label = " ".join((match[2] or match[1]).split()).casefold()
        if label not in definitions:
            result.append(Link("missing-reference:" + label,
                               visible.count("\n", 0, match.start()) + 1))
    parser = HTMLLinks()
    parser.feed(visible)
    return result + parser.links


def heading_slug(heading):
    heading = re.sub(r"!?\[([^\]]+)\]\([^)]*\)", r"\1", heading)
    heading = re.sub(r"<[^>]+>", "", heading)
    heading = unescape(heading).strip().lower().replace("`", "").replace("*", "").replace("~", "")
    heading = "".join(char for char in heading
                      if char in "-_ " or unicodedata.category(char)[0] not in "PS")
    return heading.replace(" ", "-")


def anchors(text):
    visible = prose(text)
    parser = HTMLLinks()
    parser.feed(visible)
    found = set()
    previous = ""
    for line in visible.splitlines():
        match = re.match(r"^ {0,3}#{1,6}\s+(.+?)\s*#*\s*$", line)
        heading = match[1] if match else None
        if heading is None and re.fullmatch(r" {0,3}(?:=+|-+)\s*", line) and previous.strip():
            heading = previous.strip()
        if heading is not None:
            slug = heading_slug(heading)
            candidate = slug
            index = 0
            while candidate in found:
                index += 1
                candidate = f"{slug}-{index}"
            found.add(candidate)
        previous = line
    return found | parser.anchors


def markdown_files(root):
    files = set(root.glob("*.md"))
    for directory in ("docs", "todo", ".github"):
        files.update((root / directory).rglob("*.md"))
    if (root / "web/README.md").is_file():
        files.add(root / "web/README.md")
    return sorted(path.resolve() for path in files if path.is_file())


def local_target(root, source, value):
    parsed = urlsplit(value)
    if parsed.scheme or parsed.netloc:
        return None
    path = unquote(parsed.path)
    target = (root / path.lstrip("/") if path.startswith("/")
              else source.parent / path) if path else source
    return target.resolve(), unquote(parsed.fragment)


def check(root):
    root = root.resolve()
    files = markdown_files(root)
    errors = []
    graph = {path: set() for path in files}
    anchor_cache = {}
    for source in files:
        label = source.relative_to(root)
        for link in links(source.read_text(encoding="utf-8")):
            if link.target.startswith("missing-reference:"):
                errors.append(f"{label}:{link.line}: undefined reference {link.target.split(':', 1)[1]!r}")
                continue
            try:
                resolved = local_target(root, source, link.target)
            except ValueError as error:
                errors.append(f"{label}:{link.line}: invalid URL {link.target!r}: {error}")
                continue
            if resolved is None:
                continue
            target, fragment = resolved
            if not target.is_relative_to(root):
                errors.append(f"{label}:{link.line}: link leaves repository: {link.target}")
                continue
            if not target.exists():
                errors.append(f"{label}:{link.line}: missing target: {link.target}")
                continue
            if target.is_dir() and (target / "README.md").is_file():
                target = target / "README.md"
            if target in graph:
                graph[source].add(target)
            if fragment and target.is_file() and target.suffix.lower() == ".md":
                if target not in anchor_cache:
                    anchor_cache[target] = anchors(target.read_text(encoding="utf-8"))
                if fragment.removeprefix("user-content-") not in anchor_cache[target]:
                    errors.append(f"{label}:{link.line}: missing anchor: {link.target}")

    home = root / "docs/README.md"
    required = [home] + [root / "docs" / section / "README.md" for section in SECTIONS]
    for path in required:
        if not path.is_file():
            errors.append(f"missing documentation index: {path.relative_to(root)}")
    # Historical pages may link to current docs, but cannot be the sole route to
    # an active page. This prevents archives from hiding a navigation gap.
    reached = set()
    pending = [home]
    archive = root / "docs/archive"
    while pending:
        source = pending.pop()
        if source in reached:
            continue
        reached.add(source)
        if not source.is_relative_to(archive):
            pending.extend(graph.get(source, set()) - reached)
    for path in files:
        if path.is_relative_to(root / "docs") and not path.is_relative_to(archive) and path not in reached:
            errors.append(f"unreachable from docs/README.md: {path.relative_to(root)}")
    return errors, len(files)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help="Repository root (for isolated fixtures)")
    args = parser.parse_args()
    errors, count = check(args.root)
    if errors:
        for error in errors:
            print(error)
        print(f"Documentation check failed: {len(errors)} error(s) in {count} Markdown files.")
        return 1
    print(f"Documentation check passed: {count} Markdown files; local links, anchors and navigation.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
