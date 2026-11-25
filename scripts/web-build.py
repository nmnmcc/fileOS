#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Package the browser app (crates/hs-ui) with Native Shell and check the output.

`hs` builds it in when `HS_WEB_DIR` points at the printed directory; the
release build (`artifacts`) does this.
"""

import subprocess
from pathlib import Path

import typer

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / "crates" / "hs-ui"
OUTPUT = APP / ".nsh" / "web"
BUNDLE = OUTPUT / "homeserver"


def main() -> None:
    """Package the browser app and print where it is."""
    subprocess.run(["nsh", "package", "--target", "web", "--out", str(OUTPUT)], cwd=APP, check=True)
    if (
        not (BUNDLE / "index.html").is_file()
        or not any(BUNDLE.glob("nsh-start.*.js"))
        or not any(BUNDLE.rglob("*.wasm"))
    ):
        typer.echo(f"Native Shell did not produce a browser bundle at {BUNDLE}", err=True)
        raise typer.Exit(1)
    typer.echo(BUNDLE)


if __name__ == "__main__":
    typer.run(main)
