#!/usr/bin/env python3
"""Reproduce or verify the complete expedition through the supported world CLI.

Build the compiler explicitly with tools/world.py first. This command never
invokes Cargo, reads private package files, or changes the legacy Forest source.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

from forest_expedition import document, site_document
from forest_finish import compose, final_source
from forest_verify import verify
from forest_world import ROOT, Raw, ron

CONTENT = ROOT / "assets/config/v4/forest-massif/expedition"


def run_world(target, *args):
    subprocess.run([sys.executable, str(ROOT / "tools/world.py"), "--target-dir", str(target), *map(str,args)],
                   cwd=ROOT, check=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command",choices=("compile","verify"))
    parser.add_argument("--target-dir",type=Path,default=ROOT/"target")
    parser.add_argument("--output",type=Path,default=CONTENT/"compiled")
    parser.add_argument("--scratch",type=Path,default=ROOT/".context/expedition-reproduction")
    args=parser.parse_args()
    args.scratch.mkdir(parents=True,exist_ok=True)
    generation=json.loads((CONTENT/"generation.json").read_text())
    terrain,_=document(raw=Raw,ron=ron)
    if terrain != (CONTENT/"terrain.ron").read_text():
        raise ValueError("terrain generator differs from reviewed source")
    run_world(args.target_dir,"compile","--source",CONTENT/"terrain.ron","--output",args.scratch/"terrain-compiled")
    run_world(args.target_dir,"survey","--package",args.scratch/"terrain-compiled","--output",args.scratch/"terrain-survey.json")
    survey=json.loads((args.scratch/"terrain-survey.json").read_text())
    value,metadata,assets,outputs,placement,_=compose(survey)
    revision=generation["source_revision"]
    for path,expected in outputs.items():
        committed=subprocess.run(["git","show",f"{revision}:{path}"],cwd=ROOT,check=True,capture_output=True,text=True).stdout
        if committed != expected or (ROOT/path).read_text() != expected:
            raise ValueError(f"current or committed exact artwork differs: {path}")
    source=final_source(value,placement,assets,revision)
    if source != (CONTENT/"world.ron").read_text():
        raise ValueError("terrain-aware placements no longer reproduce reviewed source")
    if metadata != json.loads((CONTENT/"authoring.json").read_text()):
        # Tuples are normalized by JSON without changing authored coordinates.
        if json.loads(json.dumps(metadata)) != json.loads((CONTENT/"authoring.json").read_text()):
            raise ValueError("site metadata differs from reviewed authoring")
    if args.command=="compile":
        run_world(args.target_dir,"compile","--source",CONTENT/"world.ron","--output",args.output)
    run_world(args.target_dir,"survey","--package",args.output,"--output",args.scratch/"final-survey.json")
    survey=json.loads((args.scratch/"final-survey.json").read_text())
    evidence=verify(survey,metadata,placement)
    if f'{survey["manifest_fingerprint"]:016x}' != generation["package_fingerprint"]:
        raise ValueError("reproduced package fingerprint differs from reviewed source")
    sidecar=site_document(metadata,world_id=survey["world_id"],manifest_fingerprint=survey["manifest_fingerprint"],raw=Raw,ron=ron)
    if args.command=="compile":
        (args.output/"arena-sites.ron").write_text(sidecar)
        (args.output/"content-verification.json").write_text(json.dumps(evidence,indent=2)+"\n")
    elif (args.output/"arena-sites.ron").read_text() != sidecar:
        raise ValueError("package companion differs from verified authoring")
    (args.scratch/"verification.json").write_text(json.dumps(evidence,indent=2)+"\n")
    print(json.dumps(evidence))


if __name__=="__main__":
    main()
