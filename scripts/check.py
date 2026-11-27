#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Run the checks CI runs.

Formatting (treefmt), the WebAssembly apps, lints, tests, the browser build of
portable crates, dependency policy and Python type safety (basedpyright,
"standard" mode). Stops at the first check that fails.
"""

import json
import os
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

import typer
from rich.console import Console

ROOT = Path(__file__).resolve().parent.parent

WASM_TARGET = "wasm32-unknown-unknown"
APPS = ["--manifest-path", "apps/Cargo.toml"]


def portable_check() -> list[str]:
    """`cargo check` for the browser, for every crate with `portable = true`.

    The list comes from `[package.metadata.fileos]` in each Cargo.toml, so it is
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
        fileos = metadata.get("fileos") if isinstance(metadata, dict) else None
        if isinstance(fileos, dict) and fileos.get("portable") is True:
            command += ["--package", str(package["name"])]
    return command


# A check is a command, or a function that builds one when it runs.
CHECKS: list[list[str] | Callable[[], list[str]]] = [
    ["treefmt", "--ci"],
    # The engine and apps in target/apps/, which the tests of the node use.
    [sys.executable, str(ROOT / "scripts" / "apps.py")],
    ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
    [
        "cargo",
        "clippy",
        *APPS,
        "--target",
        WASM_TARGET,
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ],
    ["cargo", "test", "--workspace", "--locked"],
    ["cargo", "test", *APPS, "--locked"],
    ["cargo", "doc", "--workspace", "--no-deps", "--locked"],
    portable_check,  # shared with apps, phone shells and the web app
    ["cargo", "deny", "check"],
    ["cargo", "deny", *APPS, "check"],
    ["cargo", "machete"],
    ["uv", "run", "--locked", "basedpyright"],
]


console = Console()


def main() -> None:
    """Run the checks CI runs. Stops at the first check that fails."""
    checks: list[list[str] | Callable[[], list[str]]] = [*CHECKS]
    if msrv_cargo := os.environ.get("HS_MSRV_CARGO"):
        checks.append([msrv_cargo, "check", "--workspace", "--all-targets", "--locked"])
    for check in checks:
        command = check() if callable(check) else check
        line = " ".join(command)
        console.print(f"[bold]$ {line}[/bold]", markup=True, highlight=False)
        env = os.environ.copy()
        if command[:2] == ["cargo", "doc"]:
            env["RUSTDOCFLAGS"] = f"{env.get('RUSTDOCFLAGS', '')} -D warnings".strip()
        if subprocess.run(command, check=False, cwd=ROOT, env=env).returncode != 0:
            console.print(f"\n[red]failed:[/red] {line}", highlight=False)
            raise typer.Exit(1)
    console.print("\n[green]All checks passed.[/green]")


if __name__ == "__main__":
    typer.run(main)
