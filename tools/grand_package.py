#!/usr/bin/env python3
"""Compile immutable full-scale Grand packages using the production chunk writer."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def authoring_files(source: Path) -> list[Path]:
    """The source and its named geographic companion travel as one input."""
    files = [source]
    match = re.search(r'\bgeography\s*:\s*Some\(\s*"([^"\\]+)"\s*\)', source.read_text())
    if match:
        name = match.group(1)
        if Path(name).name != name or not name.endswith(".json"):
            raise RuntimeError("Grand geography must name a sibling JSON file")
        geography = source.parent / name
        if not geography.is_file():
            raise RuntimeError(f"Missing Grand geography companion: {geography}")
        files.append(geography)
    return files


def signature(source: Path) -> str:
    paths = [*authoring_files(source),
             *sorted((ROOT / "crates/hex_schematic/src/v4/grand").rglob("*.rs")),
             *sorted((ROOT / "crates/hex_schematic/src/v4/northern").rglob("*.rs")),
             ROOT / "crates/hex_world_tool/src/grand.rs",
             ROOT / "assets/config/v4/grand-v4/forest/trees.ron"]
    digest = hashlib.sha256()
    for path in paths:
        data = path.read_bytes()
        # Length framing prevents concatenation ambiguity without tying packages
        # to a checkout's absolute path.
        digest.update(len(data).to_bytes(8, "little"))
        digest.update(data)
    return digest.hexdigest()


def package_signature(source: Path, *, plain: bool = False,
                      presentation_sample: str | None = None) -> str:
    if presentation_sample not in (None, "crystal-four"):
        raise RuntimeError("Unsupported Grand presentation sample")
    value = signature(source) + ("-plain" if plain else "-dressed")
    if presentation_sample is not None:
        value += "-presentation-sample-" + presentation_sample
    return value


def stage_plain_source(source: Path, directory: Path) -> Path:
    for companion in authoring_files(source)[1:]:
        shutil.copyfile(companion, directory / companion.name)
    text, count = re.subn(r"\bfull_dressing\s*:\s*(?:true|false)\b",
                          "full_dressing:false", source.read_text())
    if count != 1:
        raise RuntimeError("Expected one explicit full_dressing field in Grand source")
    staged = directory / source.name
    staged.write_text(text)
    return staged


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", nargs="?", choices=["compile", "ensure"], default="compile")
    parser.add_argument("--source", type=Path, default=ROOT / "assets/config/v4/grand-v4/world.ron")
    parser.add_argument("--output", type=Path, default=ROOT / "assets/config/v4/grand-v4/compiled")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/v4-authoring")
    parser.add_argument("--cargo-profile", choices=["dev", "ci"], default="dev")
    parser.add_argument("--plain", action="store_true", help="Full terrain, caves and water without object dressing")
    parser.add_argument("--presentation-sample", choices=("crystal-four",),
                        help="Explicit bounded terrain presentation diagnostic; not ordinary publication")
    parser.add_argument("--worldc", type=Path, help="Prebuilt compiler; output remains unverified")
    args = parser.parse_args()
    source = args.source.resolve()
    sig = package_signature(source, plain=args.plain, presentation_sample=args.presentation_sample)
    if args.output.exists():
        stamp = args.output / "authoring-identity.json"
        required = ["manifest.ron", "grand-overview.ron", "arena-sites.ron", "grand-biomes.ron"]
        if (args.mode == "ensure" and all((args.output / name).is_file() for name in required)
                and stamp.is_file() and json.loads(stamp.read_text()).get("signature") == sig):
            return 0
        parser.error("Output exists or is stale. Use a fresh --output; immutable packages are never overwritten.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    inputs = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in authoring_files(source)}
    with tempfile.TemporaryDirectory(prefix="grand-source-", dir=args.output.parent) as temporary:
        if args.plain:
            source = stage_plain_source(source, Path(temporary))
        command = ([str(args.worldc.resolve())] if args.worldc else
                   ["cargo", "run", "--profile", args.cargo_profile, "-p", "hex_world_tool", "--bin", "worldc", "--"])
        env = dict(os.environ, CARGO_TARGET_DIR=str(args.target_dir.resolve()),
                   CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2")
        compiler_args = ["grand-compile", "--source", str(source), "--output", str(args.output.resolve())]
        if args.presentation_sample is not None:
            compiler_args += ["--presentation-sample", args.presentation_sample]
        result = subprocess.call(command + compiler_args, cwd=ROOT, env=env)
        if result:
            return result
    identity = {
        "signature": sig, "plain": args.plain, "authoring_files": inputs,
        "compiler_mode": "prebuilt-unverified" if args.worldc else "cargo-current-source",
        "cargo_profile": None if args.worldc else args.cargo_profile}
    if args.presentation_sample is not None:
        identity["presentation_sample"] = args.presentation_sample
    (args.output / "authoring-identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
