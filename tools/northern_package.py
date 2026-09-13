#!/usr/bin/env python3
"""Compile a Northern Archipelago package through Cargo and the pure V4 writer."""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets/config/v4/northern-archipelago/world.ron"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--output", type=Path, required=True,
                        help="Fresh immutable package directory; never overwritten")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/v4-authoring")
    args = parser.parse_args()
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target_dir.resolve()),
               CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2")
    return subprocess.call(["cargo", "run", "-p", "hex_world_tool", "--bin", "worldc", "--",
                            "northern-compile", "--source", str(args.source.resolve()),
                            "--output", str(args.output.resolve())], cwd=ROOT, env=env)


if __name__ == "__main__":
    raise SystemExit(main())
