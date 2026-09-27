#!/usr/bin/env python3
"""Measure original Grand V3 ownership and derive Grand V4's area targets.

The largest six-neighbor mainland component includes inland lakes and their garden
island, but excludes open sea and offshore islands. This tool never authors terrain:
revision02 geography and its compiled receipt own the actual new coastline.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DIRECTIONS = ((1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1))


def distance(q: int, r: int) -> int:
    return max(abs(q), abs(r), abs(q + r))


def measure() -> dict:
    source = ROOT / "assets/config/schematics/grand-v3-template.ron"
    raw = source.read_text()
    cells = {}
    pattern = (r"\(id:(\d+),coord:\(q:(-?\d+),r:(-?\d+),s:-?\d+\),"
               r"facts:\(surface:(\w+),landform:(\w+),.*?overlays:\[([^]]*)\]")
    for match in re.finditer(pattern, raw):
        identity, q, r, surface, landform, overlays = match.groups()
        inland = any(name in overlays for name in ("MountainLake", "ValleyLake"))
        land = surface == "Land" and (landform != "Island" or "LakeIsland" in overlays)
        cells[int(q), int(r)] = int(identity), land or inland
    if len(cells) != 217:
        raise RuntimeError("Original Grand V3 source no longer has the expected 217 cells")
    mainland = set()
    offsets = [(q, r) for q in range(-2, 3) for r in range(-2, 3) if distance(q, r) <= 2]
    for q in range(-187, 188):
        for r in range(max(-187, -187 - q), min(187, 187 - q) + 1):
            cq, cr = q // 22, r // 22
            candidates = [
                (distance(q - (cq + dq) * 22, r - (cr + dr) * 22), *cells[cq + dq, cr + dr])
                for dq, dr in offsets if (cq + dq, cr + dr) in cells
            ]
            if not candidates:
                candidates = [(distance(q - a * 22, r - b * 22), *value)
                              for (a, b), value in cells.items()]
            if min(candidates)[2]:
                mainland.add((q, r))
    components = []
    remaining = set(mainland)
    while remaining:
        todo = [remaining.pop()]
        for q, r in todo:
            for dq, dr in DIRECTIONS:
                neighbor = q + dq, r + dr
                if neighbor in remaining:
                    remaining.remove(neighbor)
                    todo.append(neighbor)
        components.append(len(todo))
    components.sort(reverse=True)
    canonical = components[0]
    return {
        "scope": "Canonical Grand V3 measurement and production area targets; not current terrain geometry",
        "canonical_source": str(source.relative_to(ROOT)),
        "canonical_sha256": hashlib.sha256(raw.encode()).hexdigest(),
        "fine_radius": 187,
        "coarse_pitch": 22,
        "full_disk_columns": 1 + 3 * 187 * 188,
        "canonical_mainland_columns": canonical,
        "other_included_component_columns": components[1:],
        "mainland_target_columns": canonical * 7,
        "target_area_ratio": 7,
        "canonical_crystal_radius": 32,
        "canonical_crystal_columns": 1 + 3 * 32 * 33,
        "crystal_target_columns": (1 + 3 * 32 * 33) * 7,
        "hex_column_area": 3 * math.sqrt(3) / 2,
        "production_geometry": "geography-r02.json; actual area is recorded in each immutable package's compile-receipt.json",
    }


def write_measurement(receipt: dict) -> None:
    destination = ROOT / "assets/config/v4/grand-v4"
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "measurement.json").write_text(json.dumps(receipt, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="Update measurement.json only; preserve authored geography")
    arguments = parser.parse_args()
    receipt = measure()
    if arguments.write:
        write_measurement(receipt)
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
