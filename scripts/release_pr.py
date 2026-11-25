#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Open a release pull request. `knope prepare-release` runs this.

Knope has already put the new VERSION into Cargo.toml and the change files
into CHANGELOG.md. This script brings Cargo.lock up to date, puts all the
changes into one signed-off commit on the branch release/vVERSION, sends it to
GitHub and opens a pull request into main. When the pull request is merged,
the Release workflow makes the tag and the GitHub release.
"""

import subprocess
from pathlib import Path
from typing import Annotated

import typer
from rich.console import Console

ROOT = Path(__file__).resolve().parent.parent

console = Console()


def run(*args: str) -> None:
    subprocess.run(args, cwd=ROOT, check=True)


def notes(version: str) -> str:
    """The part of CHANGELOG.md for VERSION, without its heading."""
    lines: list[str] = []
    inside = False
    for line in (ROOT / "CHANGELOG.md").read_text().splitlines():
        if line.startswith("## "):
            if inside:
                break
            inside = line.split()[1] == version
        elif inside:
            lines.append(line)
    return "\n".join(lines).strip()


def main(
    version: Annotated[str, typer.Argument(help="The new version, from Knope.")],
) -> None:
    """Commit the release changes and open a pull request for VERSION."""
    branch = f"release/v{version}"
    title = f"Release {version}"
    # Only the versions of workspace crates change; nothing is fetched.
    run("cargo", "update", "--workspace", "--offline")
    run("git", "switch", "--create", branch)
    run("git", "add", "--all", "Cargo.toml", "Cargo.lock", "CHANGELOG.md", ".changeset")
    run("git", "commit", "--signoff", "--message", title)
    run("git", "push", "--set-upstream", "origin", branch)
    body = (
        f"When this is merged, the Release workflow makes the tag v{version} "
        f"and the GitHub release, with these notes:\n\n{notes(version)}\n"
    )
    run("gh", "pr", "create", "--base", "main", "--title", title, "--body", body)
    console.print(f"[green]Opened the pull request for {version}[/green]")


if __name__ == "__main__":
    typer.run(main)
