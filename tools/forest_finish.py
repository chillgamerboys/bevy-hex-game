#!/usr/bin/env python3
"""Compose the expedition from a public terrain survey, exact assets and source.

The artifact stage writes additive catalog entries and blueprints in this checkout.
After committing those assets, source_revision binds the ordinary V4 source to
that reviewed artifact commit. Compilation and survey remain worldc's authority.
"""
from __future__ import annotations

import argparse
from collections import defaultdict
import json
from pathlib import Path
import random
import re
import subprocess

from forest_world import ROOT, Raw, ron, mapping, hexpos
from forest_expedition import SEED, distance, disk, recipe, source_document
from forest_placement import PlacementWorld, TreeLibrary, place_forest
from forest_structures import catalog, placement, bridge_assembly, arena_assembly, STYLE_MATERIALS
from forest_trees import STYLES as TREE_STYLES


def structures(survey, metadata):
    """Surveyed architecture plus distributed, collision-checked rock/crystal sites."""
    library = catalog(raw=Raw)
    world = PlacementWorld(survey, metadata)
    route_columns = [(q, r) for route in metadata["routes"].values() for q, r, _ in route["ribbon"]]
    props = bridge_assembly(world.surface, raw=Raw, clear_columns=route_columns)
    arena = metadata["arena"]
    props += arena_assembly(metadata["anchors"]["mountain_shadow"], arena["walls"], arena["gate"],
                            arena["wall_top"], world.surface, raw=Raw,
                            clear_columns=route_columns)
    for dto in props:
        world.reserve_structure(dto)
    for name, pool in metadata["fountains"].items():
        q, r, level = pool["center"]
        dto = placement(library["shapes"]["fountain-rim"], f"{name}-rim", (q, r, level - 2), raw=Raw,
                        clear_columns=[(q + dq, r - 3) for dq in range(4)])
        world.reserve_structure(dto)
        props.append(dto)

    forest = set(map(tuple, metadata["forest"]["columns"]))
    world.root_exclusion.update(disk(metadata["forest"]["heart"]["center"], 28))
    for tree in metadata["forest"]["landmarks"]:
        world.root_exclusion.update(disk(tree["center"], tree["clearing_radius"]))
    eligible = set(map(tuple, metadata["forest"]["understory_root_columns"]))
    rng = random.Random(SEED + 123)
    forest_candidates = sorted(eligible)
    mountain_candidates = sorted(p for p, level in world.surface.items()
                                if p not in forest and distance(p) < 172 and p[0]+p[1]/2 > 34 and 40 <= level <= 240)
    for candidates, label, amount in ((forest_candidates, "forest-rock", 32),
                                      (forest_candidates, "forest-crystal", 24),
                                      (mountain_candidates, "mountain-rock", 16),
                                      (mountain_candidates, "mountain-crystal", 12)):
        candidates = candidates.copy()
        rng.shuffle(candidates)
        count = 0
        variants = ("slab", "pillar", "ridge", "arch") if "rock" in label else ("cluster", "needle", "fan")
        for root in candidates:
            shape = library["shapes"][f'{label.split("-")[1]}-{variants[count % len(variants)]}']
            turn = rng.randrange(6)
            first = placement(shape, f"{label}-{count+1:02}", (*root, world.surface[root]), raw=Raw, rotation=turn)
            contacts = first["foundation_levels"]
            if any(p not in world.surface or p in world.wet or p in world.protected or p in world.root_exclusion for p in contacts):
                continue
            heights = sorted(world.surface[p] for p in contacts)
            if not heights or heights[-1] - heights[0] > 6:
                continue
            ground = heights[len(heights)//2]
            dto = placement(shape, first["id"], (*root, ground), raw=Raw, rotation=turn)
            try:
                world.reserve_structure(dto)
            except ValueError:
                continue
            props.append(dto)
            world.root_exclusion.update(disk(root, 7))
            count += 1
            if count == amount:
                break
        if count != amount:
            raise ValueError(f"only {count}/{amount} {label} formations fit")
    return props, library


def compose(survey):
    value, metadata = recipe(raw=Raw)
    props, prop_library = structures(survey, metadata)
    trees = TreeLibrary(Raw)
    world, report = place_forest(survey, metadata, trees, props)
    outputs = {}
    assets = {}
    for name, (blueprint, intervals) in trees.documents.items():
        assets[blueprint["id"]] = (blueprint, intervals, TREE_STYLES)
    for dto in props:
        asset = (dto["blueprint"], dto["intervals"], STYLE_MATERIALS)
        if dto["asset"] in assets and assets[dto["asset"]] != asset:
            raise ValueError(f'asset identity collision: {dto["asset"]}')
        assets[dto["asset"]] = asset
    for asset, (blueprint, _, _) in assets.items():
        outputs[f"assets/art/objects/{asset}.ron"] = (
            "// Authored by tools/forest_finish.py; exact voxel geometry, .35 units/level.\n" + ron(blueprint) + "\n")
    for q, r, level in report["foundations"]:
        value["overrides"].append({"id": f"expedition-footing-{q}-{r}", "mask": {"center": hexpos(q,r), "radius": 0},
                                   "surface_level": Raw(f"Some({level})"), "material": None})
    report["structure_count"] = len(props)
    report["structure_counts"] = {prefix: sum(p["id"].startswith(prefix) for p in props)
                                   for prefix in ("bridge", "arena", "forest_fountain", "mountain_fountain",
                                                  "forest-rock", "forest-crystal", "mountain-rock", "mountain-crystal")}
    report["structures"] = [{key: val for key, val in p.items() if key in (
        "id", "asset", "root", "support_level", "desired_rotation", "occupied_world", "clear_columns")}
        | {"ground_contacts": [(*at, level) for at, level in sorted(p["foundation_levels"].items())]} for p in props]
    return value, metadata, assets, outputs, report, prop_library["styles"]


def final_source(value, report, assets, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("source revision must be the explicit committed artifact SHA")
    groups = defaultdict(list)
    for tree in report["placements"]:
        groups[tree["asset"], tree["rotation"]].append(tree["root"])
    for prop in report["structures"]:
        groups[prop["asset"], prop["desired_rotation"]].append(prop["root"])
    for (asset, turn), roots in sorted(groups.items()):
        _, intervals, styles = assets[asset]
        path = f"assets/art/objects/{asset}.ron"
        value["features"].append({"id": f'{asset.replace("/", "-")}-r{turn}',
            "kind": "tree" if asset.startswith("plant/") else "structure", "asset": asset,
            "provenance": Raw("Some(" + ron({"source_path": path, "source_revision": revision,
                                              "style_materials": mapping(styles)}) + ")"),
            "mask": {"center": hexpos(0,0), "radius": 187}, "density": 0,
            "roots": [hexpos(*p) for p in sorted(roots)], "rotation": Raw(f"Some({turn})"),
            "overhead_clearance": Raw("Some(4)"), "voxels": intervals})
    return source_document(value, raw=Raw, ron=ron, extra_materials=[
        {"id": "timber", "solid": True, "diggable": True, "color": Raw("(100,67,41,255)")},
        {"id": "foliage", "solid": True, "diggable": True, "color": Raw("(53,110,66,255)")}])



def merge_catalog_ids(source, additions):
    """Preserve the public manifest while maintaining its strict ID-set order."""
    match = re.search(r"\bobjects\s*:\s*\[(.*?)\]", source, re.DOTALL)
    if match is None:
        raise ValueError("object catalog has no objects list")
    # This authored field contains only quoted strings and a trailing comma.
    # Read that JSON-compatible list, not arbitrary RON or private package data.
    ids = json.loads("[" + match.group(1).rstrip().removesuffix(",") + "]")
    if any(not isinstance(asset, str) for asset in ids):
        raise ValueError("object catalog entries must be string IDs")
    body = "\n" + "".join(f"        {ron(asset)},\n" for asset in sorted(set(ids) | set(additions))) + "    "
    return source[:match.start(1)] + body + source[match.end(1):]


def write_assets(outputs, styles):
    for path, text in sorted(outputs.items()):
        target = ROOT / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
    path = ROOT / "assets/art/object_catalog.ron"
    original = path.read_text()
    additions = [p.removeprefix("assets/art/objects/").removesuffix(".ron") for p in outputs]
    path.write_text(merge_catalog_ids(original, additions))
    path = ROOT / "assets/art/voxel_styles.ron"
    original = path.read_text()
    additions = {name: style for name, style in styles.items() if f'"{name}":' not in original}
    if additions:
        path.write_text(original.replace("    },\n)", "".join(f'        {ron(name)}:{ron(style)},\n' for name, style in sorted(additions.items())) + "    },\n)"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--survey", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--write-assets", action="store_true")
    parser.add_argument("--source-revision")
    args = parser.parse_args()
    survey = json.loads(args.survey.read_text())
    value, metadata, assets, outputs, report, styles = compose(survey)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "authoring.json").write_text(json.dumps(metadata, separators=(",", ":")) + "\n")
    (args.out_dir / "placement.json").write_text(json.dumps(report, separators=(",", ":")) + "\n")
    if args.write_assets:
        write_assets(outputs, styles)
    if args.source_revision:
        for path, expected in outputs.items():
            committed = subprocess.run(["git", "show", f"{args.source_revision}:{path}"], cwd=ROOT,
                                       check=True, capture_output=True, text=True).stdout
            if committed != expected:
                raise ValueError(f"committed artifact differs: {path}")
        (args.out_dir / "world.ron").write_text(final_source(value, report, assets, args.source_revision))
    print(json.dumps({key: report[key] for key in ("tree_count", "understory", "mountain_trees", "structure_count", "canopy_fraction")}))


if __name__ == "__main__":
    main()
