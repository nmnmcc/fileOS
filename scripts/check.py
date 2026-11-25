#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Run the checks CI runs.

Formatting (treefmt), lints, tests, the browser build of portable crates,
dependency policy and Python type safety (basedpyright, "standard" mode).
Stops at the first failing check.
"""

import json
import subprocess
from collections.abc import Callable
from pathlib import Path

import typer
from rich.console import Console

ROOT = Path(__file__).resolve().parent.parent

WASM_TARGET = "wasm32-unknown-unknown"


def portable_check() -> list[str]:
    """`cargo check` for the browser, for every crate with `portable = true`.

    The list comes from `[package.metadata.hs]` in each Cargo.toml, so it is
    never written down twice.
    """
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    packages: list[dict[str, object]] = json.loads(out)["packages"]
    command = ["cargo", "check", "--locked", "--target", WASM_TARGET]
    for package in packages:
        metadata = package.get("metadata")
        hs = metadata.get("hs") if isinstance(metadata, dict) else None
        if isinstance(hs, dict) and hs.get("portable") is True:
            command += ["--package", str(package["name"])]
    return command


# A check is a command, or a function that builds one when it runs.
CHECKS: list[list[str] | Callable[[], list[str]]] = [
    ["treefmt", "--ci"],
    ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
    ["cargo", "test", "--workspace", "--locked"],
    portable_check,  # shared with plugins, phone shells and the web app
    ["cargo", "deny", "check"],
    ["uv", "run", "--locked", "basedpyright"],
]

console = Console()


def main() -> None:
    """Run the checks CI runs. Stops at the first failing check."""
    for check in CHECKS:
        command = check() if callable(check) else check
        line = " ".join(command)
        console.print(f"[bold]$ {line}[/bold]", markup=True, highlight=False)
        if subprocess.run(command, cwd=ROOT).returncode != 0:
            console.print(f"\n[red]failed:[/red] {line}", highlight=False)
            raise typer.Exit(1)
    console.print("\n[green]All checks passed.[/green]")


if __name__ == "__main__":
    typer.run(main)
