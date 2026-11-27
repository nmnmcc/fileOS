#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check the compile-time budget.

Measures the time of a clean debug build, of a rebuild after a change to a leaf
crate, and of a rebuild after a change to the core crate. Fails if any of them
goes over its budget.
"""

import subprocess
import time
from pathlib import Path
from typing import Annotated

import typer
from rich.console import Console
from rich.table import Table

ROOT = Path(__file__).resolve().parent.parent
LEAF_FILE = ROOT / "crates/fileos/src/main.rs"
CORE_FILE = ROOT / "crates/fileos-core/src/lib.rs"

console = Console()


def timed_build(*extra: str) -> float:
    """Build the whole workspace in debug mode and return the wall-clock time."""
    start = time.perf_counter()
    result = subprocess.run(
        ["cargo", "build", "--workspace", "--locked", *extra],
        check=False,
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    elapsed = time.perf_counter() - start
    if result.returncode != 0:
        console.print(result.stderr, markup=False)
        console.print("[red]build failed[/red]")
        raise typer.Exit(1)
    return elapsed


def main(
    clean: Annotated[
        float, typer.Option(envvar="BUDGET_CLEAN", help="Budget for a clean build, in seconds.")
    ] = 360,
    leaf: Annotated[
        float, typer.Option(envvar="BUDGET_LEAF", help="Budget after a change to a leaf crate.")
    ] = 5,
    core: Annotated[
        float, typer.Option(envvar="BUDGET_CORE", help="Budget after a change to fileos-core.")
    ] = 20,
) -> None:
    """Check the compile-time budget."""
    subprocess.run(["cargo", "clean", "--quiet"], cwd=ROOT, check=True)

    with console.status("Clean build..."):
        # Only the clean build writes the per-crate timing report.
        clean_seconds = timed_build("--timings")
    with console.status("Rebuild after a change to a leaf crate..."):
        LEAF_FILE.touch()  # make it look changed, but keep the contents
        leaf_seconds = timed_build()
    with console.status("Rebuild after a change to fileos-core..."):
        CORE_FILE.touch()
        core_seconds = timed_build()

    results = [
        ("clean build", clean_seconds, clean),
        ("incremental (leaf)", leaf_seconds, leaf),
        ("incremental (core)", core_seconds, core),
    ]

    table = Table(title="Compile-time budget", title_justify="left")
    table.add_column("")
    table.add_column("build")
    table.add_column("time", justify="right")
    table.add_column("budget", justify="right")
    for label, seconds, budget in results:
        status = "[green]ok[/green]" if seconds <= budget else "[red]FAIL[/red]"
        table.add_row(status, label, f"{seconds:.2f}s", f"{budget:g}s")
    console.print(table)
    console.print("Per-crate timings: target/cargo-timings/cargo-timing.html")

    if any(seconds > budget for _, seconds, budget in results):
        raise typer.Exit(1)


if __name__ == "__main__":
    typer.run(main)
