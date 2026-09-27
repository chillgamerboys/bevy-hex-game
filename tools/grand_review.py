#!/usr/bin/env python3
"""Render a windowless, actual-compiled relief review (not a gameplay screenshot)."""
import argparse,pathlib,re,json
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.colors import LightSource,LinearSegmentedColormap

def main():
 p=argparse.ArgumentParser();p.add_argument('--package',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
 s=(a.package/'grand-overview.ron').read_text()
 width=int(re.search(r'width:(\d+)',s)[1]);height=int(re.search(r'(?<![_a-z])height:(\d+)',s)[1]);h=np.fromstring(re.search(r'bed_heights:\[([^]]+)\]',s)[1],sep=',').reshape(height,width)
 origin=[float(v) for v in re.search(r'origin_xz:\(([^)]+)\)',s)[1].split(',')];spacing=float(re.search(r'spacing:([\d.]+)',s)[1]);extent=[origin[0],origin[0]+(width-1)*spacing,origin[1]+(height-1)*spacing,origin[1]]
 cmap=LinearSegmentedColormap.from_list('grand',[(0,'#12304b'),(.24,'#276080'),(.249,'#b6ad7c'),(.26,'#819b63'),(.4,'#356b4c'),(.62,'#7b8581'),(.85,'#b8c0bc'),(1,'#eaf1f4')])
 rgb=LightSource(azdeg=315,altdeg=43).shade(h,cmap,vert_exag=1,dx=spacing,dy=spacing,vmin=0,vmax=560,blend_mode='soft')
 ids=re.findall(r'id:"([^"]+)"',re.search(r'materials:\[(.*?)\],player_spawn:',s)[1])
 materials=np.fromstring(re.search(r'surface_materials:\[([^]]+)\]',s)[1],sep=',',dtype=int).reshape(height,width)
 rgb[h<140]=[.13,.34,.47,1.]
 if 'water' in ids: rgb[materials==ids.index('water')]=[.16,.44,.62,1.]
 fig,axes=plt.subplots(1,2,figsize=(19,10),gridspec_kw={'width_ratios':[1.05,1]},facecolor='#0e1823')
 for ax in axes:ax.imshow(rgb,extent=extent);ax.set_facecolor('#0e1823');ax.tick_params(colors='#aebfca');ax.set_xlabel('World X',color='#aebfca');ax.set_ylabel('World Z (north is up)',color='#aebfca')
 axes[0].set_title('Full finite world · actual compiled relief',color='white',fontsize=17,pad=15)
 axes[1].set_title('Mainland · solid relief (objects omitted)',color='white',fontsize=17,pad=15);biomes=(a.package/'grand-biomes.ron').read_text()
 rows=re.search(r'mainland_rows:\[([^]]*)\]',biomes)
 if not rows: raise RuntimeError('Compiled biome companion lacks mainland bounds')
 corners=[(3**.5*(q+r*.5),1.5*r) for r,lo,hi in re.findall(r'\((-?\d+),(-?\d+),(-?\d+)\)',rows[1]) for r,q in [(int(r),int(lo)),(int(r),int(hi))]]
 if not corners: raise RuntimeError('Compiled mainland footprint is empty')
 xs,zs=zip(*corners);axes[1].set_xlim(min(xs)-25,max(xs)+25);axes[1].set_ylim(max(zs)+25,min(zs)-25)
 anchors=re.search(r'anchors:\{(.*?)\},islands:',s)[1]
 points={name:tuple(map(float,v.split(','))) for name,v in re.findall(r'"([^"]+)":\(([^)]+)\)',anchors)}
 for key,label in [('garden','Garden / Water'),('library_hall','Library'),('shrine_air','Air summit'),('shadow_tunnel','Shadow tunnel'),('crystal_ascent','Crystal / Earth'),('world_tree','World tree / Plant'),('goblin_fort','Camp clearing'),('valley_lake','Valley lake'),('bay','Bay'),('volcano','Volcano / Fire')]:
  if key not in points:continue
  x,y,z=points[key]
  for ax in axes:
   if ax is axes[1] and key=='volcano':continue
   ax.plot(x,z,'o',ms=4,color='#ffdd84');ax.annotate(label,(x,z),xytext=(6,-10),textcoords='offset points',fontsize=8,color='white',bbox=dict(boxstyle='round,pad=.18',fc='#102031',ec='none',alpha=.85))
 receipt=json.loads((a.package/'compile-receipt.json').read_text());fig.text(.06,.04,f"Mainland {receipt['mainland_columns']:,} columns = 7 × {receipt['canonical_mainland_columns']:,}  |  Crystal {receipt['crystal_columns']:,} ({receipt.get('crystal_footprint_basis','legacy reservation')})  |  {receipt['chunks']:,} compact chunks\nHeight samples at {spacing:g} world-unit spacing, no vertical exaggeration. Caves and interiors require separate exact geometry checks; this view shows solid upper relief.",color='#b6c8d4',fontsize=10)
 fig.subplots_adjust(left=.04,right=.98,top=.93,bottom=.11,wspace=.12);a.output.parent.mkdir(parents=True,exist_ok=True);fig.savefig(a.output,dpi=140,facecolor=fig.get_facecolor());print(a.output)
if __name__=='__main__':main()
