#!/usr/bin/env python3
"""Windowless Grand V4 composition matrix with exact source/package provenance."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import sys
import time

import arena
from northern_review import metadata_unchanged, scan_log
from water_lab_review import atomic_json, file_record, png_coverage

ROOT = Path(__file__).resolve().parents[1]
VIEWS = ("grand-overview", "grand-mainland", "grand-garden", "grand-waterfall",
         "grand-valley-lake", "grand-world-tree", "grand-roots-entrance", "grand-shrine-plant",
         "grand-forest", "grand-summit", "grand-shrine-air", "grand-crystal", "grand-shrine-earth",
         "grand-frozen-woods", "grand-volcano", "grand-shrine-fire", "grand-bay",
         "grand-bay-baseline", "grand-bay-reverse", "grand-waterline", "grand-underwater",
         "grand-waterfall-cave", "grand-library", "grand-library-upper",
         "grand-shadow-tunnel", "grand-shadow-reverse", "first", "third", "start")
MOTION_ROUTE = (
    "Start at the beach; enter water, deploy/steer/fold the boat, sail to Fire; "
    "walk the river to the world tree and garden; follow water to the waterfall cave; "
    "walk both library branches, Shadow tunnel, Crystal Ascent and summit; glide down. "
    "Activate shrines, defeat Shadow, test valid and refused teleport, die/respawn. "
    "Save/restart on land, sailing and airborne; complete a 30-minute circuit and inspect seams both ways."
)


def package_state(directory: Path) -> dict:
    if directory.is_symlink() or not directory.is_dir():
        raise RuntimeError("Package must be a real compiled directory")
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Package contains a symlink: {path}")
        if path.is_file():
            files[path.relative_to(directory).as_posix()] = file_record(path)
    for required in ("manifest.ron", "grand-overview.ron", "arena-sites.ron", "grand-biomes.ron", "compile-receipt.json"):
        if required not in files:
            raise RuntimeError(f"Grand package lacks {required}")
    receipt = json.loads((directory / "compile-receipt.json").read_text())
    if (receipt.get("strict") is not True or receipt.get("world_id") != "grand-v4"
            or receipt.get("mainland_columns") != 7 * receipt.get("canonical_mainland_columns", 0)
            or receipt.get("crystal_columns") != 7 * receipt.get("canonical_crystal_columns", 0)):
        raise RuntimeError("Package does not establish the Grand area contract")
    return {"directory": str(directory), "compiler_receipt": receipt, "files": files}


def validate_native(path: Path, view: str, package: dict) -> dict:
    state = json.loads(path.read_text())
    identity = state.get("package_identity") or {}
    if (state.get("view") != view or state.get("selection", {}).get("map") != "Grand V4"
            or identity.get("manifest_fingerprint") != package["compiler_receipt"]["package_fingerprint"]
            or identity.get("world_id") != "grand-v4"):
        raise RuntimeError(f"{view}: native receipt has the wrong view/map/package")
    ready = state.get("render_ready_frame")
    if type(ready) is not int or state.get("frame", 0) < ready + 4:
        raise RuntimeError(f"{view}: render did not settle")
    if not state.get("camera", {}).get("position"):
        raise RuntimeError(f"{view}: missing camera")
    return {"file": path.name, **file_record(path), "package_identity": identity,
            "tick": state.get("tick"), "camera": state["camera"]}


def capture(args: argparse.Namespace) -> int:
    source, staged, unstaged = arena.source_state()
    if source["dirty"] and not args.dirty_diagnostic:
        raise RuntimeError("Commit the candidate first, or request --dirty-diagnostic scratch evidence")
    views = args.view or VIEWS
    label = source["head"] + ("-dirty-" + source["state_sha256"][:12] if source["dirty"] else "")
    pack = ROOT / ".context/grand-review" / label / args.label
    if pack.exists():
        raise RuntimeError(f"Evidence exists already: {pack}")
    package = package_state(args.package)
    env, removed = arena.environment(args.target_dir)
    env.update(HEX_GRAND_WORLD=str(args.package), HEX_ARENA_MAP="grand-v4")
    pack.mkdir(parents=True)
    (pack / "staged.patch").write_bytes(staged)
    (pack / "unstaged.patch").write_bytes(unstaged)
    atomic_json(pack / "package-state.json", package)
    receipt = {"source": source, "package": package, "matrix": "grand-v4-composition-v1",
               "source_label": "UNAPPROVABLE-DIRTY" if source["dirty"] else "COMMITTED-CANDIDATE",
               "static_review": "UNREVIEWED", "human_motion": "HUMAN-MOTION-PENDING",
               "motion_route": MOTION_ROUTE, "mechanical_status": "INCOMPLETE",
               "inherited_capability_names_removed": removed, "frames": []}
    print(f"Grand windowless evidence: {pack}", flush=True)
    try:
        hashes = set()
        for view in views:
            if arena.source_state()[0] != source or not metadata_unchanged(package):
                raise RuntimeError("Source/package changed during capture")
            png, log = pack / f"{view}.png", pack / f"{view}.log"
            frame_env = dict(env, HEX_ARENA_CAPTURE=str(png), HEX_ARENA_VIEW=view,
                             HEX_ARENA_CAPTURE_SETTLE_FRAMES=str(args.settle_frames),
                             HEX_GAME_DATA_DIR=str(pack / "disposable-user-data"))
            row = {"view": view, "started_at": arena.utc_now(), "command": ["cargo", *arena.CARGO_ARGS],
                   "static_review": "UNREVIEWED", "log": log.name}
            receipt["frames"].append(row)
            atomic_json(pack / "receipt.json", receipt)
            started = time.monotonic()
            print(f"Capturing {view}…", flush=True)
            row["exit_code"] = arena.run_cargo(frame_env, log, args.timeout)
            row["wall_seconds"] = time.monotonic() - started
            row["warnings"] = scan_log(log)
            if row["exit_code"]:
                raise RuntimeError(f"{view}: capture failed; see {log}")
            row.update(arena.png_info(png))
            row["coverage"] = png_coverage(png, tuple(arena.CANVAS))
            row["native_state"] = validate_native(png.with_suffix(".json"), view, package)
            if row["sha256"] in hashes:
                raise RuntimeError(f"Duplicate frame at {view}")
            hashes.add(row["sha256"])
            row["mechanical_status"] = "CAPTURED"
            atomic_json(pack / "receipt.json", receipt)
        if arena.source_state()[0] != source or package_state(args.package) != package:
            raise RuntimeError("Source/package changed during capture")
        receipt["mechanical_status"] = "COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        receipt.update(mechanical_status="BLOCKED", error=str(error) or "Interrupted")
        raise
    finally:
        receipt["finished_at"] = arena.utc_now()
        atomic_json(pack / "receipt.json", receipt)
        rows = [f"# Grand V4 — {label}", "", f"Mechanical: {receipt['mechanical_status']}. Static: UNREVIEWED.",
                "Native movement: HUMAN-MOTION-PENDING.", "", MOTION_ROUTE, ""]
        rows += [f"- [{row['view']}]({row['view']}.png): UNREVIEWED" for row in receipt["frames"]]
        (pack / "review-index.md").write_text("\n".join(rows) + "\n")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--target-dir", type=Path, default=arena.DEFAULT_TARGET)
    parser.add_argument("--label", required=True)
    parser.add_argument("--view", choices=VIEWS, action="append")
    parser.add_argument("--dirty-diagnostic", action="store_true")
    parser.add_argument("--timeout", type=float, default=900)
    parser.add_argument("--settle-frames", type=int, default=4)
    args = parser.parse_args()
    if (not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,79}", args.label)
            or not math.isfinite(args.timeout) or args.timeout <= 0
            or not 4 <= args.settle_frames <= 600):
        parser.error("Invalid label, timeout or settle frames")
    for name in ("package", "target_dir"):
        path = getattr(args, name)
        if not path.is_absolute():
            parser.error(f"--{name.replace('_', '-')} must be absolute")
        setattr(args, name, path.resolve())
    try:
        return capture(args)
    except (Exception, KeyboardInterrupt) as error:
        print(f"Grand capture BLOCKED: {error or 'Interrupted'}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
