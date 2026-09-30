#!/usr/bin/env python3
"""Regenerate/check the authored Forest–Massif map, then compile using verified worldc.

This offline authoring helper emits ordinary V4 RON and exact catalog blueprints.
It does not participate in runtime generation or scale render-only geometry.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
from pathlib import Path
import random
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CONTENT = ROOT / "assets/config/v4/forest-massif"
SOURCE = CONTENT / "world.ron"
LEVEL_HEIGHT = .35
BLUEPRINT_SOURCE_REV = "9233a82e9a80aa32c4d2856dde6c021b89006245"
SEED = 20260911
RADIUS = 187
GIANT = (-143, 5)
ANCHORS = {
    "party_start": (-94, 125, 40), "hostile_start": (-109, 100, 40),
    "forest_outer_a": (-109, 100, 40), "forest_outer_b": (-72, 84, 40),
    "forest_middle": (-92, 18, 40), "forest_deep_a": (-130, 55, 40),
    "forest_deep_b": (-110, -25, 40), "ancient_tree": (-132, 4, 40),
    "bridge_west": (-24, 0, 48), "bridge_east": (24, 0, 48),
    "dragon_lower": (68, 25, 80), "dragon_middle": (118, -25, 130),
    "dragon_upper": (113, -77, 190),
}
STYLES = {"plant/trunk": "timber", "plant/foliage-dark": "foliage",
          "plant/foliage-mid": "foliage", "plant/foliage-light": "foliage"}


class Raw(str):
    """An explicitly constructed RON enum/option/tuple, never user text."""


def ron(value):
    if isinstance(value, Raw):
        return str(value)
    if value is None:
        return "None"
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, dict):
        return "(" + ",".join(f"{key}:{ron(item)}" for key, item in value.items()) + ")"
    if isinstance(value, list):
        return "[\n" + ",\n".join(ron(item) for item in value) + ",\n]" if value else "[]"
    return str(value)


def mapping(value):
    return Raw("{" + ",".join(f"{ron(key)}:{ron(item)}" for key, item in value.items()) + "}")


def hexpos(q, r):
    return {"q": q, "r": r}


def dist(a, b=(0, 0)):
    q, r = a[0] - b[0], a[1] - b[1]
    return max(abs(q), abs(r), abs(q + r))


def worlddist(a, b):
    q, r = a[0] - b[0], a[1] - b[1]
    return math.sqrt(3 * (q * q + q * r + r * r))


def disk(center, radius):
    return {"center": hexpos(*center), "radius": radius}


def grade(point):
    return {"column": hexpos(*point[:2]), "level": point[2]}


def tree(name, height, radius, pine=False):
    """Connected solid voxel crowns and branches over a walkable open understory."""
    voxels = {}
    trunk_radius = 2 if name.endswith("heart") else (1 if height >= 80 else 0)
    crown_depth = 13 if height >= 80 else (9 if pine else 5)
    crown_start = height - crown_depth
    for q in range(-radius, radius + 1):
        for r in range(-radius, radius + 1):
            d = dist((q, r))
            if d <= trunk_radius:
                for level in range(crown_start + crown_depth // 2):
                    voxels[q, r, level] = ("plant/trunk", "Root" if level == 0 else "Trunk")
            if d > radius:
                continue
            # Layered crown tiers keep the silhouette authored and legible.
            for level in range(crown_start, height):
                t = (level - crown_start) / max(1, crown_depth - 1)
                width = radius * (1 - t * .8) if pine else radius * math.sqrt(max(0, 1 - (2 * t - 1) ** 2))
                if d <= max(.1, width):
                    color = "dark" if t < .3 else ("mid" if t < .72 else "light")
                    voxels.setdefault((q, r, level), (f"plant/foliage-{color}", "Foliage"))
    assert len(voxels) <= 65536, (name, len(voxels))
    placements = [{"position": {"q": q, "r": r, "level": level}, "style": style,
                   "part": Raw(f"Plant({part})")} for (q, r, level), (style, part) in sorted(voxels.items())]
    blueprint = {"schema_version": 1, "id": f"plant/{name}",
                 "display_name": name.replace("forest-", "").replace("-", " ").title(),
                 "category": Raw("Plant"), "bounds": {"radius": radius, "min_level": 0, "height": height},
                 "connectivity": Raw("Grounded"), "origin": {"q": 0, "r": 0, "level": 0},
                 "placements": placements,
                 "blocker_footprint": [hexpos(q, r) for (q, r, level), (_, part) in sorted(voxels.items()) if part == "Root"],
                 "canopy_occluders": [{"q": q, "r": r, "level": level} for (q, r, level), (_, part) in sorted(voxels.items()) if part == "Foliage"]}
    # Exact source provenance and matching occupied intervals, not a visual approximation.
    intervals = []
    columns = {}
    for (q, r, level), (style, _) in sorted(voxels.items()):
        columns.setdefault((q, r), []).append((level, STYLES[style]))
    for (q, r), entries in columns.items():
        for level, material in entries:
            if intervals and intervals[-1]["offset"] == hexpos(q, r) and intervals[-1]["top"] == level and intervals[-1]["material"] == material:
                intervals[-1]["top"] += 1
            else:
                intervals.append({"offset": hexpos(q, r), "bottom": level, "top": level + 1, "material": material})
    return blueprint, intervals


def route(name, points, width=0, material="gravel"):
    return {"id": name, "points": [grade(p) for p in points], "half_width": width,
            "shoulder_width": 6, "material": material}


def point_segment_distance(point, start, end):
    def xy(p):
        return (math.sqrt(3) * (p[0] + p[1] / 2), 1.5 * p[1])
    x, y = xy(point)
    ax, ay = xy(start)
    bx, by = xy(end)
    t = max(0, min(1, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / ((bx - ax) ** 2 + (by - ay) ** 2)))
    return math.hypot(x - ax - t * (bx - ax), y - ay - t * (by - ay))


def documents():
    trees = {}
    for band, heights, radius in [("broadleaf", (18, 23, 28), 4), ("pine", (40, 51, 62), 7), ("ancient", (80, 97, 114), 13)]:
        for index, height in enumerate(heights):
            name = f"forest-{band}-{index + 1}"
            trees[name] = tree(name, height, radius, pine=band == "pine")
    trees["forest-heart"] = tree("forest-heart", 172, 24)
    forest_routes = [route("forest-spine", [ANCHORS[key] for key in ("party_start", "forest_outer_a", "forest_middle", "forest_deep_a")], 2, "moss"),
                     route("outer-clearing-trail", [ANCHORS["forest_outer_a"], ANCHORS["forest_outer_b"]], 2, "grass"),
                     route("ancient-trail", [ANCHORS["forest_middle"], ANCHORS["forest_deep_b"], (-125, -5, 40)], 2, "moss"),
                     route("west-bank-trail", [ANCHORS["forest_outer_a"], (-58, 0, 40)], 2, "grass")]
    routes = forest_routes + [route("bridge-west-approach", [(-54, 0, 40), ANCHORS["bridge_west"]], 3),
        route("massif-lower-ascent", [ANCHORS["bridge_east"], (56, 25, 80)]),
        route("massif-lower-shelf", [(56, 25, 80), (80, 25, 80)], material="limestone"),
        route("massif-middle-ascent", [(80, 25, 80), (106, -25, 130)]),
        route("massif-middle-shelf", [(106, -25, 130), (130, -25, 130)], material="limestone"),
        route("massif-upper-ascent", [(130, -25, 130), (162, -59, 160), (125, -77, 190)]),
        route("massif-upper-shelf", [(125, -77, 190), (101, -77, 190)], material="limestone")]
    # Trunks are frozen authoring inputs. Color overlapping crown footprints
    # into disjoint height bands; no two objects occupy the same voxel.
    layout = json.loads((CONTENT / "trunk-layout.json").read_text())
    roots = {name: [] for name in trees}
    for band, radius in [("broadleaf", 4), ("pine", 7), ("ancient", 13)]:
        points = sorted(tuple(p) for p in layout[band])
        adjacent = {a: {b for b in points if a != b and dist(a, b) <= radius * 2} for a in points}
        colors = {}
        while len(colors) < len(points):
            point = max((p for p in points if p not in colors), key=lambda p: (
                len({colors[n] for n in adjacent[p] if n in colors}), len(adjacent[p]), p))
            used = {colors[n] for n in adjacent[point] if n in colors}
            colors[point] = next(c for c in range(3) if c not in used)
        if band == "ancient":
            colors[min(points)] = 2  # Keep the tallest ordinary ancient variant present.
        for point, color in colors.items():
            roots[f"forest-{band}-{color + 1}"].append(point)
    roots["forest-heart"] = [GIANT]
    selected = [(p, {"broadleaf": 10, "pine": 18, "ancient": 30, "heart": 48}[name.split("-")[1]], name)
                for name, positions in roots.items() for p in positions]
    features = []
    outputs = {}
    for name, (blueprint, intervals) in trees.items():
        path = f"assets/art/objects/plant/{name}.ron"
        encoded = "// Authored by tools/forest_world.py; exact voxel geometry, .35 units/level.\n" + ron(blueprint) + "\n"
        outputs[ROOT / path] = encoded
        features.append({"id": name, "kind": "tree", "asset": f"plant/{name}",
                         "provenance": Raw("Some(" + ron({"source_path": path, "source_revision": BLUEPRINT_SOURCE_REV, "style_materials": mapping(STYLES)}) + ")"),
                         "mask": disk((0, 0), 187), "density": 0,
                         "roots": [hexpos(*p) for p in sorted(roots[name])], "overhead_clearance": Raw("Some(8)"), "voxels": intervals})
    colors = {"bedrock": (33, 38, 42, 255), "basalt": (76, 88, 101, 255), "soil": (98, 76, 49, 255),
              "grass": (105, 141, 72, 255), "moss": (49, 91, 57, 255), "pine-floor": (78, 98, 60, 255),
              "sand": (160, 155, 113, 255), "snow": (219, 231, 235, 255), "gravel": (126, 131, 120, 255),
              "water": (42, 115, 144, 190), "timber": (100, 67, 41, 255), "foliage": (53, 110, 66, 255), "limestone": (167, 170, 158, 255), "stone": (105, 112, 116, 255)}
    materials = [{"id": name, "solid": name != "water", "diggable": name not in ("bedrock", "water"),
                  "color": Raw(str(color))} for name, color in colors.items()]
    recipe = {"base_level": 40,
              "strata": {"bedrock": "bedrock", "rock": "basalt", "soil": "soil", "soil_depth": 4, "surface": "grass"},
              "landforms": [
                  {"id": "massif-body", "centers": [hexpos(120, -55)], "radius": 83, "plateau_radius": 12, "rise": 210, "relief": 7},
                  {"id": "massif-crown", "centers": [hexpos(132, -93), hexpos(143, -101), hexpos(149, -86)], "radius": 42, "plateau_radius": 3, "rise": 132, "relief": 9},
                  {"id": "dragon-lower-buttress", "centers": [hexpos(68, 25)], "radius": 34, "plateau_radius": 12, "rise": 40, "relief": 0},
                  {"id": "north-companion", "centers": [hexpos(108, -136), hexpos(113, -137)], "radius": 33, "plateau_radius": 2, "rise": 85, "relief": 6},
                  {"id": "east-companion", "centers": [hexpos(164, -46)], "radius": 22, "plateau_radius": 1, "rise": 95, "relief": 5},
                  {"id": "southeast-companion", "centers": [hexpos(127, 36)], "radius": 22, "plateau_radius": 2, "rise": 85, "relief": 7},
                  {"id": "south-companion", "centers": [hexpos(55, 100), hexpos(61, 94)], "radius": 30, "plateau_radius": 2, "rise": 105, "relief": 8},
                  {"id": "western-ravine", "centers": [hexpos(96, -65), hexpos(103, -77)], "radius": 24, "plateau_radius": 0, "rise": -45, "relief": 2},
                  {"id": "crown-cleft", "centers": [hexpos(147, -103)], "radius": 21, "plateau_radius": 0, "rise": -35, "relief": 3}],
              "biomes": [
                  {"id": "pine-uplands", "mask": disk((-83, 25), 60), "priority": 2, "material": "pine-floor"},
                  {"id": "ancient-mosswood", "mask": disk((-132, 30), 45), "priority": 3, "material": "moss"},
                  {"id": "ancient-fernwood", "mask": disk((-123, -9), 35), "priority": 3, "material": "moss"},
                  {"id": "massif-stone", "mask": disk((100, -38), 83), "priority": 4, "material": "basalt"},
                  {"id": "massif-snow", "mask": disk((132, -93), 42), "priority": 5, "material": "snow"},
                  {"id": "north-stone", "mask": disk((108, -136), 33), "priority": 4, "material": "basalt"},
                  {"id": "east-stone", "mask": disk((164, -46), 22), "priority": 4, "material": "basalt"},
                  {"id": "southeast-stone", "mask": disk((127, 36), 22), "priority": 4, "material": "basalt"},
                  {"id": "south-stone", "mask": disk((55, 100), 30), "priority": 4, "material": "basalt"}],
              "channels": [{"id": "great-river", "points": [grade((87, -174, 34)), grade((-87, 174, 34))],
                            "half_width": 13, "depth": 12, "material": "water", "bed_material": "sand", "bank_width": 6}],
              "routes": routes,
              "bridges": [{"id": "central-crossing", "points": [grade(ANCHORS["bridge_west"]), grade(ANCHORS["bridge_east"])],
                           "half_width": 3, "thickness": 2, "material": "stone"}],
              "features": features,
              "overrides": [{"id": f"clearing-{key}", "mask": disk(value[:2], 9 if key.startswith("dragon") else 7),
                             "surface_level": Raw(f"Some({value[2]})"), "material": Raw('Some("limestone")') if key.startswith("dragon") else None}
                            for key, value in ANCHORS.items() if key.startswith(("forest_", "dragon_"))],
              "anchors": [{"id": key, "column": hexpos(*value[:2]), "level": Raw(f"Some({value[2]})"),
                           "role": Raw("Observation" if key == "ancient_tree" else "Gameplay")} for key, value in ANCHORS.items()],
              "hub": grade(ANCHORS["party_start"])}
    spec = {"version": 1, "id": "forest-massif-battle", "seed": SEED, "materials": materials,
            "recipes": mapping({"forest-massif": recipe}),
            "regions": [{"id": "forest", "recipe": "forest-massif", "origin": hexpos(0, 0), "radius": RADIUS, "rotation": 0}], "connections": []}
    outputs[SOURCE] = "// Forest–Massif Battle. Rebuild with python3 tools/forest_world.py generate.\n" + ron(spec) + "\n"
    report = {"radius": RADIUS, "columns": 1 + 3 * RADIUS * (RADIUS + 1), "level_height": LEVEL_HEIGHT,
              "anchors": {f"forest/anchor/{key}": value for key, value in ANCHORS.items()},
              "tree_count": len(selected), "trees": [{"asset": f"plant/{name}", "height_units": tree_data[0]["bounds"]["height"] * LEVEL_HEIGHT,
                    "voxel_count": len(tree_data[0]["placements"]), "roots": sorted(roots[name])} for name, tree_data in trees.items()],
              "minimum_spacing_units": {"broadleaf": 10, "pine": 18, "ancient": 30, "heart": 48}}
    mask = {(q, r) for q in range(-171, 20) for r in range(-122, 163)
            if dist((q, r)) <= 171 and q + r / 2 < -28}
    canopy = set()
    occupied = {}
    for name, (blueprint, _) in trees.items():
        for root in roots[name]:
            for placement in blueprint["placements"]:
                p = placement["position"]
                # Crowns/cores are sixfold symmetric; compiler root rotations
                # preserve this occupied-column footprint and interval schedule.
                position = (root[0] + p["q"], root[1] + p["r"], p["level"])
                if position in occupied:
                    raise ValueError(f"overlapping exact object voxels: {position}, {name}, {occupied[position]}")
                occupied[position] = name
                if str(placement["part"]) == "Plant(Foliage)":
                    canopy.add(position[:2])
    covered = canopy & mask
    report["canopy_coverage"] = {
        "mask": {"q_min": -171, "q_max": 19, "r_min": -122, "r_max": 162,
                 "hex_radius": 171, "q_plus_half_r_exclusive_max": -28},
        "includes_camps_trails_and_clearings": True,
        "forest_land_columns": len(mask), "unique_canopy_columns": len(covered),
        "fraction": len(covered) / len(mask), "minimum_required_fraction": .5,
        "exact_object_voxels": len(occupied), "actual_voxel_overlaps": 0}
    western_land = {(q, r) for q in range(-RADIUS, RADIUS + 1)
                    for r in range(-RADIUS, RADIUS + 1)
                    if dist((q, r)) <= RADIUS and q + r / 2 < -28}
    report["canopy_coverage"]["all_western_land_secondary"] = {
        "mask": {"hex_radius": RADIUS, "q_plus_half_r_exclusive_max": -28},
        "land_columns": len(western_land),
        "unique_canopy_columns": len(canopy & western_land),
        "fraction": len(canopy & western_land) / len(western_land)}
    assert len(covered) >= len(mask) * .5, report["canopy_coverage"]
    outputs[CONTENT / "authoring.json"] = json.dumps(report, indent=2) + "\n"
    return outputs, report


def check():
    outputs, report = documents()
    for path, content in outputs.items():
        if not path.exists() or path.read_text() != content:
            raise ValueError(f"authored output differs: {path.relative_to(ROOT)}; run generate")
        if path.parent == ROOT / "assets/art/objects/plant":
            # Read the actual emitted bytes with an independent occupancy parser.
            positions = [tuple(map(int, row)) for row in re.findall(
                r"position:\(q:(-?\d+),r:(-?\d+),level:(\d+)\)", content)]
            occupied = set(positions)
            assert len(occupied) == len(positions) <= 65536, path.name
            reached, frontier = {(0, 0, 0)}, [(0, 0, 0)]
            steps = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0),
                     (1, -1, 0), (-1, 1, 0), (0, 0, 1), (0, 0, -1)]
            while frontier:
                p = frontier.pop()
                for step in steps:
                    neighbor = tuple(a + b for a, b in zip(p, step))
                    if neighbor in occupied and neighbor not in reached:
                        reached.add(neighbor)
                        frontier.append(neighbor)
            assert reached == occupied, f"ungrounded branch or crown: {path.name}"
            committed = subprocess.check_output(["git", "show", f"{BLUEPRINT_SOURCE_REV}:{path.relative_to(ROOT)}"], cwd=ROOT)
            assert committed.decode() == content, f"stock provenance changed: {path.name}"
    assert report["columns"] == 105469
    roots = [(tuple(p), row["asset"].split("/")[1]) for row in report["trees"] for p in row["roots"]]
    for index, (position, asset) in enumerate(roots):
        band = asset.split("-")[1]
        spacing = report["minimum_spacing_units"][band]
        for other, other_asset in roots[index + 1:]:
            required = max(spacing, report["minimum_spacing_units"][other_asset.split("-")[1]])
            assert worlddist(position, other) >= required, (position, other, required)
    print(f"Exact generated files and spacing verified: {report['tree_count']} trees; {report['columns']} terrain columns.")


def verify_package(target, package):
    """Inspect independently loaded runtime queries across the full river barrier."""
    import world
    binary, identity = world.checked_binary(target.resolve())
    authoring = json.loads((CONTENT / "authoring.json").read_text())
    tree_roots = {tuple(p) for tree in authoring["trees"] for p in tree["roots"]}
    points = sorted({point[:2] for point in ANCHORS.values()} |
                    {(round(-r / 2), r) for r in range(-187, 188)} | tree_roots)

    def probe(point):
        output = subprocess.check_output([str(binary), "probe", "--package", str(package.resolve()),
                                          "--at", f"{point[0]},{point[1]}"], cwd=ROOT)
        return point, json.loads(output)

    with ThreadPoolExecutor(max_workers=4) as executor:
        probes = dict(executor.map(probe, points))
    fingerprints = {value["package_fingerprint"] for value in probes.values()}
    assert len(fingerprints) == 1, "package changed during verification"
    for name, point in ANCHORS.items():
        assert any(surface["position"]["level"] == point[2] and
                   (surface["headroom"] is None or surface["headroom"] >= 2)
                   for surface in probes[point[:2]]["surfaces"]), f"anchor support absent: {name}"
    bridge_rows = []
    for r in range(-187, 188):
        value = probes[round(-r / 2), r]
        assert any(water["body_id"] == "forest/great-river" and water["top"] == 35 for water in value["liquids"]), f"river gap at {r}"
        dry = [s for s in value["surfaces"] if s["position"]["level"] >= 34]
        if dry:
            assert all(s["material"] == "stone" and s["position"]["level"] == 48 for s in dry), (r, dry)
            bridge_rows.append(r)
    assert bridge_rows == list(range(-3, 4)), bridge_rows
    heart = probes[GIANT]["root_objects"]
    assert len(heart) == 1 and heart[0]["asset"] == "plant/forest-heart"
    assert max(run["top"] for column in heart[0]["occupancy"] for run in column["runs"]) == 213
    objects = {obj["id"]: obj for p in tree_roots for obj in probes[p]["root_objects"]}
    assert len(objects) == authoring["tree_count"], "compiled tree identity count differs"
    canopy, occupied = set(), set()
    for obj in objects.values():
        for column in obj["occupancy"]:
            p = column["position"]
            for run in column["runs"]:
                cells = {(p["q"], p["r"], level) for level in range(run["bottom"], run["top"])}
                assert not occupied & cells, f"actual compiled object overlap: {obj['id']}"
                occupied.update(cells)
                if run["material"] == "foliage":
                    canopy.add((p["q"], p["r"]))
    primary = {(q, r) for q in range(-171, 20) for r in range(-122, 163)
               if dist((q, r)) <= 171 and q + r / 2 < -28}
    western = {(q, r) for q in range(-RADIUS, RADIUS + 1) for r in range(-RADIUS, RADIUS + 1)
               if dist((q, r)) <= RADIUS and q + r / 2 < -28}
    coverage = authoring["canopy_coverage"]
    assert len(canopy & primary) == coverage["unique_canopy_columns"]
    assert len(canopy & western) == coverage["all_western_land_secondary"]["unique_canopy_columns"]
    assert len(occupied) == coverage["exact_object_voxels"]
    assert len(canopy & primary) / len(primary) >= .5
    coverage_receipt = {"status": "PASS", "package_fingerprint": next(iter(fingerprints)),
                        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                        "compiled_objects": len(objects), "method": "Union of exact compiled foliage occupancy queried at every tree root; no summed overlap area.",
                        **coverage}
    (package / "canopy-verification.json").write_text(json.dumps(coverage_receipt, indent=2) + "\n")
    assert world.checked_binary(target.resolve())[1] == identity
    receipt = {"world_id": "forest-massif-battle", "package_fingerprint": next(iter(fingerprints)),
               "runtime_column_probes": len(probes), "river_rows": 375,
               "unique_bridge_rows": bridge_rows, "verified_named_anchors": list(ANCHORS),
               "giant_top_exclusive": 213, "compiler": identity,
               "scope": "Exact runtime geometry and authored assets; no aesthetic, motion, or performance approval."}
    (package / "content-verification.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"Verified {len(probes)} runtime columns: 375-row river barrier, one seven-row bridge, every named support, exact giant occupancy, and both exact canopy union domains.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "check", "compile", "validate", "verify-package"))
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/v4-authoring")
    parser.add_argument("--output", type=Path, default=CONTENT / "compiled")
    args = parser.parse_args()
    if args.command == "generate":
        outputs, report = documents()
        for path, content in outputs.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        print(f"Authored {report['tree_count']} trees in radius-{RADIUS} forest; wrote {len(outputs)} sources.")
        return 0
    check()
    if args.command == "verify-package":
        verify_package(args.target_dir, args.output)
        return 0
    if args.command in ("compile", "validate"):
        command = [sys.executable, str(ROOT / "tools/world.py"), "--target-dir", str(args.target_dir), args.command, "--source", str(SOURCE)]
        if args.command == "compile":
            command += ["--output", str(args.output)]
        return subprocess.call(command, cwd=ROOT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
