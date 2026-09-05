#!/usr/bin/env python3
"""Build once, then run the V4 authoring tool with source/binary identity checks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
PACKAGES = ("hex_world_contracts", "hex_world_runtime", "hex_schematic", "hex_world_tool")


def source_identity(root: Path = ROOT) -> str:
    """Hash compiler inputs, deliberately excluding editable world source assets."""
    inputs = [root / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml")]
    # The shared schematic crate still embeds this frozen reference module. V4
    # authoring RON remains runtime input; this legacy include_str is binary input.
    inputs.append(root / "assets/config/schematics/grand-v3-template.ron")
    for package in PACKAGES:
        directory = root / "crates" / package
        if not directory.is_dir():
            raise ValueError(f"missing compiler package: {directory}")
        inputs.append(directory / "Cargo.toml")
        inputs.extend(sorted((directory / "src").rglob("*.rs")))
    result = hashlib.sha256()
    for path in sorted(inputs):
        if not path.is_file():
            raise ValueError(f"missing compiler input: {path}")
        result.update(path.relative_to(root).as_posix().encode("utf-8"))
        result.update(b"\0")
        result.update(path.read_bytes())
        result.update(b"\0")
    return result.hexdigest()


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(value, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def paths(target: Path) -> tuple[Path, Path]:
    suffix = ".exe" if os.name == "nt" else ""
    return target / "release" / f"worldc{suffix}", target / "worldc-build.json"


def build(target: Path) -> int:
    """One explicit build; no authoring command calls this automatically."""
    target.mkdir(parents=True, exist_ok=True)
    binary, receipt_path = paths(target)
    receipt_path.unlink(missing_ok=True)
    # Resolve new workspace packages before binding the lockfile to this build.
    subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    before = source_identity()
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0", CARGO_PROFILE_RELEASE_DEBUG="0")
    subprocess.run(["cargo", "build", "--locked", "--release", "--package", "hex_world_tool", "--bin", "worldc"],
                   cwd=ROOT, env=env, check=True)
    after = source_identity()
    if before != after:
        raise ValueError("compiler inputs changed during build; run build again after edits finish")
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()
    atomic_json(receipt_path, {"version": 1, "source_sha256": after, "binary_sha256": file_hash(binary),
                              "binary": str(binary.resolve()), "git_head": revision})
    print(f"Built and identified {binary}")
    return 0


def checked_binary(target: Path) -> tuple[Path, dict]:
    binary, receipt_path = paths(target)
    if not receipt_path.is_file() or not binary.is_file():
        raise ValueError("worldc has no verified build; run python3 tools/world.py build first")
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    if receipt.get("version") != 1 or receipt.get("binary") != str(binary.resolve()):
        raise ValueError("build receipt belongs to a different binary or format")
    if receipt.get("source_sha256") != source_identity():
        raise ValueError("compiler code or dependencies changed; rebuild worldc before publishing maps")
    if receipt.get("binary_sha256") != file_hash(binary):
        raise ValueError("worldc binary differs from its build receipt; rebuild before using it")
    return binary, receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target" / "v4-authoring")
    parser.add_argument("command", choices=("build", "validate", "compile", "preview", "inspect", "probe", "benchmark", "edit-benchmark", "runtime-benchmark", "replication-benchmark"))
    args, trailing = parser.parse_known_args(argv)
    target = args.target_dir.resolve()
    try:
        if args.command == "build":
            if trailing:
                raise ValueError(f"unknown build arguments: {trailing}")
            return build(target)
        binary, receipt = checked_binary(target)
        before = receipt["source_sha256"]
        result = subprocess.run([str(binary), args.command, *trailing], cwd=ROOT)
        if result.returncode:
            return result.returncode
        if source_identity() != before or file_hash(binary) != receipt["binary_sha256"]:
            raise ValueError("compiler changed during the command; resulting output is not accepted")
        if args.command == "compile":
            try:
                output_index = trailing.index("--output") + 1
                output = Path(trailing[output_index])
            except (ValueError, IndexError) as error:
                raise ValueError("successful compile did not identify its output") from error
            if not output.is_absolute():
                output = ROOT / output
            atomic_json(output / "compiler-identity.json", receipt)
        elif args.command in ("benchmark", "edit-benchmark", "runtime-benchmark", "replication-benchmark"):
            output_index = trailing.index("--output") + 1
            output = Path(trailing[output_index])
            if not output.is_absolute():
                output = ROOT / output
            # Bind measurements to the actual executed compiler, not HEAD alone.
            # The measured receipt remains intact and is itself checksummed.
            atomic_json(output.with_suffix(output.suffix + ".identity.json"), {
                "version": 1, "compiler": receipt, "receipt_sha256": file_hash(output),
                "command": [args.command, *trailing],
            })
        return 0
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"world: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
