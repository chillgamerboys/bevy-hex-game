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
 rgb=LightSource(azdeg=315,altdeg=43).shade(h,cmap,vert_exag=2,dx=spacing,dy=spacing,vmin=0,vmax=560,blend_mode='soft')
 fig,axes=plt.subplots(1,2,figsize=(19,10),gridspec_kw={'width_ratios':[1.05,1]},facecolor='#0e1823')
 for ax in axes:ax.imshow(rgb,extent=extent);ax.set_facecolor('#0e1823');ax.tick_params(colors='#aebfca');ax.set_xlabel('World X',color='#aebfca');ax.set_ylabel('World Z (north is up)',color='#aebfca')
 axes[0].set_title('Full finite world · actual compiled relief',color='white',fontsize=17,pad=15)
 axes[1].set_title('Mainland · terrain before object dressing',color='white',fontsize=17,pad=15);axes[1].set_xlim(-875,875);axes[1].set_ylim(780,-790)
 anchors=re.search(r'anchors:\{(.*?)\},islands:',s)[1]
 points={name:tuple(map(float,v.split(','))) for name,v in re.findall(r'"([^"]+)":\(([^)]+)\)',anchors)}
 for key,label in [('garden','Garden / Water'),('library_hall','Library'),('shrine_air','Air summit'),('shadow_tunnel','Shadow tunnel'),('crystal_ascent','Crystal / Earth'),('world_tree','World tree / Plant'),('goblin_fort','Goblin fort'),('valley_lake','Valley lake'),('bay','Bay'),('volcano','Volcano / Fire')]:
  if key not in points:continue
  x,y,z=points[key]
  for ax in axes:
   if ax is axes[1] and key=='volcano':continue
   ax.plot(x,z,'o',ms=4,color='#ffdd84');ax.annotate(label,(x,z),xytext=(6,-10),textcoords='offset points',fontsize=8,color='white',bbox=dict(boxstyle='round,pad=.18',fc='#102031',ec='none',alpha=.85))
 receipt=json.loads((a.package/'compile-receipt.json').read_text());fig.text(.06,.04,f"Mainland {receipt['mainland_columns']:,} columns = 7 × {receipt['canonical_mainland_columns']:,}  |  Crystal {receipt['crystal_columns']:,} = 7 × 3,169  |  {receipt['chunks']:,} compact chunks\nHeight samples at 4 world-unit spacing. Caves and interiors require separate exact geometry checks; this view shows solid upper relief.",color='#b6c8d4',fontsize=10)
 fig.subplots_adjust(left=.04,right=.98,top=.93,bottom=.11,wspace=.12);a.output.parent.mkdir(parents=True,exist_ok=True);fig.savefig(a.output,dpi=140,facecolor=fig.get_facecolor());print(a.output)
if __name__=='__main__':main()
