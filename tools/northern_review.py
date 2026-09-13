#!/usr/bin/env python3
"""Capture the six Northern Archipelago views through Cargo without a native window.

A clean, committed candidate produces a fresh .context/northern-review/<HEAD>/
pack. --dirty-diagnostic permits explicitly unapprovable scratch evidence.
Mechanical completion never grants static review or native-motion approval.

Example:
  python3 tools/northern_review.py --package /absolute/northern-package \
      --target-dir /absolute/cargo-target --label checkpoint-01

Use --dry-run to inspect the exact command and output path without launching or
writing artifacts. --view may be repeated for a clearly scoped diagnostic matrix.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
from pathlib import Path
import sys
import time

import arena
from v4_review import atomic_json, file_record, png_coverage

ROOT = Path(__file__).resolve().parents[1]
VIEWS = (
    "northern-overview", "northern-bay", "northern-settlement",
    "northern-summit", "northern-waterline", "northern-underwater",
)
EXTRA_VIEWS = ("northern-boat",)
MATRIX = "northern-six-v1"
CRITERIA = {
    "northern-boat": "Synthetic admitted-water B deployment: the player, voxel hull, sail and compact sailing HUD are readable; this does not establish travel or native feel.",
    "northern-overview": "All three clusters and the complete finite footprint have visible margins; distant island silhouettes remain coherent.",
    "northern-bay": "Unequal rocky bay arms, pale sand pocket, wooded ledges and a continuous sea boundary remain legible.",
    "northern-settlement": "Longhouse, cottages, shed and field have readable scale, supported foundations and sheltered woodland.",
    "northern-summit": "The dominant snowy crater ridge reads as irregular connected terrain with varied gray stone.",
    "northern-waterline": "Close blue water meets actual banks without floating sheets, cracks or invented streaming walls.",
    "northern-underwater": "The same waterline has a closed volume and colored underwater attenuation while HUD text stays readable.",
}
MOTION_ROUTE = (
    "At the bay watch several slow swells; fly to another cluster and reverse; "
    "walk the settlement, enter/leave water, and toggle F/G. Check wave motion, "
    "transparency, streaming pop, collision and control feel. User-owned native check."
)
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
ERRORS = re.compile(
    r"\bERROR\b|panicked at|Path not found|Cannot render (?:invalid )?authored object|"
    r"(?:Shader|shader).*(?:failed|invalid|error)|Arena capture failed"
)


def package_state(directory: Path) -> dict:
    """Hash every runtime byte without loading the complete fine world into memory."""
    if directory.is_symlink() or not directory.is_dir():
        raise RuntimeError("--package must be a real compiled package directory.")
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Package contains a symbolic link: {path}")
        if path.is_file():
            files[path.relative_to(directory).as_posix()] = file_record(path)
    for required in ("manifest.ron", "northern-overview.ron", "compile-receipt.json"):
        if required not in files:
            raise RuntimeError(f"Northern package lacks {required}.")
    receipt = json.loads((directory / "compile-receipt.json").read_text())
    if (receipt.get("strict") is not True
            or type(receipt.get("package_fingerprint")) is not int
            or type(receipt.get("source_fingerprint")) is not int
            or not receipt.get("world_id")):
        raise RuntimeError("Northern capture requires a strict compiler receipt with source/package identities.")
    return {"directory": str(directory), "compiler_receipt": receipt, "files": files}


def metadata_unchanged(package: dict) -> bool:
    """Cheap between-frame check; full hashes are checked again before completion."""
    root = Path(package["directory"])
    paths = {p.relative_to(root).as_posix(): p for p in root.rglob("*") if p.is_file() or p.is_symlink()}
    if paths.keys() != package["files"].keys():
        return False
    for name, path in paths.items():
        if path.is_symlink():
            return False
        info = path.stat()
        record = package["files"][name]
        if (info.st_size, info.st_mtime_ns, info.st_ctime_ns) != (
                record["bytes"], record["mtime_ns"], record["ctime_ns"]):
            return False
    return True


def validate_native(path: Path, view: str, package: dict) -> dict:
    state = json.loads(path.read_text())
    expected = package["compiler_receipt"]
    if state.get("view") != view or [state.get("width"), state.get("height")] != arena.CANVAS:
        raise RuntimeError(f"{view}: wrong view or logical canvas in native receipt.")
    if state.get("selection", {}).get("map") != "Northern Archipelago":
        raise RuntimeError(f"{view}: native capture selected the wrong map.")
    identity = state.get("package_identity") or {}
    if (identity.get("world_id") != expected["world_id"]
            or identity.get("manifest_fingerprint") != expected["package_fingerprint"]):
        raise RuntimeError(f"{view}: loaded package does not match the frozen compiler receipt.")
    if state.get("progress") is not None or state.get("expedition") is not None:
        raise RuntimeError(f"{view}: exploration unexpectedly has combat progression.")
    actors = state.get("actors", [])
    if (state.get("human_actor_id") != 0 or len(actors) != 1
            or actors[0].get("species") != "Human" or actors[0].get("hp", 0) <= 0
            or state.get("terminal_menu_outcome") is not None):
        raise RuntimeError(f"{view}: expected one living exploration player, no enemies and no terminal menu.")
    if view == "northern-boat" and not actors[0].get("boat", {}).get("active"):
        raise RuntimeError("Boat presentation fixture has not reached ordinary controller deployment.")
    ready = state.get("render_ready_frame")
    if type(ready) is not int or state.get("frame", 0) < ready + 4:
        raise RuntimeError(f"{view}: missing four settled render frames.")
    if state.get("liquid_phase_seconds") != 0.0:
        raise RuntimeError(f"{view}: liquid presentation was not frozen at phase zero.")
    camera = state.get("camera") or {}
    if not isinstance(camera.get("position"), list) or len(camera["position"]) != 3:
        raise RuntimeError(f"{view}: native camera pose missing.")
    return {"file": path.name, **file_record(path), "package_identity": identity,
            "frame": state["frame"], "tick": state.get("tick"), "camera": camera}


def scan_log(path: Path) -> list[str]:
    warnings = []
    with path.open(errors="replace") as stream:
        for line in stream:
            clean = ANSI.sub("", line).strip()
            if ERRORS.search(clean):
                raise RuntimeError(f"Capture log reports an error: {clean[:400]}")
            if re.search(r"\bWARN\b|^warning:", clean) and len(warnings) < 12:
                warnings.append(clean[:400])
    return warnings


def write_index(pack: Path, receipt: dict) -> None:
    lines = [f"# {receipt['source_label']}: Northern Archipelago", "",
             f"Mechanical status: **{receipt['mechanical_status']}**. Static review: **UNREVIEWED**.",
             "Native motion: **HUMAN-MOTION-PENDING**. These are external composition cameras.",
             "", "Inspect each original at full resolution, then an independently reviewed contact sheet.",
             "The six-frame set is the requested checkpoint; it does not replace native first/third-person movement or additional seam azimuths.",
             "", "| View | Capture | Criterion | Review |", "| --- | --- | --- | --- |"]
    for row in receipt["frames"]:
        image = f"[{row['view']}]({row['view']}.png)" if (pack / f"{row['view']}.png").is_file() else "Not captured"
        lines.append(f"| {row['view']} | {image} | {CRITERIA[row['view']]} | UNREVIEWED |")
    lines += ["", MOTION_ROUTE, "", "See receipt.json for exact source, package, commands and hashes.", ""]
    (pack / "review-index.md").write_text("\n".join(lines))


def capture(args: argparse.Namespace) -> int:
    initial, staged, unstaged = arena.source_state()
    if initial["dirty"] and not args.dirty_diagnostic:
        raise RuntimeError("Commit the candidate before capture, or explicitly use --dirty-diagnostic for UNAPPROVABLE-DIRTY scratch.")
    views = tuple(view for view in (*VIEWS, *EXTRA_VIEWS) if view in args.view) if args.view else VIEWS
    state_id = initial["head"]
    if args.dirty_diagnostic:
        state_id += "-UNAPPROVABLE-DIRTY-" + initial["state_sha256"][:12]
    selection = "" if views == VIEWS else "-focused-" + hashlib.sha256(",".join(views).encode()).hexdigest()[:8]
    pack = ROOT / ".context" / "northern-review" / state_id / f"{MATRIX}{selection}-{args.label}"
    if pack.exists() or pack.is_symlink():
        raise RuntimeError(f"Refusing to overwrite existing evidence: {pack}. Choose a new --label.")
    if args.dry_run:
        print(json.dumps({"output": str(pack), "source": initial,
                          "package": str(args.package), "views": views,
                          "command": ["cargo", *arena.CARGO_ARGS], "windowless": True}, indent=2))
        return 0
    package = package_state(args.package)
    env, removed = arena.environment(args.target_dir)
    env.pop("HEX_FOREST_WORLD", None)
    env.update(HEX_NORTHERN_WORLD=str(args.package), HEX_ARENA_MAP="northern-archipelago")
    pack.mkdir(parents=True, exist_ok=False)
    (pack / "staged.patch").write_bytes(staged)
    (pack / "unstaged.patch").write_bytes(unstaged)
    atomic_json(pack / "source-state.json", initial)
    atomic_json(pack / "package-state.json", package)
    receipt = {
        "schema_version": 1, "matrix": MATRIX + selection, "repository": str(ROOT),
        "pack": str(pack), "source": initial, "package": package,
        "source_label": "UNAPPROVABLE-DIRTY" if args.dirty_diagnostic else "COMMITTED-CANDIDATE",
        "started_at": arena.utc_now(), "mechanical_status": "INCOMPLETE",
        "static_review": "UNREVIEWED", "human_motion": "HUMAN-MOTION-PENDING",
        "capture_method": "Cargo --arena; HEX_ARENA_CAPTURE disables Winit and selects an image-target ScheduleRunner",
        "authored_contract": "docs/planning/waves/northern-archipelago/manifest.md",
        "composition": "Three distant clusters; eleven islands; a dominant snowy dormant crater; sheltered timber settlement; blue ocean with slow visual swells.",
        "camera_note": "Six external composition cameras, not native walking/first-person/third-person evidence.",
        "requested_wave_phase_seconds": 0.0, "configured_sun_elevation_degrees": 18.0,
        "logical_canvas": arena.CANVAS, "device_scale": 1.0,
        "expected_views": views, "human_route": MOTION_ROUTE,
        "gameplay_evidence": "Native typed receipts identify state; pixels establish static presentation only. No timing/FPS/motion claim.",
        "inherited_capability_names_removed": removed,
        "environment": {key: value for key, value in env.items() if key.startswith("HEX_") or key in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS")},
        "frames": [],
    }
    print(f"Windowless Northern capture pack: {pack}", flush=True)
    atomic_json(pack / "receipt.json", receipt)
    write_index(pack, receipt)
    try:
        seen = {}
        for view in views:
            if arena.source_state()[0] != initial or not metadata_unchanged(package):
                raise RuntimeError("Source or package changed during capture; this pack is stale.")
            png = pack / f"{view}.png"
            log = pack / f"{view}.log"
            frame_env = dict(env, HEX_ARENA_CAPTURE=str(png), HEX_ARENA_VIEW=view)
            row = {"view": view, "started_at": arena.utc_now(), "mechanical_status": "INCOMPLETE",
                   "static_review": "UNREVIEWED", "criterion": CRITERIA[view], "log": log.name,
                   "command": ["cargo", *arena.CARGO_ARGS], "cwd": str(ROOT),
                   "capabilities": {key: value for key, value in frame_env.items() if key.startswith("HEX_")}}
            receipt["frames"].append(row)
            atomic_json(pack / "receipt.json", receipt)
            print(f"Capturing {view}…", flush=True)
            started = time.monotonic()
            row["exit_code"] = arena.run_cargo(frame_env, log, args.timeout)
            row["wall_seconds"] = time.monotonic() - started
            row["warnings"] = scan_log(log)
            if row["exit_code"]:
                raise RuntimeError(f"{view} exited with {row['exit_code']}; see {log}.")
            row.update(arena.png_info(png))
            row["coverage"] = png_coverage(png, tuple(arena.CANVAS))
            row["native_state_receipt"] = validate_native(png.with_suffix(".json"), view, package)
            if row["sha256"] in seen:
                raise RuntimeError(f"Unexpected identical captures: {view} and {seen[row['sha256']]}.")
            seen[row["sha256"]] = view
            row.update(mechanical_status="CAPTURED", finished_at=arena.utc_now())
            atomic_json(pack / "receipt.json", receipt)
            write_index(pack, receipt)
        receipt["source_after"] = arena.source_state()[0]
        if receipt["source_after"] != initial or package_state(args.package) != package:
            raise RuntimeError("Source or package changed while capturing; this pack is stale.")
        receipt["mechanical_status"] = "COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        receipt["mechanical_status"] = "BLOCKED"
        receipt["error"] = str(error) or "Interrupted"
        raise
    finally:
        receipt["finished_at"] = arena.utc_now()
        for row in receipt["frames"]:
            row["artifacts"] = {suffix: file_record(pack / f"{row['view']}.{suffix}")
                                for suffix in ("png", "json", "log")
                                if (pack / f"{row['view']}.{suffix}").is_file()}
            atomic_json(pack / f"{row['view']}.receipt.json", row)
        atomic_json(pack / "receipt.json", receipt)
        write_index(pack, receipt)
    print("Capture complete. Static review UNREVIEWED; native motion HUMAN-MOTION-PENDING.", flush=True)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path, help="Explicit compiled Northern package directory.")
    parser.add_argument("--target-dir", required=True, type=Path, help="Explicit shared Cargo target directory.")
    parser.add_argument("--label", default="checkpoint-01", help="New matrix label; existing output is never reused.")
    parser.add_argument("--timeout", type=float, default=900.0, help="Seconds per frame including Cargo work; no native fallback.")
    parser.add_argument("--dirty-diagnostic", action="store_true", help="Permit explicitly UNAPPROVABLE-DIRTY scratch evidence.")
    parser.add_argument("--dry-run", action="store_true", help="Print source/commands/output without launching or writing.")
    parser.add_argument("--view", action="append", choices=(*VIEWS, *EXTRA_VIEWS), help="Explicit focused subset; repeat as needed. Default: all six.")
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,79}", args.label):
        parser.error("--label must be a short filename-safe identifier")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    for field in ("package", "target_dir"):
        path = getattr(args, field).expanduser()
        if not path.is_absolute():
            parser.error(f"--{field.replace('_', '-')} must be absolute")
        setattr(args, field, path.resolve())
    if args.target_dir.is_relative_to(args.package):
        parser.error("Cargo target must not be inside the immutable package")
    try:
        return capture(args)
    except (Exception, KeyboardInterrupt) as error:
        print(f"Northern capture BLOCKED: {error or 'Interrupted'}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
