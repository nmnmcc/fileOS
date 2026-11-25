#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Preview the documentation site in every language. It reloads when files change."""

import subprocess
from pathlib import Path
from typing import Annotated

import typer

DOCS = Path(__file__).resolve().parent.parent / "docs"


def main(
    open_browser: Annotated[
        bool, typer.Option("--open/--no-open", help="Open the site in a browser.")
    ] = True,
) -> None:
    """Install the site's dependencies if needed, then start the Astro dev server."""
    subprocess.run(["npm", "install", "--no-fund", "--no-audit"], cwd=DOCS, check=True)
    command = ["npm", "run", "dev"]
    if open_browser:
        command += ["--", "--open"]
    try:
        code = subprocess.run(command, cwd=DOCS).returncode
    except KeyboardInterrupt:
        code = 0
    raise typer.Exit(code)


if __name__ == "__main__":
    typer.run(main)
