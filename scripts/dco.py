#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every commit is signed off (Developer Certificate of Origin).

Looks at the non-merge commits in BASE..HEAD and fails if any of them lacks a
Signed-off-by line. Fix a missing one with `git commit -s`, or several with
`git rebase --signoff BASE`.
"""

import subprocess
from typing import Annotated

import typer
from rich.console import Console

console = Console(stderr=True)


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def main(
    base: Annotated[str, typer.Argument(help="The branch the commits will be merged into.")],
    head: Annotated[str, typer.Argument(help="The last commit to check.")] = "HEAD",
) -> None:
    """Check that every commit in BASE..HEAD is signed off."""
    missing = [
        sha
        for sha in git("rev-list", "--no-merges", f"{base}..{head}").split()
        if not any(
            line.startswith("Signed-off-by: ")
            for line in git("show", "--no-patch", "--format=%B", sha).splitlines()
        )
    ]
    for sha in missing:
        console.print(f"[red]Commit {sha} is missing Signed-off-by[/red] (use git commit -s)")
    if missing:
        raise typer.Exit(1)


if __name__ == "__main__":
    typer.run(main)
