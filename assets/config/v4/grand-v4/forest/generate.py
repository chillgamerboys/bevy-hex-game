#!/usr/bin/env python3
"""Reproduce the bounded Grand tree library from its committed art provenance.

Run without flags to verify bytes. --write replaces only the adjacent trees.ron.
The twelve stock shapes remain exact copies; three wide grove shapes reuse the
accepted Forest expedition sculptor with explicit local parameters.
"""
from pathlib import Path
import argparse
import hashlib
import json
import re

ROOT = Path(__file__).resolve().parents[5]
HERE = Path(__file__).resolve().parent
STYLES = {"plant/trunk": "timber", "plant/foliage-dark": "foliage_dark",
          "plant/foliage-mid": "foliage", "plant/foliage-light": "foliage_light"}


def source(entry):
    path = ROOT / entry["source"]
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ValueError(f"authored source changed: {path}")
    return path, data.decode()


def render():
    import sys
    sys.path.insert(0, str(ROOT / "tools"))
    from forest_trees import generate_tree
    manifest = json.loads((HERE / "provenance.json").read_text())
    lines = ["// Exact compact voxel geometry from the accepted Forest expedition artwork.",
             "// See provenance.json for stock hashes and the three broadened Grand grove recipes.",
             "(version:1,trees:["]
    for entry in manifest["templates"]:
        _, text = source(entry)
        if "parameters" in entry:
            params = entry["parameters"]
            shape = generate_tree(**params)
            height, radius = shape.height, shape.radius
            voxels = [(v.q, v.r, v.level, STYLES[v.style]) for v in shape.cells]
        else:
            height = int(re.search(r"height:(\d+)", text).group(1))
            radius = int(re.search(r"radius:(\d+)", text).group(1))
            voxels = [(int(q), int(r), int(y), STYLES[style]) for q, r, y, style in re.findall(
                r'position:\(q:(-?\d+),r:(-?\d+),level:(-?\d+)\),style:"([^"]+)"', text)]
        if not voxels or len(voxels) > 65_536:
            raise ValueError(f"invalid occupied-cell budget: {entry['name']}")
        runs = []
        for q, r, y, material in sorted(voxels):
            if runs and runs[-1][:2] == [q, r] and runs[-1][3] == y and runs[-1][4] == material:
                runs[-1][3] = y + 1
            else:
                runs.append([q, r, y, y + 1, material])
        lines.append(f'(name:"{entry["name"]}",height:{height},radius:{radius},runs:[')
        lines.extend(f'({q},{r},{lo},{hi},"{mat}"),' for q, r, lo, hi, mat in runs)
        lines.append("]),")
    lines.append("])")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    expected = render()
    path = HERE / "trees.ron"
    if args.write:
        path.write_text(expected)
    elif path.read_text() != expected:
        raise ValueError("Grand forest library differs from its recorded art sources")
    print("Grand forest artwork provenance: exact")


if __name__ == "__main__":
    main()
