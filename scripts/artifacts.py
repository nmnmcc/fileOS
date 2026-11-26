#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build the release artifacts with Nix and copy them into one directory.

Builds both Nix outputs and the WebAssembly apps (scripts/apps.py). Then it
runs `fos --version`, as a quick test that it starts.
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Annotated

import typer
from rich.console import Console

ROOT = Path(__file__).resolve().parent.parent

console = Console()


def main(
    target: Annotated[
        Path, typer.Argument(help="Where to put the artifacts.", file_okay=False)
    ] = ROOT / "artifacts",
) -> None:
    """Build the release artifacts with Nix and copy them into TARGET."""
    with console.status("devenv build outputs.fos outputs.docs"):
        built = subprocess.run(
            ["devenv", "build", "outputs.fos", "outputs.docs"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
    # A Nix build with an empty cache can print download and build logs
    # before the last JSON object.
    output_start = built.stdout.rfind("\n{") + 1
    outputs: dict[str, str] = json.loads(built.stdout[output_start:])

    shutil.rmtree(target, ignore_errors=True)
    target.mkdir(parents=True)
    fos = target / "fos"
    shutil.copyfile(Path(outputs["outputs.fos"]) / "bin" / "fos", fos)
    shutil.copytree(outputs["outputs.docs"], target / "docs")
    # The engine and the apps that come with fileOS, as .app folders.
    subprocess.run([sys.executable, str(ROOT / "scripts" / "apps.py")], cwd=ROOT, check=True)
    shutil.copytree(ROOT / "target" / "apps", target / "apps")
    # Everything in the Nix store is read-only, so give the copies our own permissions.
    for path in [target, *target.rglob("*")]:
        path.chmod(0o755 if path.is_dir() or path == fos else 0o644)

    subprocess.run([fos, "--version"], check=True)
    console.print(f"[green]Artifacts in {target}[/green]")


if __name__ == "__main__":
    typer.run(main)
