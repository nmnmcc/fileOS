#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build the WebAssembly apps and engines into .app folders.

Each crate in apps/ that has a `<Name>.app/` folder is built for
wasm32-unknown-unknown as a cdylib, made into a component with
`wasm-tools component new`, and put with the files of its `<Name>.app/` folder
into target/apps/<Name>.app/. The crates themselves are plain libraries, so
their tests also build and run on the machine itself.
"""

import shutil
import subprocess
from pathlib import Path

import typer
from rich.console import Console

ROOT = Path(__file__).resolve().parent.parent
APPS = ROOT / "apps"
BUILD = ROOT / "target" / "apps-build"
OUT = ROOT / "target" / "apps"
TARGET = "wasm32-unknown-unknown"

console = Console()


def run(command: list[str]) -> None:
    console.print(f"[bold]$ {' '.join(command)}[/bold]", markup=True, highlight=False)
    if subprocess.run(command, check=False, cwd=ROOT).returncode != 0:
        raise typer.Exit(1)


def main() -> None:
    """Build every app and engine in apps/ into target/apps/."""
    for crate in sorted(p for p in APPS.iterdir() if p.is_dir()):
        for bundle in crate.glob("*.app"):
            run(
                [
                    "cargo",
                    "rustc",
                    "--release",
                    "--locked",
                    "--manifest-path",
                    str(crate / "Cargo.toml"),
                    "--target",
                    TARGET,
                    "--target-dir",
                    str(BUILD),
                    "--crate-type",
                    "cdylib",
                ]
            )
            module = BUILD / TARGET / "release" / f"{crate.name.replace('-', '_')}.wasm"
            out = OUT / bundle.name
            if out.exists():
                shutil.rmtree(out)
            shutil.copytree(bundle, out)
            (out / "bin").mkdir(exist_ok=True)
            run(
                ["wasm-tools", "component", "new", str(module), "-o", str(out / "bin" / "app.wasm")]
            )
            console.print(f"[green]built[/green] {out.relative_to(ROOT)}", highlight=False)


if __name__ == "__main__":
    typer.run(main)
