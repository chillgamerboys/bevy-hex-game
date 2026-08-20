#!/usr/bin/env python3
"""Compile the pinned EOS C ABI probe against a protected official SDK mount.

Ordinary CI does not have or need the private portal artifact. Protected native jobs
pass an explicit include directory and compiler; this script performs no download and
never reads product credentials.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
PROBE = REPOSITORY_ROOT / "crates/hex_eos_ffi/abi/eos_1_19_1_probe.c"


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--include-dir",
        type=Path,
        required=True,
        help="absolute directory containing the official eos_sdk.h",
    )
    parser.add_argument(
        "--cc",
        default=os.environ.get("CC", "cc"),
        help="native C compiler for the protected target (default: CC or cc)",
    )
    return parser.parse_args()


def command(compiler: str, include_dir: Path, output: Path) -> list[str]:
    executable = Path(compiler).name.lower()
    if executable in {"cl", "cl.exe"}:
        return [
            compiler,
            "/nologo",
            "/W4",
            "/WX",
            "/std:c11",
            "/c",
            f"/I{include_dir}",
            f"/Fo{output}",
            str(PROBE),
        ]
    return [
        compiler,
        "-std=c11",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-c",
        "-I",
        str(include_dir),
        "-o",
        str(output),
        str(PROBE),
    ]


def main() -> int:
    args = arguments()
    include_dir = args.include_dir.resolve()
    if not args.include_dir.is_absolute():
        print("EOS SDK include directory must be absolute", file=sys.stderr)
        return 2
    if not (include_dir / "eos_sdk.h").is_file():
        print("EOS SDK include directory does not contain eos_sdk.h", file=sys.stderr)
        return 2
    compiler = shutil.which(args.cc)
    if compiler is None:
        print(f"C compiler is unavailable: {args.cc}", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="hex-eos-abi-") as temporary:
        suffix = ".obj" if Path(compiler).name.lower() in {"cl", "cl.exe"} else ".o"
        output = Path(temporary) / f"eos_abi_probe{suffix}"
        completed = subprocess.run(
            command(compiler, include_dir, output),
            check=False,
        )
        return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
