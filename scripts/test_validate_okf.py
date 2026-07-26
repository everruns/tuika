import importlib.util
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("validate_okf.py")
SPEC = importlib.util.spec_from_file_location("validate_okf", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
validate = MODULE.validate


class ValidateOkfTests(unittest.TestCase):
    def test_accepts_typed_concept_and_existing_relative_link(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "index.md").write_text("# Index\n\n[Concept](specs/concept.md)\n")
            (root / "specs").mkdir()
            (root / "specs" / "concept.md").write_text("---\ntype: Policy\n---\n\n# Concept\n")
            messages, concepts = validate(root, check_links=True)
            self.assertEqual(messages, [])
            self.assertEqual(concepts, 1)

    def test_rejects_missing_type_and_broken_relative_link(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "concept.md").write_text("---\ntitle: Concept\n---\n\n[Missing](missing.md)\n")
            messages, concepts = validate(root, check_links=True)
            self.assertEqual(concepts, 1)
            self.assertTrue(any("no non-empty `type`" in message for message in messages))
            self.assertTrue(any("broken link -> missing.md" in message for message in messages))

    def test_reserved_files_need_no_frontmatter_but_links_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "index.md").write_text("# Index\n\n[Missing](specs/missing.md)\n")
            messages, concepts = validate(root, check_links=True)
            self.assertEqual(concepts, 0)
            self.assertEqual(messages, ["index.md: broken link -> specs/missing.md"])

    def test_accepts_root_index_version_and_date_grouped_log(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "index.md").write_text(
                '---\nokf_version: "0.2"\n---\n\n# Index\n'
            )
            (root / "log.md").write_text(
                "# Knowledge Log\n\n## 2026-07-25\n\n- **Update**: Current.\n"
                "\n## 2026-07-24\n\n- **Creation**: Initial.\n"
            )
            messages, concepts = validate(root)
            self.assertEqual(messages, [])
            self.assertEqual(concepts, 0)

    def test_rejects_decorated_log_date_heading(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "log.md").write_text(
                "# Knowledge Log\n\n"
                "## 2026-07-25 — Update\n\n- First.\n\n"
                "## 2026-07-25\n\n- Second.\n"
            )
            messages, _ = validate(root)
            self.assertIn(
                "log.md: log date heading is not ISO 8601 `YYYY-MM-DD`: "
                "2026-07-25 — Update",
                messages,
            )

    def test_rejects_repeated_log_date_groups(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "log.md").write_text(
                "# Knowledge Log\n\n"
                "## 2026-07-25\n\n- First.\n\n"
                "## 2026-07-25\n\n- Second.\n"
            )
            messages, _ = validate(root)
            self.assertEqual(
                messages,
                ["log.md: log entries for the same date must share one date group"],
            )

    def test_rejects_impossible_log_date(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "log.md").write_text(
                "# Knowledge Log\n\n## 2026-02-30\n\n- Impossible.\n"
            )
            messages, _ = validate(root)
            self.assertEqual(
                messages,
                [
                    "log.md: log date heading is not ISO 8601 `YYYY-MM-DD`: "
                    "2026-02-30"
                ],
            )

    def test_rejects_nested_index_frontmatter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "group").mkdir()
            (root / "group" / "index.md").write_text(
                '---\nokf_version: "0.2"\n---\n\n# Group\n'
            )
            messages, _ = validate(root)
            self.assertEqual(
                messages,
                [
                    "group/index.md: frontmatter is only permitted in the "
                    "bundle-root index.md"
                ],
            )

    def test_rejects_log_groups_that_are_not_newest_first(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "log.md").write_text(
                "# Knowledge Log\n\n## 2026-07-24\n\n- Old.\n"
                "\n## 2026-07-25\n\n- New.\n"
            )
            messages, _ = validate(root)
            self.assertEqual(
                messages, ["log.md: log date groups are not newest first"]
            )

    def test_rejects_concept_the_index_does_not_list(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "index.md").write_text("# Index\n\n[Listed](specs/listed.md)\n")
            (root / "specs").mkdir()
            (root / "specs" / "listed.md").write_text("---\ntype: Policy\n---\n")
            (root / "specs" / "orphan.md").write_text("---\ntype: Policy\n---\n")
            messages, concepts = validate(root)
            self.assertEqual(concepts, 2)
            self.assertEqual(messages, ["specs/orphan.md: concept is not listed in index.md"])

    def test_moving_a_concept_without_updating_the_index_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # The index still points at the old location after the move.
            (root / "index.md").write_text("# Index\n\n[Shipping](specs/shipping.md)\n")
            (root / "processes").mkdir()
            (root / "processes" / "shipping.md").write_text("---\ntype: Process\n---\n")
            messages, _ = validate(root, check_links=True)
            self.assertIn("processes/shipping.md: concept is not listed in index.md", messages)
            self.assertIn("index.md: broken link -> specs/shipping.md", messages)

    def test_bundle_without_an_index_skips_the_coverage_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "concept.md").write_text("---\ntype: Policy\n---\n")
            messages, concepts = validate(root)
            self.assertEqual(concepts, 1)
            self.assertEqual(messages, [])

    def test_log_entries_do_not_count_as_index_listings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "index.md").write_text("# Index\n")
            (root / "log.md").write_text(
                "# Log\n\n## 2026-07-25\n\n- [Concept](specs/concept.md)\n"
            )
            (root / "specs").mkdir()
            (root / "specs" / "concept.md").write_text("---\ntype: Policy\n---\n")
            messages, _ = validate(root)
            self.assertEqual(messages, ["specs/concept.md: concept is not listed in index.md"])


if __name__ == "__main__":
    unittest.main()
