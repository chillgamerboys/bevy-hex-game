//! Lightweight geographic review from compiled summaries, without loading terrain.

use std::error::Error;
use std::path::Path;

use hex_world_contracts::WorldManifest;

pub(super) fn write(manifest: &WorldManifest, path: &Path) -> Result<(), Box<dyn Error>> {
    manifest.validate()?;
    let mut value = serde_json::to_value(manifest)?;
    stringify_coordinates(&mut value);
    let data = serde_json::to_string(&value)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let html = HTML.replace("__WORLD_DATA__", &data).replace(
        "__SAMPLE_PITCH__",
        &hex_world_contracts::SUMMARY_SAMPLE_PITCH.to_string(),
    );
    super::write_bytes(path, html.as_bytes())?;
    Ok(())
}

// JavaScript numbers lose exact i64 addresses before a view origin can be
// subtracted. Preserve horizontal coordinates as decimal strings in the atlas.
fn stringify_coordinates(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == "q" || key == "r" {
                    if let Some(integer) = value.as_i64() {
                        *value = serde_json::Value::String(integer.to_string());
                    }
                } else {
                    stringify_coordinates(value);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(stringify_coordinates),
        _ => {}
    }
}

const HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>World atlas · V4</title><style>
:root{color-scheme:dark;font-family:system-ui,-apple-system,BlinkMacSystemFont,sans-serif;color:#eae9e0;background:#141a1a}
*{box-sizing:border-box}body{margin:0;display:grid;grid-template-rows:auto 1fr;height:100vh;overflow:hidden}
header{display:flex;align-items:center;gap:18px;padding:20px 28px;border-bottom:1px solid #ffffff14;background:#172020}
h1{margin:0;font-size:21px;font-weight:550;letter-spacing:-.5px}header small{color:#9aada7;font-size:12px;display:block;margin-top:5px}
.tag{font-size:10px;letter-spacing:2px;text-transform:uppercase;color:#bacdc1;background:#263c34;border:1px solid #496154;border-radius:5px;padding:7px 9px}
.layout{display:grid;grid-template-columns:260px 1fr;min-height:0}aside{padding:25px 22px;background:#172020;overflow-y:auto;border-right:1px solid #ffffff12}
aside h2{font-size:11px;letter-spacing:1.8px;color:#98aea5;text-transform:uppercase;margin:0 0 18px}.stat{display:flex;justify-content:space-between;padding:9px 0;font-size:13px;border-bottom:1px solid #ffffff0b}.stat span{color:#a4b3ad}.stat strong{font-weight:500}
.region{width:100%;display:block;text-align:left;padding:11px 12px;border:1px solid #ffffff12;border-radius:6px;background:#202b28;color:#e6eadf;margin:7px 0;cursor:pointer}.region:hover{background:#2c4036}.region b{display:block;font-size:13px;font-weight:500}.region small{display:block;color:#9ab1a4;margin-top:3px}
.controls{display:flex;gap:7px;flex-wrap:wrap;margin:20px 0}button,select{font:inherit;font-size:12px;background:#273c33;color:#dce5da;border:1px solid #51675b;border-radius:5px;padding:8px 11px;cursor:pointer}button:hover{background:#3d5547}
.help{color:#a1b1a8;font-size:12px;line-height:1.7}.note{font-size:11px;line-height:1.6;color:#9aa9a2;border-top:1px solid #ffffff12;padding-top:14px;margin-top:24px}
main{position:relative;min-width:0;min-height:0}canvas{width:100%;height:100%;display:block;touch-action:none;cursor:grab}canvas.dragging{cursor:grabbing}
.hud{position:absolute;bottom:22px;left:25px;right:25px;display:flex;justify-content:space-between;pointer-events:none;font-size:12px;color:#c5d2c9}.hud span{background:#17211ddb;padding:9px 13px;border:1px solid #ffffff19;border-radius:5px}.legend{display:flex;gap:12px;flex-wrap:wrap;margin-top:20px;font-size:10px;color:#b7c7b9}.legend i{display:inline-block;width:9px;height:9px;border-radius:50%;margin-right:4px}
@media(max-width:750px){.layout{grid-template-columns:180px 1fr}aside{padding:18px 13px}header{padding:15px 18px}.tag{display:none}}
</style></head><body><header><span class="tag">V4 · Geographic review</span><div><h1 id="name">World atlas</h1><small id="identity"></small></div></header>
<div class="layout"><aside><h2>World overview</h2><div id="stats"></div><div class="controls"><button id="fit">Fit world</button><button id="seams">Boundaries</button><button id="features">Landmarks</button></div><h2>Regions</h2><div id="regions"></div><div id="legend" class="legend"></div><p class="help">Drag to explore.<br>Scroll to zoom.<br>Select a region to focus.</p><p class="note">This view reads compiled geographic summaries. Detailed terrain is loaded only by the world runtime. Visual and motion acceptance are separate from this preview.</p></aside><main><canvas id="map" aria-label="Interactive compiled world atlas"></canvas><div class="hud"><span id="location">Drag to explore</span><span id="scale"></span></div></main></div>
<script type="application/json" id="world">__WORLD_DATA__</script><script>
'use strict';
const world=JSON.parse(document.getElementById('world').textContent);
const canvas=document.getElementById('map'),ctx=canvas.getContext('2d');
const landmarks=world.features.filter(f=>['entry','observation','ruin','gameplay-anchor'].includes(f.kind));
const materials=new Map(world.materials.map(m=>[m.id,m]));
const origin=world.regions[0]?.origin??{q:0,r:0};
const xy=p=>{const q=Number(BigInt(p.q)-BigInt(origin.q)),r=Number(BigInt(p.r)-BigInt(origin.r));return[Math.sqrt(3)*(q+r/2),1.5*r];};
const points=world.summary.map(s=>({...s,xy:xy(s.position)}));
let width=0,height=0,zoom=1,offset=[0,0],drag=null,showSeams=true,showFeatures=true;
document.getElementById('name').textContent=world.world_id;
document.getElementById('identity').textContent=world.compiler_version+' · '+world.regions.length+' connected region'+(world.regions.length===1?'':'s');
for(const [label,value] of [['Regions',world.regions.length],['Storage chunks',world.chunks.length.toLocaleString()],['Geographic samples',points.length.toLocaleString()],['Boundaries',world.boundaries.length],['Feature instances',world.features.length],['Landmarks',landmarks.length]]){const row=document.createElement('div');row.className='stat';const a=document.createElement('span');a.textContent=label;const b=document.createElement('strong');b.textContent=String(value);row.append(a,b);document.getElementById('stats').append(row);}
for(const region of world.regions){const b=document.createElement('button');b.className='region';const title=document.createElement('b');title.textContent=region.id;const sub=document.createElement('small');sub.textContent='Radius '+region.radius+' · '+(1+3*region.radius*(region.radius+1)).toLocaleString()+' columns';b.append(title,sub);b.onclick=()=>fit(region);document.getElementById('regions').append(b);}
for(const m of world.materials){const e=document.createElement('span'),dot=document.createElement('i');dot.style.background='rgb('+m.color.slice(0,3).join(',')+')';e.append(dot,document.createTextNode(m.id));document.getElementById('legend').append(e);}
function fit(region){let source=region?points.filter(p=>p.region_id===region.id):points;if(!source.length)return;let minX=Infinity,minY=Infinity,maxX=-Infinity,maxY=-Infinity;for(const p of source){minX=Math.min(minX,p.xy[0]);maxX=Math.max(maxX,p.xy[0]);minY=Math.min(minY,p.xy[1]);maxY=Math.max(maxY,p.xy[1]);}zoom=Math.min(width/Math.max(1,maxX-minX+30),height/Math.max(1,maxY-minY+30))*.86;offset=[width/2-(minX+maxX)/2*zoom,height/2-(minY+maxY)/2*zoom];draw();}
function screen(p){return[p[0]*zoom+offset[0],p[1]*zoom+offset[1]];}
function draw(){ctx.setTransform(devicePixelRatio,0,0,devicePixelRatio,0,0);ctx.fillStyle='#111d1e';ctx.fillRect(0,0,width,height);const step=__SAMPLE_PITCH__;for(const p of points){const [x,y]=screen(p.xy);if(x< -40||y< -40||x>width+40||y>height+40)continue;const material=materials.get(p.material),rgb=material?.color??[105,128,92,255],light=.73+Math.min(.37,Math.max(0,p.level)/180);ctx.fillStyle='rgb('+rgb.slice(0,3).map(c=>Math.round(Math.min(255,c*light))).join(',')+')';const r=Math.max(1.4,step*zoom*.95);ctx.beginPath();for(let i=0;i<6;i++){const a=(i*60-30)*Math.PI/180,px=x+Math.cos(a)*r,py=y+Math.sin(a)*r;i?ctx.lineTo(px,py):ctx.moveTo(px,py);}ctx.closePath();ctx.fill();}
if(showSeams){ctx.strokeStyle='#edead56c';ctx.lineWidth=1;for(const boundary of world.boundaries){ctx.beginPath();let first=true;for(const s of boundary.samples){const a=xy(s.a),b=xy(s.b),p=screen([(a[0]+b[0])/2,(a[1]+b[1])/2]);if(first){ctx.moveTo(...p);first=false;}else ctx.lineTo(...p);}ctx.stroke();}}
if(showFeatures){const boxes=[];ctx.font='11px system-ui';for(const feature of landmarks){const [x,y]=screen(xy(feature.anchor.column));if(x<0||y<0||x>width||y>height)continue;ctx.fillStyle='#faf1c9';ctx.strokeStyle='#17211d';ctx.lineWidth=2;ctx.beginPath();ctx.arc(x,y,3.8,0,Math.PI*2);ctx.fill();ctx.stroke();const label=feature.id.split('/').slice(1,3).join(' / ').replaceAll('-',' '),w=ctx.measureText(label).width;for(const [lx,ly] of [[x+8,y+4],[x+8,y-13],[x-w-8,y+4]]){const box=[lx-3,ly-12,w+6,17];if(box[0]<0||box[0]+box[2]>width||box[1]<0||box[1]+box[3]>height||boxes.some(b=>box[0]<b[0]+b[2]&&box[0]+box[2]>b[0]&&box[1]<b[1]+b[3]&&box[1]+box[3]>b[1]))continue;boxes.push(box);ctx.fillStyle='#13201eda';ctx.fillRect(...box);ctx.fillStyle='#f1efdc';ctx.fillText(label,lx,ly);break;}}}
document.getElementById('scale').textContent=zoom.toFixed(2)+' px / world unit';}
function resize(){const rect=canvas.getBoundingClientRect();width=rect.width;height=rect.height;canvas.width=Math.round(width*devicePixelRatio);canvas.height=Math.round(height*devicePixelRatio);fit();}
new ResizeObserver(resize).observe(canvas);
canvas.addEventListener('pointerdown',e=>{drag=[e.clientX,e.clientY,...offset];canvas.setPointerCapture(e.pointerId);canvas.classList.add('dragging');});
canvas.addEventListener('pointermove',e=>{if(drag){offset=[drag[2]+e.clientX-drag[0],drag[3]+e.clientY-drag[1]];draw();}const rect=canvas.getBoundingClientRect(),x=(e.clientX-rect.left-offset[0])/zoom,y=(e.clientY-rect.top-offset[1])/zoom,r=2*y/3,q=x/Math.sqrt(3)-r/2;document.getElementById('location').textContent='q '+(BigInt(Math.round(q))+BigInt(origin.q)).toString()+' · r '+(BigInt(Math.round(r))+BigInt(origin.r)).toString();});
const end=()=>{drag=null;canvas.classList.remove('dragging');};canvas.addEventListener('pointerup',end);canvas.addEventListener('pointercancel',end);
canvas.addEventListener('wheel',e=>{e.preventDefault();const rect=canvas.getBoundingClientRect(),x=e.clientX-rect.left,y=e.clientY-rect.top,next=Math.min(35,Math.max(.025,zoom*Math.exp(-e.deltaY*.001)));offset=[x-(x-offset[0])*next/zoom,y-(y-offset[1])*next/zoom];zoom=next;draw();},{passive:false});
document.getElementById('fit').onclick=()=>fit();document.getElementById('seams').onclick=()=>{showSeams=!showSeams;draw();};document.getElementById('features').onclick=()=>{showFeatures=!showFeatures;draw();};
</script></body></html>"##;

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_json_cannot_close_its_script_element() {
        let text = "</script><script>alert(1)</script>";
        let escaped = serde_json::to_string(text)
            .expect("string serialization")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
        assert!(!escaped.contains("</script>"));
        let restored: String = serde_json::from_str(&escaped).expect("escaped JSON remains valid");
        assert_eq!(restored, text);
    }
}
