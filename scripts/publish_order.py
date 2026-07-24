#!/usr/bin/env python3
"""Print publishable workspace crates in dependency-first order.

The order is derived from Cargo metadata rather than duplicated in the release
workflow, so adding a new in-workspace dependency cannot silently omit it from
publishing — or publish it before the crate it depends on.

Unlike a root-rooted traversal, this walks *every* workspace member: the root
package (`tuika`) is a dependency of the member crate, not the other way round,
so starting from the root would never reach `tuika-codeformatters`.
"""

from __future__ import annotations

import json
import subprocess
from collections.abc import Mapping, Sequence
from typing import Any


def load_metadata() -> Mapping[str, Any]:
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
    )
    return json.loads(output)


def publish_order(metadata: Mapping[str, Any]) -> list[str]:
    packages = {
        package["name"]: package
        for package in metadata["packages"]
        if package["id"] in metadata["workspace_members"]
    }

    ordered: list[str] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(name: str) -> None:
        if name in visited:
            return
        if name in visiting:
            raise ValueError(f"workspace dependency cycle involving {name}")
        visiting.add(name)
        package = packages[name]
        dependencies: Sequence[Mapping[str, Any]] = package.get("dependencies", [])
        for dependency in dependencies:
            dependency_name = dependency["name"]
            if dependency.get("path") and dependency_name in packages:
                visit(dependency_name)
        visiting.remove(name)
        visited.add(name)
        # `publish = []` marks a crate as unpublishable: skip it in the output,
        # but keep walking through it so anything it depends on is still ordered.
        if package.get("publish") != []:
            ordered.append(name)

    # Sorted for determinism: two crates with no dependency between them would
    # otherwise be ordered by however cargo happened to emit them.
    for name in sorted(packages):
        visit(name)
    return ordered


def main() -> None:
    for crate in publish_order(load_metadata()):
        print(crate)


if __name__ == "__main__":
    main()
