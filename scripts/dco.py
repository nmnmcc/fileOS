#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every commit is signed off (Developer Certificate of Origin).

Looks at the non-merge commits in BASE..HEAD and fails if any of them lacks a
Signed-off-by line. Commits made by GitHub Apps such as Dependabot
(`...[bot]@users.noreply.github.com`) do not need one: a bot is not able to
give the certificate, and the person who merges its changes checks them. Fix a missing one with `git commit -s`, or several with
`git rebase --signoff BASE`.
"""

import subprocess
from typing import Annotated

import typer
from rich.console import Console

console = Console(stderr=True)


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def signed_off(sha: str) -> bool:
    """Whether the commit has a sign-off, or was made by a bot."""
    email, _, message = git("show", "--no-patch", "--format=%ae%n%B", sha).partition("\n")
    return email.endswith("[bot]@users.noreply.github.com") or any(
        line.startswith("Signed-off-by: ") for line in message.splitlines()
    )


def main(
    base: Annotated[str, typer.Argument(help="The branch the commits will be merged into.")],
    head: Annotated[str, typer.Argument(help="The last commit to check.")] = "HEAD",
) -> None:
    """Check that every commit in BASE..HEAD is signed off."""
    missing = [
        sha
        for sha in git("rev-list", "--no-merges", f"{base}..{head}").split()
        if not signed_off(sha)
    ]
    for sha in missing:
        console.print(f"[red]Commit {sha} is missing Signed-off-by[/red] (use git commit -s)")
    if missing:
        raise typer.Exit(1)


if __name__ == "__main__":
    typer.run(main)
