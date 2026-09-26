#!/usr/bin/env python3
"""Compile immutable full-scale Grand packages using the production chunk writer."""
import argparse, hashlib, json, os, pathlib, subprocess, tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
def signature(source):
    paths=[source,*sorted((ROOT/'crates/hex_schematic/src/v4/grand').rglob('*.rs')),*sorted((ROOT/'crates/hex_schematic/src/v4/northern').rglob('*.rs')),ROOT/'crates/hex_world_tool/src/grand.rs',ROOT/'assets/config/v4/grand-v4/forest/trees.ron']
    h=hashlib.sha256()
    for p in paths:h.update(p.read_bytes())
    return h.hexdigest()
def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('mode',nargs='?',choices=['compile','ensure'],default='compile');ap.add_argument('--source',type=pathlib.Path,default=ROOT/'assets/config/v4/grand-v4/world.ron');ap.add_argument('--output',type=pathlib.Path,default=ROOT/'assets/config/v4/grand-v4/compiled');ap.add_argument('--target-dir',type=pathlib.Path,default=ROOT/'target/v4-authoring');ap.add_argument('--plain',action='store_true',help='Same full terrain, caves and water; omit object dressing for composition review');ap.add_argument('--worldc',type=pathlib.Path,help='Use a prebuilt worldc; never invokes Cargo when supplied');a=ap.parse_args()
    sig=signature(a.source)+('-plain' if a.plain else '-dressed')
    if a.output.exists():
        stamp=a.output/'authoring-identity.json'
        if a.mode=='ensure' and all((a.output/f).is_file() for f in ['manifest.ron','grand-overview.ron','arena-sites.ron','grand-biomes.ron']) and stamp.is_file() and json.loads(stamp.read_text()).get('signature')==sig:return 0
        ap.error('Output already exists or is stale. Compile to a fresh --output and select it with HEX_GRAND_WORLD; immutable packages are never overwritten.')
    a.output.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='grand-source-',dir=a.output.parent) as tmp:
        source=a.source.resolve()
        if a.plain:
            source=pathlib.Path(tmp)/'world.ron';source.write_text(a.source.read_text().replace('full_dressing:true','full_dressing:false'))
        cmd=[str(a.worldc.resolve())] if a.worldc else ['cargo','run','-p','hex_world_tool','--bin','worldc','--']
        env=dict(os.environ,CARGO_TARGET_DIR=str(a.target_dir.resolve()),CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='2')
        result=subprocess.call(cmd+['grand-compile','--source',str(source),'--output',str(a.output.resolve())],cwd=ROOT,env=env)
        if result:return result
    (a.output/'authoring-identity.json').write_text(json.dumps({'signature':sig,'plain':a.plain},indent=2)+'\n')
    return 0
if __name__=='__main__':raise SystemExit(main())
