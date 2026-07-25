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
            (root / "log.md").write_text("# Log\n\n[Concept](specs/concept.md)\n")
            (root / "specs").mkdir()
            (root / "specs" / "concept.md").write_text("---\ntype: Policy\n---\n")
            messages, _ = validate(root)
            self.assertEqual(messages, ["specs/concept.md: concept is not listed in index.md"])


if __name__ == "__main__":
    unittest.main()
