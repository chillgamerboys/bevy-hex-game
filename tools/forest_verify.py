#!/usr/bin/env python3
"""Check authored expedition geometry against worldc's validated public survey.

This consumes a supported public JSON boundary, never private package RON. The
production world/site loader remains responsible for runtime admission.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from forest_expedition import site_document
from forest_world import Raw, ron


def position(value):
    return value["column"]["q"], value["column"]["r"], value["level"]


def verify(survey, metadata, report):
    if survey["version"] != 1 or survey["world_id"] != metadata["world_id"] or survey["world_id"] != report["world_id"]:
        raise ValueError("survey and authoring identities differ")
    solid = {m["id"] for m in survey["materials"] if m["solid"]}
    terrain = {(c["position"]["q"],c["position"]["r"]): c["runs"] for c in survey["columns"]}
    actual = {}
    objects_by_column = {}
    for obj in survey["objects"]:
        key = obj["asset"], tuple(position(obj["origin"])[:2]), obj["rotation"]
        if key in actual:
            raise ValueError(f"duplicate object identity {key}")
        actual[key] = obj
        for c in obj["occupancy"]:
            p = c["position"]["q"], c["position"]["r"]
            objects_by_column.setdefault(p, []).extend(c["runs"])
    expected = len(report["placements"]) + len(report["structures"])
    if len(actual) != expected:
        raise ValueError(f"compiled {len(actual)} objects instead of {expected}")
    canopy = set()
    contacts_count = 0
    for entry in report["placements"] + report["structures"]:
        turn = entry.get("rotation", entry.get("desired_rotation"))
        key = entry["asset"], tuple(entry["root"]), turn
        if key not in actual:
            raise ValueError(f'missing {entry["id"]}')
        obj = actual.pop(key)
        if obj["origin"]["level"] != entry["support_level"] + 1:
            raise ValueError(f'{entry["id"]}: origin shifted vertically')
        contacts = {position(p) for p in obj["grounding"]}
        if contacts != set(map(tuple, entry["ground_contacts"])):
            raise ValueError(f'{entry["id"]}: grounding differs from exact proposed contacts')
        contacts_count += len(contacts)
        for q, r, level in contacts:
            runs = terrain[q,r]
            if not any(v["bottom"] <= level < v["top"] and v["material"] in solid for v in runs):
                raise ValueError(f'{entry["id"]}: floating compiled contact')
        actual_runs = sorted((c["position"]["q"], c["position"]["r"], r["bottom"], r["top"], r["material"])
                             for c in obj["occupancy"] for r in c["runs"])
        if "occupied_runs" in entry:
            if actual_runs != sorted(map(tuple, entry["occupied_runs"])):
                raise ValueError(f'{entry["id"]}: visible/tree collision intervals differ')
            actual_canopy = {(q,r) for q,r,_,_,m in actual_runs if m == "foliage"}
            if actual_canopy != set(map(tuple, entry["canopy_columns"])):
                raise ValueError(f'{entry["id"]}: foliage footprint differs')
            canopy.update(actual_canopy)
        else:
            voxels = {(q,r,z) for q,r,low,high,_ in actual_runs for z in range(low,high)}
            if voxels != set(map(tuple, entry["occupied_world"])):
                raise ValueError(f'{entry["id"]}: structure geometry differs')
    if actual:
        raise ValueError("unaccounted-for objects remain")
    # Full authored supports and clearances are checked, including the last
    # cells of winding routes and all deployment columns, not just anchors.
    supports = {}
    for route in metadata["routes"].values():
        for q,r,level in route["ribbon"]:
            supports[q,r,level] = max(supports.get((q,r,level),0), route["clearance_levels"])
    for site in metadata["encounters"].values():
        for q,r,level in site["surfaces"]:
            supports[q,r,level] = max(supports.get((q,r,level),0), site.get("clearance_levels", 4))
    for (q,r,level), clear in supports.items():
        if not any(v["bottom"] <= level < v["top"] and v["material"] in solid for v in terrain[q,r]):
            raise ValueError(f"site support changed at {(q,r,level)}")
        for run in terrain[q,r] + objects_by_column.get((q,r),[]):
            if run["material"] in solid and run["bottom"] < level+1+clear and level+1 < run["top"]:
                raise ValueError(f"site clearance blocked at {(q,r,level)}")
    liquid_cells = {(w["column"]["q"],w["column"]["r"],z) for w in survey["liquids"] for z in range(w["bottom"],w["top"])}
    for name,pool in metadata["fountains"].items():
        if not set(map(tuple,pool["cells"])) <= liquid_cells:
            raise ValueError(f"{name}: compiled water is missing")
    forest = set(map(tuple,metadata["forest"]["columns"]))
    canopy_count = len(canopy & forest)
    if canopy_count != report["canopy_columns"] or canopy_count < .5*len(forest):
        raise ValueError("compiled canopy differs from the report or misses the50%floor")
    return {"version":1,"world_id":survey["world_id"],"manifest_fingerprint":survey["manifest_fingerprint"],
            "object_count":expected,"tree_count":len(report["placements"]),"structure_count":len(report["structures"]),
            "exact_ground_contacts":contacts_count,"validated_site_supports":len(supports),
            "route_count":len(metadata["routes"]),"encounter_count":len(metadata["encounters"]),
            "fountain_count":len(metadata["fountains"]),"forest_columns":len(forest),
            "canopy_columns":canopy_count,"canopy_fraction":canopy_count/len(forest),
            "evidence":"compiled-public-world-facts; production admission and rendered/native review separate"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--survey",type=Path,required=True)
    parser.add_argument("--authoring",type=Path,required=True)
    parser.add_argument("--placement",type=Path,required=True)
    parser.add_argument("--package",type=Path,required=True,help="compiled package receiving its strict site companion")
    parser.add_argument("--report",type=Path,required=True)
    args=parser.parse_args()
    survey,metadata,placement=(json.loads(p.read_text()) for p in (args.survey,args.authoring,args.placement))
    result=verify(survey,metadata,placement)
    (args.package/"arena-sites.ron").write_text(site_document(metadata,world_id=survey["world_id"],
        manifest_fingerprint=survey["manifest_fingerprint"],raw=Raw,ron=ron))
    args.report.write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result))


if __name__=="__main__":
    main()
