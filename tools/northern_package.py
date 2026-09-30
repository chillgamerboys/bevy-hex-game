#!/usr/bin/env python3
"""Compile a Northern Archipelago package through Cargo and the pure V4 writer."""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets/config/v4/northern-archipelago/world.ron"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", nargs="?", choices=("ensure", "compile"), default="compile")
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--output", type=Path, default=ROOT / "assets/config/v4/northern-archipelago/compiled",
                        help="Fresh immutable package directory; never overwritten")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/v4-authoring")
    args = parser.parse_args()
    if args.mode == "ensure" and (args.output / "manifest.ron").is_file() and (args.output / "northern-overview.ron").is_file():
        return 0
    output = args.output
    if args.mode == "ensure":
        output.parent.mkdir(parents=True, exist_ok=True)
        output = Path(tempfile.mkdtemp(prefix="northern-build-", dir=output.parent)) / "compiled"
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target_dir.resolve()),
               CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2")
    result = subprocess.call(["cargo", "run", "-p", "hex_world_tool", "--bin", "worldc", "--",
                            "northern-compile", "--source", str(args.source.resolve()),
                            "--output", str(output.resolve())], cwd=ROOT, env=env)
    if result == 0 and args.mode == "ensure":
        output.rename(args.output)
        output.parent.rmdir()
    return result


if __name__ == "__main__":
    raise SystemExit(main())
