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
import grand_package
from northern_review import metadata_unchanged, scan_log
from v4_review import atomic_json, file_record, png_coverage

ROOT = Path(__file__).resolve().parents[1]
VIEWS = ("grand-overview", "grand-mainland", "grand-mainland-south", "grand-mainland-east", "grand-mainland-northwest",
         "grand-crystal-frozen", "grand-valley-tree-bank", "grand-valley-lake-bank",
         "grand-valley-waterfall-approach",
         "grand-west-foothill-crossing", "grand-west-foothill-uphill",
         "grand-lake-foothill-crossing", "grand-lake-foothill-uphill",
         "grand-garden", "grand-garden-ground", "grand-waterfall",
         "grand-valley-lake", "grand-world-tree", "grand-roots-entrance", "grand-shrine-plant",
         "grand-forest", "grand-forest-ground", "grand-forest-ground-reverse", "grand-river-exit",
         "grand-island-landing", "grand-summit", "grand-shrine-air", "grand-crystal", "grand-shrine-earth",
         "grand-frozen-woods", "grand-volcano", "grand-shrine-fire", "grand-bay",
         "grand-bay-baseline", "grand-bay-reverse", "grand-waterline", "grand-underwater",
         "grand-waterfall-cave", "grand-library", "grand-library-reverse", "grand-library-upper",
         "grand-shadow-tunnel", "grand-shadow-reverse", "grand-shadow-exit", "first", "third", "start")
MOTION_ROUTE = (
    "Start at the beach; enter water, deploy/steer/fold the boat, sail to Fire; "
    "walk off both riverbanks into the hills and through the forest in both directions; "
    "cross the western and lake-front foothills laterally and uphill; "
    "land on the island and walk its ascent; return to the mainland and follow the river "
    "to the lakeside world tree and waterfall cave; walk both library branches and summit. "
    "Return to the separate Shadow approach, climb Crystal Ascent and follow Frozen Woods "
    "to the hidden mountain lake and garden island; glide down. "
    "Activate shrines, defeat Shadow, test valid and refused teleport, die/respawn. "
    "Save/restart on land, sailing and airborne; complete a 30-minute circuit and inspect seams both ways."
)


def cargo_arguments(profile: str, *, test_support: bool = False) -> tuple[str, ...]:
    """CI omits inspector/dylib features; default development captures stay unchanged."""
    if profile not in ("dev", "ci"):
        raise RuntimeError("Unknown Grand capture Cargo profile")
    features = "dev,arena-prototype" if profile == "dev" else "arena-prototype"
    if test_support:
        features += ",test-support"
    return ("run", "--profile", profile, "-p", "hex_game", "--features", features, "--", "--arena")


def package_state(directory: Path, *, plain: bool = False) -> dict:
    if directory.is_symlink() or not directory.is_dir():
        raise RuntimeError("Package must be a real compiled directory")
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Package contains a symlink: {path}")
        if path.is_file():
            files[path.relative_to(directory).as_posix()] = file_record(path)
    for required in ("manifest.ron", "grand-overview.ron", "arena-sites.ron", "grand-biomes.ron", "compile-receipt.json", "authoring-identity.json"):
        if required not in files:
            raise RuntimeError(f"Grand package lacks {required}")
    identity = json.loads((directory / "authoring-identity.json").read_text())
    expected_signature = grand_package.signature(ROOT / "assets/config/v4/grand-v4/world.ron") + ("-plain" if plain else "-dressed")
    if identity.get("plain") is not plain or identity.get("signature") != expected_signature:
        raise RuntimeError("Package authoring identity is stale or differs from requested dressing mode; compile current source to a fresh directory")
    if identity.get("compiler_mode") != "cargo-current-source":
        raise RuntimeError("Package lacks current-source compiler provenance; rebuild through Cargo before capture")
    receipt = json.loads((directory / "compile-receipt.json").read_text())
    measurement = json.loads((ROOT / "assets/config/v4/grand-v4/measurement.json").read_text())
    geography = json.loads((ROOT / "assets/config/v4/grand-v4/geography-r02.json").read_text())
    validate_area(receipt, measurement, geography)
    return {"directory": str(directory), "compiler_receipt": receipt,
            "authoring_identity": identity, "files": files}



def validate_area(receipt: dict, measurement: dict, geography: dict) -> None:
    """Require the emitted feature, including documented hex-lattice rounding.

    A package's own canonical counts cannot authorize its area. Bind them to the
    retained V3 measurement and the current authored outer polygon instead.
    """
    crystal = geography["ascent"]["expected_columns"]
    canonical = measurement["canonical_crystal_columns"]
    ratio = receipt.get("crystal_area_ratio")
    expected = {
        "world_id": "grand-v4",
        "mainland_columns": measurement["mainland_target_columns"],
        "canonical_mainland_columns": measurement["canonical_mainland_columns"],
        "canonical_crystal_columns": canonical,
        "crystal_target_columns": measurement["crystal_target_columns"],
        "crystal_columns": crystal,
        "crystal_authored_columns": crystal,
        "crystal_footprint_basis": "authored_outer_hex",
    }
    if (receipt.get("strict") is not True
            or any(receipt.get(key) != value for key, value in expected.items())
            or type(ratio) not in (int, float) or not math.isfinite(ratio)
            or not math.isclose(ratio, crystal / canonical, rel_tol=1e-12)):
        raise RuntimeError("Package does not establish the Grand area contract")


