"""Behavioral documentation parser/navigation tests, without network or Cargo."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_docs


class DocumentationTests(unittest.TestCase):
    def test_inline_html_reference_images_and_balanced_destinations(self):
        text = '''[One](one.md#title) ![image](images/a.png)
[Space](<with space.md> "label") [Paren](with(parentheses).md)
[Escape](with\\(escaped\\).md)
<a href="other.md#id">HTML</a><img src='image.png' />
[![Badge](badge.png)](linked.md)
[named][ref] [ref][] [ref]
[ref]: target.md#anchor "title"
'''
        self.assertEqual(
            {link.target for link in check_docs.links(text)},
            {"one.md#title", "images/a.png", "with space.md", "with(parentheses).md",
             "with(escaped).md", "other.md#id", "image.png", "target.md#anchor",
             "badge.png", "linked.md"},
        )

    def test_code_comments_and_fences_do_not_create_links_or_anchors(self):
        text = '''# Real `code` *title*
`[fake](missing.md)` and ``<img src="missing.png">``
<!-- [hidden](hidden.md) -->
```md
# Not a heading
[code](bad.md)
````
~~~text
<a href="no.md">No</a>
~~~
[yes](real.md)
'''
        self.assertEqual([link.target for link in check_docs.links(text)], ["real.md"])
        self.assertEqual(check_docs.anchors(text), {"real-code-title"})

    def test_heading_anchors_unicode_duplicates_setext_and_html_ids(self):
        text = '''# One & two — `code`
## Repeat
## Repeat
## Repeat-1
Zażółć gęślą
------------
<a id="custom"></a><a name="legacy"></a>
'''
        self.assertEqual(check_docs.anchors(text), {
            "one--two--code", "repeat", "repeat-1", "repeat-1-1",
            "zażółć-gęślą", "custom", "legacy",
        })

    def fixture(self, root):
        for section in check_docs.SECTIONS:
            path = root / "docs" / section / "README.md"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"# {section}\n")
        (root / "docs/README.md").write_text("# Docs\n" + "\n".join(
            f"[{section}]({section}/README.md)" for section in check_docs.SECTIONS
        ))

    def test_valid_tree_relative_assets_encoded_fragments_and_directory_index(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            guide = root / "docs/user-guide/README.md"
            guide.write_text('# Guide\n[Page](a%20b.md#za%C5%BC%C3%B3%C5%82%C4%87)\n'
                             '[Directory](../reference/)\n<img src="image.png">\n')
            (guide.parent / "a b.md").write_text("# Zażółć\n[Here](#zażółć)\n")
            (guide.parent / "image.png").write_bytes(b"fixture")
            self.assertEqual(check_docs.check(root)[0], [])

    def test_missing_target_anchor_undefined_reference_and_escape_are_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            path = root / "docs/user-guide/README.md"
            path.write_text("# Guide\n[Missing](missing.md)\n[Bad](#absent)\n"
                            "[Ref][undefined]\n[Escape](../../../outside.md)\n")
            errors, _ = check_docs.check(root)
            self.assertTrue(any("missing target" in error for error in errors))
            self.assertTrue(any("missing anchor" in error for error in errors))
            self.assertTrue(any("undefined reference" in error for error in errors))
            self.assertTrue(any("leaves repository" in error for error in errors))

    def test_archive_cannot_be_the_only_route_to_an_active_page(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            (root / "docs/user-guide/orphan.md").write_text("# Orphan\n")
            (root / "docs/archive/README.md").write_text(
                "# Archive\n[Orphan](../user-guide/orphan.md)\n")
            errors, _ = check_docs.check(root)
            self.assertEqual(errors, ["unreachable from docs/README.md: docs/user-guide/orphan.md"])

    def test_missing_section_index_and_broken_archive_links_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs/archive").mkdir(parents=True)
            (root / "docs/README.md").write_text("# Docs\n")
            (root / "docs/archive/old.md").write_text("# History\n[Broken](missing.md)\n")
            errors, _ = check_docs.check(root)
            self.assertTrue(any("missing documentation index" in error for error in errors))
            self.assertTrue(any("archive/old.md:2: missing target" in error for error in errors))

    def test_scan_excludes_vendor_generated_output_and_external_urls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            for directory in ("vendor", "target", "dist"):
                path = root / directory / "README.md"
                path.parent.mkdir()
                path.write_text("[Broken](missing.md)")
            (root / "README.md").write_text(
                "[Web](https://example.invalid/missing#absent)\n"
                "[Mail](mailto:docs@example.invalid)\n")
            self.assertEqual(check_docs.check(root)[0], [])

    def test_cli_fails_then_succeeds_on_isolated_navigation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            path = root / "docs/README.md"
            original = path.read_text()
            path.write_text(original + "\n[Bad](absent.md)\n")
            command = [sys.executable, str(Path(check_docs.__file__)), "--root", str(root)]
            failed = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(failed.returncode, 1)
            self.assertIn("missing target", failed.stdout)
            path.write_text(original)
            passed = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(passed.returncode, 0, passed.stdout + passed.stderr)

    def test_required_ci_checks_run_the_documentation_checker(self):
        workflow = (check_docs.ROOT / ".github/workflows/ci.yml").read_text()
        checks = workflow.split("  macos-checks:", 1)[1].split("\n  macos-build:", 1)[0]
        self.assertIn("run: python3 scripts/check_docs.py", checks)
        self.assertNotIn("continue-on-error", checks)


if __name__ == "__main__":
    unittest.main()
