#!/usr/bin/env python3
"""Source-based, windowless Water Lab captures with bounded evidence and provenance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
VIEWS = ("overview", "shore", "reverse", "channel", "swim-first", "swim-third", "boat-first", "glider-third", "controls")


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def identity():
    changes = git("diff", "--binary", "HEAD")
    for name in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0"):
        if name:
            changes += name + (ROOT / os.fsdecode(name)).read_bytes()
    return {"head": git("rev-parse", "HEAD").decode().strip(), "dirty_digest": hashlib.sha256(changes).hexdigest(), "dirty": bool(git("status", "--porcelain"))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--view", action="append", choices=(*VIEWS, "motion", "motion-reverse"))
    parser.add_argument("--wave", choices=("flat", "regular", "swell", "crossing"), default="regular")
    parser.add_argument("--style", choices=("depth", "crests", "patterns"), default="patterns")
    parser.add_argument("--wind", choices=("calm", "steady", "strong", "gusts", "turning", "shelter"), default="steady")
    parser.add_argument("--phase", type=float, default=1.5)
    parser.add_argument("--glider-wind", type=float, choices=(1.0, 0.65, 0.45), default=1.0)
    parser.add_argument("--dirty-diagnostic", action="store_true")
    args = parser.parse_args()
    source = identity()
    if source["dirty"] and not args.dirty_diagnostic:
        parser.error("Commit the candidate first, or label scratch captures with --dirty-diagnostic.")
    output = args.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error("Use a fresh empty evidence directory.")
    output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    for key in tuple(env):
        if key.startswith(("HEX_ARENA_", "HEX_WATER_LAB_")):
            env.pop(key)
    env.update(CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2", CARGO_TARGET_DIR=str(args.target_dir.resolve()),
               HEX_ARENA_MAP="water-lab", HEX_WATER_LAB_WAVE=args.wave,
               HEX_WATER_LAB_STYLE=args.style, HEX_WATER_LAB_WIND=args.wind,
               HEX_WATER_LAB_GLIDER_WIND=str(args.glider_wind))
    manifest = {"source": source, "worktree": str(ROOT), "status": "INCOMPLETE", "views": [],
                "review": "UNREVIEWED; native control feel and user taste pending"}
    manifest_path = output / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2))
    for view in args.view or VIEWS:
        if identity() != source:
            raise RuntimeError("Source changed during capture; this pack is stale.")
        if view.startswith("motion"):
            env.pop("HEX_WATER_LAB_PHASE", None)
        else:
            env["HEX_WATER_LAB_PHASE"] = str(args.phase)
        env.update(HEX_ARENA_CAPTURE=str(output / f"{view}.png"), HEX_ARENA_VIEW=f"water-lab-{view}")
        command = ["cargo", "run", "-p", "hex_game", "--features", "dev,arena-prototype", "--", "--arena"]
        started = time.monotonic()
        print(f"Windowless Water Lab: {view}", flush=True)
        with (output / f"{view}.log").open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=900)
        log_text = (output / f"{view}.log").read_text()
        if result.returncode or "Path not found" in log_text or "panicked at" in log_text:
            raise RuntimeError(f"{view} failed; inspect {view}.log")
        paths = sorted(output.glob(f"{view}-[0-9][0-9].png")) if view.startswith("motion") else [output / f"{view}.png"]
        if len(paths) != (24 if view.startswith("motion") else 1):
            raise RuntimeError(f"{view}: incomplete image sequence")
        hashes = []
        for path in paths:
            receipt = json.loads(path.with_suffix(".json").read_text())
            lab = receipt.get("water_lab", receipt.get("lab", {}))
            if not lab.get("enabled") or lab.get("wave", "").lower() != args.wave:
                raise RuntimeError(f"{view}: wrong actual map or preset")
            hashes.append({"file": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        if len(paths) > 1 and len({row["sha256"] for row in hashes}) != len(paths):
            raise RuntimeError(f"{view}: unexpected identical motion frames")
        intervals = receipt.get("app_frame_wall_intervals_ms", [])[10:]
        row = {"view": view, "command": command, "elapsed_seconds": time.monotonic() - started, "files": hashes,
               "actual": lab, "render_ready_ms": receipt.get("app_construction_to_render_ready_ms")}
        if intervals:
            row["app_frame_median_ms"] = statistics.median(intervals)
            row["app_frame_p95_ms"] = sorted(intervals)[int((len(intervals) - 1) * 0.95)]
        manifest["views"].append(row)
        manifest_path.write_text(json.dumps(manifest, indent=2))
    if identity() != source:
        raise RuntimeError("Source changed during capture; this pack is stale.")
    manifest["status"] = "CAPTURED_UNREVIEWED" if not source["dirty"] else "DIRTY_DIAGNOSTIC_UNREVIEWED"
    manifest_path.write_text(json.dumps(manifest, indent=2))
    print(manifest_path, flush=True)


if __name__ == "__main__":
    main()
