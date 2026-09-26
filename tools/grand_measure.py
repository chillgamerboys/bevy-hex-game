#!/usr/bin/env python3
"""Reproduce the canonical Grand V3 mainland ownership and author its 7x footprint.

Uses the production radius187, pitch22, nearest axial-hex-distance + CellId tie
break. Excludes offshore Island facts and marine open water, includes inland
MountainLake/ValleyLake and their LakeIsland. The mainland is the largest six-neighbour component.
No radius-disk substitution. Output rows describe occupied footprint columns,
not solid volume; inland lakes are deliberately included.
"""
import argparse, collections, hashlib, json, math, pathlib, re
ROOT = pathlib.Path(__file__).resolve().parents[1]
DIRS = [(1,0),(0,1),(-1,1),(-1,0),(0,-1),(1,-1)]
def distance(q,r): return max(abs(q),abs(r),abs(q+r))
def nearest(q,r):
    a,b,c=round(q),round(r),round(-q-r)
    x,y,z=abs(a-q),abs(b-r),abs(c+q+r)
    if x>y and x>z:a=-b-c
    elif y>z:b=-a-c
    return a,b

def measure():
    source=ROOT/'assets/config/schematics/grand-v3-template.ron'
    raw=source.read_text()
    cells={}
    for m in re.finditer(r'\(id:(\d+),coord:\(q:(-?\d+),r:(-?\d+),s:-?\d+\),facts:\(surface:(\w+),landform:(\w+),.*?overlays:\[([^]]*)\]',raw):
        i,q,r,surf,form,overlays=m.groups()
        cells[int(q),int(r)]=(int(i), (surf=='Land' and (form!='Island' or 'LakeIsland' in overlays)) or any(x in overlays for x in ['MountainLake','ValleyLake']))
    assert len(cells)==217
    mainland=set()
    offsets=[(q,r) for q in range(-2,3) for r in range(-2,3) if distance(q,r)<=2]
    for q in range(-187,188):
        for r in range(max(-187,-187-q),min(187,187-q)+1):
            cq,cr=q//22,r//22
            candidates=[(distance(q-(cq+dq)*22,r-(cr+dr)*22),*cells[cq+dq,cr+dr]) for dq,dr in offsets if (cq+dq,cr+dr) in cells]
            if not candidates:candidates=[(distance(q-a*22,r-b*22),*v) for (a,b),v in cells.items()]
            if min(candidates)[2]:mainland.add((q,r))
    components=[]
    rest=set(mainland)
    while rest:
        start=rest.pop(); todo=[start];component={start}
        for q,r in todo:
            for dq,dr in DIRS:
                p=q+dq,r+dr
                if p in rest:rest.remove(p);component.add(p);todo.append(p)
        components.append(component)
    canonical=max(components,key=len)
    scale=math.sqrt(7); expanded=set()
    # Keep the geographic center unchanged. Ocean is enlarged separately.
    for q in range(-550,551):
        for r in range(-550,551):
            # A smooth bijective-scale coastal warp removes the clipped paper-grid
            # edges while preserving landmark ordering. Exact area is corrected below.
            aq=q/scale + 8*math.sin(q*.019+r*.004)+6*math.sin(r*.035)
            ar=r/scale + 8*math.sin(q*.022)-7*math.sin((q+r)*.027)
            if nearest(aq,ar) in canonical:expanded.add((q,r))
    raw_count=len(expanded); target=len(canonical)*7
    # Discrete resampling is rounded only at the coast. Add/remove deterministic
    # boundary cells to make the agreed integer area exact without moving sites.
    while len(expanded)!=target:
        if len(expanded)<target:
            rim={ (q+dq,r+dr) for q,r in expanded for dq,dr in DIRS if (q+dq,r+dr) not in expanded }
            ordered=sorted(rim,key=lambda p:(((p[0]*73856093)^(p[1]*19349663))%2147483647,p))
            expanded.update(ordered[:target-len(expanded)])
        elif len(expanded)>target:
            rim=[p for p in expanded if sum((p[0]+dq,p[1]+dr) in expanded for dq,dr in DIRS)>=3 and any((p[0]+dq,p[1]+dr) not in expanded for dq,dr in DIRS)]
            for p in sorted(rim,key=lambda p:(((p[0]*73856093)^(p[1]*19349663))%2147483647,p))[:len(expanded)-target]:expanded.remove(p)
    assert len(expanded)==target
    rows=[]
    grouped=collections.defaultdict(list)
    for q,r in expanded: grouped[r].append(q)
    for r in sorted(grouped):
        qs=sorted(grouped[r])
        start=prev=qs[0]
        for q in qs[1:]:
            if q!=prev+1:rows.append((r,start,prev));start=q
            prev=q
        rows.append((r,start,prev))
    # Standalone Crystal map radius40 is NOT the embedded Grand footprint32.
    receipt=dict(canonical_source=str(source.relative_to(ROOT)),canonical_sha256=hashlib.sha256(raw.encode()).hexdigest(),fine_radius=187,coarse_pitch=22,full_disk_columns=1+3*187*188,canonical_mainland_columns=len(canonical),other_included_component_columns=sorted(len(c) for c in components if c is not canonical),mainland_columns=len(expanded),mainland_area_ratio=len(expanded)/len(canonical),linear_scale=scale,coast_shape="smooth bounded warp of canonical footprint; exact area preserved",coast_rounding_columns=abs(raw_count-target),canonical_crystal_radius=32,canonical_crystal_columns=1+3*32*33,crystal_target_columns=(1+3*32*33)*7,hex_column_area=3*math.sqrt(3)/2)
    return rows,receipt

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--write',action='store_true');args=ap.parse_args()
    rows,receipt=measure()
    if args.write:
        out=ROOT/'assets/config/v4/grand-v4';out.mkdir(parents=True,exist_ok=True)
        (out/'measurement.json').write_text(json.dumps(receipt,indent=2)+'\n')
        (out/'world.ron').write_text('// Generated by tools/grand_measure.py --write; human geometry lives in GrandCompiler.\n(\n version:1, full_dressing:true, id:"grand-v4", seed:20260925,\n canonical_mainland_columns:'+str(receipt['canonical_mainland_columns'])+',\n canonical_crystal_columns:3169,\n mainland_rows:[\n'+''.join(f'  ({r},{a},{b}),\n' for r,a,b in rows)+'],\n)\n')
    print(json.dumps(receipt,indent=2))
if __name__=='__main__':main()
