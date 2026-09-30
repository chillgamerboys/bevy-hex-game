#!/usr/bin/env python3
"""Bounded real-controller, windowless Grand temporal sequences; human feel stays pending."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import sys

import arena
import grand_capture
from northern_review import metadata_unchanged, scan_log
from v4_review import atomic_json, file_record, png_coverage

ROOT = Path(__file__).resolve().parents[1]
ROUTES = ("forest-forward", "forest-reverse", "river-forward", "river-reverse")
COUNT = 24


def finite_vector(value, size=3):
    return isinstance(value, list) and len(value) == size and all(
        isinstance(v, (float, int)) and math.isfinite(v) for v in value)


def validate_sequence(rows: list[dict], route: str, package_fingerprint: int) -> dict:
    if len(rows) != COUNT or [r.get("index") for r in rows] != list(range(COUNT)):
        raise RuntimeError("Temporal sequence needs all 24 ordered frames")
    for row in rows:
        identity = row.get("package_identity") or {}
        camera = row.get("camera") or {}
        if (row.get("route") != route or identity.get("world_id") != "grand-v4"
                or identity.get("manifest_fingerprint") != package_fingerprint):
            raise RuntimeError("Temporal frame has wrong route/package")
        if row.get("body_valid") is not True or row.get("relocations") != 1:
            raise RuntimeError("Temporal frame needs valid body and exactly one setup relocation")
        if (not finite_vector(row.get("feet")) or not finite_vector(row.get("eye"))
                or not finite_vector(row.get("body_dimensions"))
                or not finite_vector(camera.get("position")) or not finite_vector(camera.get("rotation"), 4)):
            raise RuntimeError("Temporal body/camera values must be finite")
        if row.get("camera_mode") != "ordinary-first-person":
            raise RuntimeError("Temporal sequence must use ordinary moving camera")
        for key in ("tick", "start_tick", "frame", "start_frame", "ocean_generation"):
            if type(row.get(key)) is not int:
                raise RuntimeError(f"Missing exact temporal integer: {key}")
        seconds = row.get("ocean_time_seconds")
        if not isinstance(seconds, (float, int)) or not math.isfinite(seconds):
            raise RuntimeError("Missing real simulation clock")
        if (not math.isclose(row.get("rendered_ocean_time_seconds", -1), seconds, abs_tol=1e-6)
                or not math.isclose(row.get("river_phase", -1), seconds % 4 / 4, abs_tol=1e-6)):
            raise RuntimeError("Rendered river/ocean clock is not the live saved clock")
        completed = row.get("readback_completion") or {}
        if (type(completed.get("frame")) is not int or type(completed.get("tick")) is not int
                or completed["frame"] < row["frame"] or completed["tick"] < row["tick"]):
            raise RuntimeError("Missing asynchronous readback completion")
        if (row["frame"] - row["start_frame"] > 900 or row["frame"] <= row["start_frame"]
                or row["tick"] <= row["start_tick"]):
            raise RuntimeError("Temporal route exceeded its bounded frame window")
    first, last = rows[0], rows[-1]
    for previous, current in zip(rows, rows[1:]):
        if (current["tick"] <= previous["tick"] or current["frame"] <= previous["frame"]
                or current["ocean_time_seconds"] <= previous["ocean_time_seconds"]):
            raise RuntimeError("Temporal ticks, frames and clock must advance monotonically")
        if any(current[k] != first[k] for k in ("start_tick", "start_frame", "start_feet", "ocean_generation", "body_dimensions", "direction")):
            raise RuntimeError("Temporal route setup/profile changed during movement")
        dt = current["ocean_time_seconds"] - previous["ocean_time_seconds"]
        if not math.isclose(dt, (current["tick"] - previous["tick"]) / 120, abs_tol=1e-5):
            raise RuntimeError("Temporal clock was relabelled without completed simulation ticks")
        distance = math.dist(current["feet"], previous["feet"])
        if distance < 0.15 or distance > dt * 12 + 1:
            raise RuntimeError("Temporal frames lack ordinary continuous movement")
        if math.dist(current["camera"]["position"], previous["camera"]["position"]) < 0.1:
            raise RuntimeError("Temporal camera did not follow the walking body")
    elapsed = last["ocean_time_seconds"] - first["ocean_time_seconds"]
    progress = last.get("progress_units", 0) - first.get("progress_units", 0)
    if elapsed < 4 or progress < 20:
        raise RuntimeError("Temporal sequence lacks a full river period or meaningful route progress")
    chunks = {tuple(row.get("chunk", ())) for row in rows}
    handoffs = set()
    in_view_handoffs = set()
    for previous, current in zip(rows, rows[1:]):
        handoffs |= set(previous.get("forest_proxy_visible", ())) & set(current.get("forest_proxy_hidden", ()))
        handoffs |= set(previous.get("forest_proxy_hidden", ())) & set(current.get("forest_proxy_visible", ()))
        in_view_handoffs |= set(previous.get("forest_proxy_in_view", ())) & set(current.get("forest_proxy_hidden", ()))
        in_view_handoffs |= set(previous.get("forest_proxy_hidden", ())) & set(current.get("forest_proxy_in_view", ()))
    if route.startswith("forest") and (len(chunks) < 2 or not handoffs):
        raise RuntimeError("Forest temporal route did not exercise a chunk boundary and actual proxy handoff")
    return {"simulation_seconds": elapsed, "forward_progress_units": progress,
            "crossed_chunks": len(chunks), "forest_handoff_objects": len(handoffs),
            "in_frustum_handoff_candidates": len(in_view_handoffs),
            "visible_handoff_confirmation": "PENDING_IMAGE_REVIEW",
            "sample_frequency_hz": (COUNT - 1) / elapsed,
            "evidence": "WINDOWLESS_TEMPORAL_SAMPLES; single-frame flicker, human control feel and taste pending"}


def capture(args):
    source, staged, unstaged = arena.source_state()
    if source["dirty"]:
        raise RuntimeError("Commit the composed candidate before temporal capture")
    package = grand_capture.package_state(args.package)
    pack = ROOT / ".context/grand-motion" / source["head"] / args.label
    if pack.exists():
        raise RuntimeError(f"Evidence already exists: {pack}")
    pack.mkdir(parents=True)
    (pack / "staged.patch").write_bytes(staged)
    (pack / "unstaged.patch").write_bytes(unstaged)
    command = grand_capture.cargo_arguments(args.cargo_profile, test_support=True)
    env, removed = arena.environment(args.target_dir)
    env.update(HEX_GRAND_WORLD=str(args.package), HEX_ARENA_MAP="grand-v4")
    receipt = {"source": source, "package": package, "routes": list(ROUTES), "sequences": [],
               "mechanical_status": "INCOMPLETE", "temporal_review": "UNREVIEWED",
               "human_control": "PENDING", "single_frame_flicker": "UNPROVEN",
               "canvas": arena.CANVAS, "requested_stride_ticks": 48,
               "capture_method": "continuously rendered windowless image target; asynchronous readback",
               "removed_inherited_capabilities": removed,
               "cargo_profile": args.cargo_profile, "command": ["cargo", *command]}
    try:
        for route in ROUTES:
            if source != arena.source_state()[0] or not metadata_unchanged(package):
                raise RuntimeError("Source/package changed during temporal capture")
            directory = pack / route
            directory.mkdir()
            frame_env = dict(env, HEX_ARENA_VIEW=f"grand-motion-{route}",
                             HEX_ARENA_CAPTURE=str(directory / "frame.png"),
                             HEX_GAME_DATA_DIR=str(directory / "disposable-user-data"))
            row = {"route": route, "status": "INCOMPLETE", "temporal_review": "UNREVIEWED"}
            receipt["sequences"].append(row)
            atomic_json(pack / "receipt.json", receipt)
            print(f"Grand windowless temporal capture: {route}", flush=True)
            log = directory / "capture.log"
            row["exit_code"] = arena.run_cargo(frame_env, log, args.timeout, args=command)
            row["warnings"] = scan_log(log)
            if row["exit_code"]:
                raise RuntimeError(f"{route}: native temporal capture failed; see {log}")
            files = sorted(directory.glob("frame-[0-9][0-9].png"))
            if len(files) != COUNT:
                raise RuntimeError(f"{route}: expected 24 PNGs")
            frames, hashes = [], set()
            native = []
            for path in files:
                image = arena.png_info(path)
                if image["physical_pixels"] != arena.CANVAS:
                    raise RuntimeError("Wrong temporal canvas")
                hashes.add(image["sha256"])
                state = path.with_suffix(".json")
                native.append(json.loads(state.read_text()))
                frames.append({"png": path.name, **image, "state": file_record(state),
                               "coverage": png_coverage(path, tuple(arena.CANVAS))})
            if len(hashes) != COUNT:
                raise RuntimeError("Repeated PNGs cannot establish temporal movement")
            row.update(status="COMPLETE", frames=frames,
                       measured=validate_sequence(native, route, package["compiler_receipt"]["package_fingerprint"]))
            atomic_json(pack / "receipt.json", receipt)
        if source != arena.source_state()[0] or package != grand_capture.package_state(args.package):
            raise RuntimeError("Source/package changed during temporal capture")
        receipt["mechanical_status"] = "COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        receipt.update(mechanical_status="BLOCKED", error=str(error) or "Interrupted")
        raise
    finally:
        atomic_json(pack / "receipt.json", receipt)
    print(f"Temporal images retained for independent review: {pack}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--target-dir", required=True, type=Path)
    parser.add_argument("--label", required=True)
    parser.add_argument("--cargo-profile", choices=("ci", "dev"), default="ci")
    parser.add_argument("--timeout", type=float, default=600)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,79}", args.label) or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("Invalid label or timeout")
    for name in ("package", "target_dir"):
        path = getattr(args, name)
        if not path.is_absolute():
            parser.error(f"--{name.replace('_','-')} must be absolute")
        setattr(args, name, path.resolve())
    try:
        capture(args)
        return 0
    except (Exception, KeyboardInterrupt) as error:
        print(f"Grand temporal capture BLOCKED: {error or 'Interrupted'}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
