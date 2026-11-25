#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build the release artifacts with Nix and copy them into one directory.

Builds both Nix outputs, links `hsctl` to `hs`, then runs both with
`--version` as a smoke test.
"""

import json
import shutil
import subprocess
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
    with console.status("devenv build outputs.hs outputs.docs"):
        built = subprocess.run(
            ["devenv", "build", "outputs.hs", "outputs.docs"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
    # A cold Nix build can print fetch and build logs before the final JSON object.
    output_start = built.stdout.rfind("\n{") + 1
    outputs: dict[str, str] = json.loads(built.stdout[output_start:])

    shutil.rmtree(target, ignore_errors=True)
    target.mkdir(parents=True)
    hs = target / "hs"
    shutil.copyfile(Path(outputs["outputs.hs"]) / "bin" / "hs", hs)
    shutil.copytree(outputs["outputs.docs"], target / "docs")
    # Everything in the Nix store is read-only; make the copies ours.
    for path in [target, *target.rglob("*")]:
        path.chmod(0o755 if path.is_dir() or path == hs else 0o644)
    # Run as `hsctl`, hs behaves as `hs ctl`.
    hsctl = target / "hsctl"
    hsctl.symlink_to("hs")

    subprocess.run([hs, "--version"], check=True)
    subprocess.run([hsctl, "--version"], check=True)
    console.print(f"[green]Artifacts in {target}[/green]")


if __name__ == "__main__":
    typer.run(main)
