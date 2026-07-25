#!/usr/bin/env python3

import unittest

from publish_order import publish_order


class PublishOrderTests(unittest.TestCase):
    def test_orders_dependency_before_dependent(self) -> None:
        # The real shape: both companions depend on the root package, so a
        # traversal rooted at `tuika` alone would never reach them.
        metadata = {
            "workspace_members": ["tuika-id", "formatter-id", "mermaid-id"],
            "packages": [
                {
                    "id": "formatter-id",
                    "name": "tuika-codeformatters",
                    "publish": None,
                    "dependencies": [{"name": "tuika", "path": ".."}],
                },
                {
                    "id": "tuika-id",
                    "name": "tuika",
                    "publish": None,
                    "dependencies": [],
                },
                {
                    "id": "mermaid-id",
                    "name": "tuika-mermaid",
                    "publish": None,
                    "dependencies": [{"name": "tuika", "path": ".."}],
                },
            ],
        }

        self.assertEqual(
            publish_order(metadata),
            ["tuika", "tuika-codeformatters", "tuika-mermaid"],
        )

    def test_orders_transitive_workspace_dependencies(self) -> None:
        metadata = {
            "workspace_members": ["tuika-id", "formatter-id", "extra-id"],
            "packages": [
                {
                    "id": "extra-id",
                    "name": "extra",
                    "publish": None,
                    "dependencies": [
                        {"name": "tuika-codeformatters", "path": "formatter"}
                    ],
                },
                {
                    "id": "formatter-id",
                    "name": "tuika-codeformatters",
                    "publish": None,
                    "dependencies": [{"name": "tuika", "path": ".."}],
                },
                {
                    "id": "tuika-id",
                    "name": "tuika",
                    "publish": None,
                    "dependencies": [],
                },
            ],
        }

        self.assertEqual(
            publish_order(metadata),
            ["tuika", "tuika-codeformatters", "extra"],
        )

    def test_excludes_non_publishable_crate(self) -> None:
        metadata = {
            "workspace_members": ["tuika-id", "private-id"],
            "packages": [
                {
                    "id": "tuika-id",
                    "name": "tuika",
                    "publish": None,
                    "dependencies": [],
                },
                {
                    "id": "private-id",
                    "name": "private",
                    "publish": [],
                    "dependencies": [{"name": "tuika", "path": ".."}],
                },
            ],
        }

        self.assertEqual(publish_order(metadata), ["tuika"])

    def test_detects_dependency_cycle(self) -> None:
        metadata = {
            "workspace_members": ["a-id", "b-id"],
            "packages": [
                {
                    "id": "a-id",
                    "name": "a",
                    "publish": None,
                    "dependencies": [{"name": "b", "path": "b"}],
                },
                {
                    "id": "b-id",
                    "name": "b",
                    "publish": None,
                    "dependencies": [{"name": "a", "path": "a"}],
                },
            ],
        }

        with self.assertRaises(ValueError):
            publish_order(metadata)


if __name__ == "__main__":
    unittest.main()