def matrix_contract(selected: list[str] | None) -> dict:
    requested = list(VIEWS if selected is None else selected)
    if not requested or len(requested) != len(set(requested)) or not set(requested) <= set(VIEWS):
        raise RuntimeError("Capture views must be a nonempty, unique subset of the declared matrix")
    full = set(requested) == set(VIEWS)
    return {"matrix": "grand-v4-composition-r02", "matrix_scope": "FULL" if full else "FOCUSED-DIAGNOSTIC",
            "expected_views": list(VIEWS), "requested_views": requested,
            "completed_views": [], "missing_views": list(VIEWS), "full_matrix_completed": False}


def update_coverage(receipt: dict) -> None:
    receipt["completed_views"] = [row["view"] for row in receipt["frames"]
                                  if row.get("mechanical_status") == "CAPTURED"]
    receipt["missing_views"] = [view for view in receipt["expected_views"]
                                if view not in receipt["completed_views"]]
    receipt["full_matrix_completed"] = (
        receipt.get("mechanical_status") == "COMPLETE"
        and receipt["matrix_scope"] == "FULL" and not receipt["missing_views"]
        and len(receipt["completed_views"]) == len(receipt["expected_views"]))


def complete_matrix(receipt: dict) -> None:
    update_coverage(receipt)
    if receipt["completed_views"] != receipt["requested_views"]:
        raise RuntimeError("Requested matrix views were not all captured exactly once")
    receipt["mechanical_status"] = "COMPLETE" if receipt["matrix_scope"] == "FULL" else "PARTIAL_COMPLETE"
    update_coverage(receipt)


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
    matrix = matrix_contract(args.view)
    views = matrix["requested_views"]
    label = source["head"] + ("-dirty-" + source["state_sha256"][:12] if source["dirty"] else "")
    pack = ROOT / ".context/grand-review" / label / args.label
    if pack.exists():
        raise RuntimeError(f"Evidence exists already: {pack}")
    package = package_state(args.package, plain=args.plain)
    command = cargo_arguments(args.cargo_profile)
    env, removed = arena.environment(args.target_dir)
    env.update(HEX_GRAND_WORLD=str(args.package), HEX_ARENA_MAP="grand-v4")
    pack.mkdir(parents=True)
    (pack / "staged.patch").write_bytes(staged)
    (pack / "unstaged.patch").write_bytes(unstaged)
    atomic_json(pack / "package-state.json", package)
    receipt = {"source": source, "package": package, **matrix,
               "content_stage": "PLAIN-TERRAIN" if args.plain else "DRESSED-WORLD",
               "cargo_profile": args.cargo_profile, "command": ["cargo", *command],
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
            row = {"view": view, "started_at": arena.utc_now(), "command": ["cargo", *command],
                   "static_review": "UNREVIEWED", "log": log.name}
            receipt["frames"].append(row)
            atomic_json(pack / "receipt.json", receipt)
            started = time.monotonic()
            print(f"Capturing {view}…", flush=True)
            row["exit_code"] = arena.run_cargo(frame_env, log, args.timeout, args=command)
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
        if arena.source_state()[0] != source or package_state(args.package, plain=args.plain) != package:
            raise RuntimeError("Source/package changed during capture")
        complete_matrix(receipt)
    except (Exception, KeyboardInterrupt) as error:
        receipt.update(mechanical_status="BLOCKED", error=str(error) or "Interrupted")
        raise
    finally:
        update_coverage(receipt)
        receipt["finished_at"] = arena.utc_now()
        atomic_json(pack / "receipt.json", receipt)
        rows = [f"# Grand V4 — {label}", "", f"Mechanical: {receipt['mechanical_status']}. Static: UNREVIEWED.",
                f"Content stage: {receipt['content_stage']}.",
                f"Matrix: {receipt['matrix_scope']}; {len(receipt['completed_views'])}/{len(VIEWS)} full-matrix views captured.",
                "Full-matrix presentation approval is unavailable for focused subsets." if receipt["matrix_scope"] != "FULL" else "Full matrix captured only after all declared views complete; independent static review is still required.",
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
    parser.add_argument("--plain", action="store_true", help="Explicitly admit a current plain-terrain package for geometry review")
    parser.add_argument("--cargo-profile", choices=("dev", "ci"), default="dev")
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
