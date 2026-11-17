//! The static viewer `remodel texmap` writes: one HTML file with the
//! frames, their ID and texel-position images and every texture's picture
//! embedded as data URIs, so it opens from file:// (canvas reads of
//! same-origin data URIs work there; of files next to the page they do not)
//! and needs no server and no library.

use crate::texmap::{Run, FIRST_ID};
use std::fmt::Write as _;
use std::path::Path;

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn uri(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64(png))
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The viewer's HTML, with up to `max_frames` of the run's frames (spread
/// evenly) read back from `out_dir/frames`.
pub fn html(run: &Run, out_dir: &Path, game: &str, max_frames: usize) -> Result<String, String> {
    let n = run.frames.len();
    let take = max_frames.clamp(1, n);
    let picked: Vec<usize> = (0..take).map(|i| i * n / take).collect();
    let read = |name: String| {
        let p = out_dir.join("frames").join(&name);
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let mut d = format!("{{\"game\":{},\"frames\":[", json_str(game));
    for (k, &i) in picked.iter().enumerate() {
        let f = &run.frames[i];
        let stem = format!("{:06}", f.frame);
        let _ = write!(
            d,
            "{}{{\"frame\":{},\"w\":{},\"h\":{},\"pic\":\"{}\",\"ids\":\"{}\",\"uv\":\"{}\",\"stats\":{{",
            if k > 0 { "," } else { "" },
            f.frame,
            f.width,
            f.height,
            uri(&read(format!("{stem}.png"))?),
            uri(&read(format!("{stem}-ids.png"))?),
            uri(&read(format!("{stem}-uv.png"))?),
        );
        for (j, s) in f.stats.iter().filter(|s| s.id >= FIRST_ID).enumerate() {
            let _ = write!(
                d,
                "{}\"{}\":[{},{},{},{},{},{},{},{},{},{}]",
                if j > 0 { "," } else { "" },
                s.id,
                s.pixels,
                s.bbox[0],
                s.bbox[1],
                s.bbox[2],
                s.bbox[3],
                s.polys,
                s.uv_box[0] as i64,
                s.uv_box[1] as i64,
                s.uv_box[2],
                s.uv_box[3]
            );
        }
        d.push_str("}}");
    }
    d.push_str("],\"tex\":[");
    for (i, id) in run.ranked().into_iter().enumerate() {
        let (e, t) = (run.reg.entry(id).unwrap(), &run.totals[&id]);
        let thumb = png::encode_rgba(e.image.width, e.image.height, &e.image.data);
        let _ = write!(
            d,
            "{}{{\"id\":{},\"name\":{},\"kind\":{},\"w\":{},\"h\":{},\"pixels\":{},\"frames\":{},\"group\":{},\"wrap\":{},\"detail\":{},\"img\":\"{}\"}}",
            if i > 0 { "," } else { "" },
            id,
            json_str(&e.name),
            json_str(e.kind()),
            e.w,
            e.h,
            t.pixels,
            t.frames,
            run.group_of.get(&id).copied().unwrap_or(0),
            json_str(&crate::texmap::wrap_names(t.wraps)),
            json_str(&e.detail),
            uri(&thumb)
        );
    }
    d.push_str("],\"groups\":[");
    for (i, g) in run.groups.iter().enumerate() {
        let m: Vec<String> = g.iter().map(|id| id.to_string()).collect();
        let _ = write!(d, "{}[{}]", if i > 0 { "," } else { "" }, m.join(","));
    }
    d.push_str("]}");
    Ok(TEMPLATE.replace("/*TITLE*/", game).replace("/*DATA*/", &d))
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>texmap: /*TITLE*/</title>
<style>
:root{color-scheme:dark;--bg:#14161a;--panel:#1d2026;--line:#2e333c;--fg:#e4e6ea;--dim:#9aa1ad;--acc:#ffb020}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:13px/1.4 system-ui,sans-serif}
header{display:flex;gap:16px;align-items:baseline;padding:10px 16px;border-bottom:1px solid var(--line)}
header h1{font-size:15px;margin:0}header span{color:var(--dim)}
main{display:grid;grid-template-columns:minmax(0,1fr) 340px;gap:12px;padding:12px 16px}
@media(max-width:900px){main{grid-template-columns:1fr}}
.tabs{display:flex;flex-wrap:wrap;gap:4px;margin-bottom:8px}
.tabs button{background:var(--panel);color:var(--fg);border:1px solid var(--line);border-radius:4px;padding:3px 9px;cursor:pointer}
.tabs button.on{border-color:var(--acc);color:var(--acc)}
.stage{position:relative;width:100%;background:#000;line-height:0}
.stage canvas{width:100%;height:auto;display:block;image-rendering:pixelated}
.stage canvas#ov{position:absolute;inset:0;pointer-events:none}
.side{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:10px;align-self:start;position:sticky;top:8px}
.side h2{font-size:13px;margin:0 0 6px}.side .name{font-family:ui-monospace,monospace;word-break:break-all}
.prev{position:relative;display:inline-block;line-height:0;background:repeating-conic-gradient(#3a3d44 0 25%,#4a4e56 0 50%) 0 0/12px 12px;margin:6px 0}
.prev img{image-rendering:pixelated;display:block}.prev .box{position:absolute;border:1px solid var(--acc);box-shadow:0 0 0 1px #000;pointer-events:none}
.prev .cross{position:absolute;width:9px;height:9px;margin:-4px 0 0 -4px;border:2px solid #f33;border-radius:50%;pointer-events:none}
dl{display:grid;grid-template-columns:auto 1fr;gap:2px 10px;margin:4px 0}dt{color:var(--dim)}dd{margin:0}
.members{display:flex;flex-wrap:wrap;gap:6px;margin-top:6px}
.members figure,.cell{margin:0;cursor:pointer;border:1px solid var(--line);border-radius:4px;padding:3px;background:#101216;text-align:center}
.members figure.on,.cell.on{border-color:var(--acc)}
.members img,.cell img{image-rendering:pixelated;display:block;margin:0 auto;background:repeating-conic-gradient(#3a3d44 0 25%,#4a4e56 0 50%) 0 0/8px 8px}
figcaption{font-size:11px;color:var(--dim)}
section.gallery{padding:0 16px 24px}
.bar{display:flex;gap:12px;align-items:center;margin:10px 0;flex-wrap:wrap}
.bar input[type=search]{background:var(--panel);color:var(--fg);border:1px solid var(--line);border-radius:4px;padding:4px 8px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(96px,1fr));gap:8px}
.cell .t{font-size:11px;color:var(--dim);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.hint{color:var(--dim);margin:6px 0 0}
</style></head><body>
<header><h1>texmap: /*TITLE*/</h1><span id="summary"></span><span>hover a pixel for its texture, click for its group, Esc to clear</span></header>
<main>
<div><div class="tabs" id="tabs"></div>
<div class="stage"><canvas id="pic"></canvas><canvas id="ov"></canvas></div>
<p class="hint">Highlighted: the hovered texture (amber); with a group selected, its other members (blue).</p></div>
<aside class="side" id="side"><h2>Nothing under the cursor</h2><p class="hint">Move over the picture.</p></aside>
</main>
<section class="gallery"><h2>All textures and tiles of the run, by coverage</h2>
<div class="bar"><label><input type="checkbox" id="fTex" checked> 3D textures</label><label><input type="checkbox" id="fTile" checked> tiles</label>
<input type="search" id="q" placeholder="filter by id or name"><span id="count"></span></div>
<div class="grid" id="grid"></div></section>
<script>
const D=/*DATA*/;
const T=new Map(D.tex.map(t=>[t.id,t]));
const $=s=>document.getElementById(s);
const groupOf=new Map();D.groups.forEach((g,i)=>g.forEach(id=>groupOf.set(id,i+1)));
let cur=0,hover=0,sticky=0,selGroup=0,ids=null,uvd=null,pos=null;
const pic=$('pic'),ov=$('ov'),pc=pic.getContext('2d'),oc=ov.getContext('2d');
const fmt=n=>n.toLocaleString();
function load(i){
  cur=i;const f=D.frames[i];
  [...$('tabs').children].forEach((b,j)=>b.classList.toggle('on',j==i));
  const get=(src,cb)=>{const im=new Image();im.onload=()=>cb(im);im.src=src};
  pic.width=ov.width=f.w;pic.height=ov.height=f.h;
  get(f.pic,im=>pc.drawImage(im,0,0));
  get(f.ids,im=>{const c=document.createElement('canvas');c.width=f.w;c.height=f.h;const x=c.getContext('2d');x.drawImage(im,0,0);ids=x.getImageData(0,0,f.w,f.h).data;paint()});
  get(f.uv,im=>{const c=document.createElement('canvas');c.width=f.w;c.height=f.h;const x=c.getContext('2d');x.drawImage(im,0,0);uvd=x.getImageData(0,0,f.w,f.h).data});
}
function idAt(x,y){if(!ids||x<0||y<0||x>=pic.width||y>=pic.height)return 0;const o=(y*pic.width+x)*4;return ids[o]|ids[o+1]<<8|ids[o+2]<<16}
function paint(){
  if(!ids)return;const f=D.frames[cur];const img=oc.createImageData(f.w,f.h),o=img.data;
  const shown=sticky||hover;const grp=selGroup?new Set(D.groups[selGroup-1]):null;
  for(let p=0,q=0;p<f.w*f.h;p++,q+=4){
    const id=ids[q]|ids[q+1]<<8|ids[q+2]<<16;
    if(id<2)continue;
    if(id==shown){o[q]=255;o[q+1]=176;o[q+2]=32;o[q+3]=170}
    else if(grp&&grp.has(id)){o[q]=60;o[q+1]=140;o[q+2]=255;o[q+3]=130}
  }
  oc.putImageData(img,0,0);
}
function info(id){
  const side=$('side');const t=T.get(id);
  if(!t){side.innerHTML=id==1?'<h2>Untextured polygon</h2><p class="hint">A flat colour: nothing to restyle with a texture.</p>':'<h2>Nothing here</h2><p class="hint">Backdrop or no layer draws this pixel.</p>';return}
  const s=D.frames[cur].stats[id];const S=Math.max(1,Math.min(8,Math.floor(160/Math.max(t.w,t.h))));
  const g=groupOf.get(id),gm=g?D.groups[g-1]:[id];
  let h=`<h2>#${id} <span class="name">${t.name}</span></h2>`;
  h+=`<div class="prev" id="prev"><img src="${t.img}" width="${t.w*S}" height="${t.h*S}">`;
  if(s&&s[6]>=0&&t.kind=='tex')h+=`<div class="box" style="left:${s[6]*S}px;top:${s[7]*S}px;width:${(s[8]-s[6]+1)*S}px;height:${(s[9]-s[7]+1)*S}px"></div>`;
  h+=`<div class="cross" id="cross" style="display:none"></div></div>`;
  h+=`<dl><dt>kind</dt><dd>${t.kind}, ${t.w}x${t.h}</dd>`;
  h+=`<dt>this frame</dt><dd>${s?fmt(s[0])+' px, '+s[5]+' polygons, box '+s[1]+','+s[2]+'-'+s[3]+','+s[4]:'not shown'}</dd>`;
  if(s&&t.kind=='tex')h+=`<dt>texels shown</dt><dd>${s[6]},${s[7]} to ${s[8]},${s[9]} (amber box)</dd>`;
  h+=`<dt>run</dt><dd>${fmt(t.pixels)} px over ${t.frames} frame(s)</dd><dt>wrap</dt><dd>${t.wrap||'-'}</dd><dt>detail</dt><dd>${t.detail}</dd>`;
  h+=`<dt>group</dt><dd>${gm.length>1?'#'+g+', '+gm.length+' members':'alone'}</dd></dl>`;
  if(gm.length>1){h+='<div class="members">';for(const m of gm){const u=T.get(m);if(!u)continue;const k=Math.max(1,Math.min(4,Math.floor(48/Math.max(u.w,u.h))));h+=`<figure data-id="${m}" class="${m==id?'on':''}"><img src="${u.img}" width="${u.w*k}" height="${u.h*k}"><figcaption>#${m}</figcaption></figure>`}h+='</div>'}
  side.innerHTML=h;
  side.querySelectorAll('figure').forEach(el=>el.onclick=()=>select(+el.dataset.id,true));
  showCross();
}
function showCross(){
  const id=sticky||hover,t=T.get(id),c=$('cross');if(!c||!t||!pos||!uvd||idAt(pos[0],pos[1])!=id)return;
  const o=(pos[1]*pic.width+pos[0])*4;const u=uvd[o]/256,v=uvd[o+1]/256;
  const im=$('prev').querySelector('img');c.style.display='block';c.style.left=(u*im.width)+'px';c.style.top=(v*im.height)+'px';
}
function select(id,keepGroup){
  sticky=id;hover=id;if(!keepGroup||!selGroup)selGroup=groupOf.get(id)&&D.groups[groupOf.get(id)-1].length>1?groupOf.get(id):0;
  if(!D.frames[cur].stats[id]){const j=D.frames.findIndex(f=>f.stats[id]);if(j>=0){load(j)}}
  info(id);paint();document.querySelectorAll('.cell').forEach(c=>c.classList.toggle('on',+c.dataset.id==id));
}
function clear(){sticky=0;selGroup=0;hover=0;info(0);paint()}
ov.parentElement.addEventListener('mousemove',e=>{
  const r=pic.getBoundingClientRect();const x=Math.floor((e.clientX-r.left)*pic.width/r.width),y=Math.floor((e.clientY-r.top)*pic.height/r.height);
  pos=[x,y];const id=idAt(x,y);
  if(sticky){showCross();return}
  if(id!=hover){hover=id;info(id);paint()}else showCross();
});
ov.parentElement.addEventListener('mouseleave',()=>{if(!sticky){hover=0;paint()}});
ov.parentElement.addEventListener('click',e=>{const id=idAt(pos?pos[0]:-1,pos?pos[1]:-1);if(id>=2)select(id,false);else clear()});
addEventListener('keydown',e=>{if(e.key=='Escape')clear()});
D.frames.forEach((f,i)=>{const b=document.createElement('button');b.textContent='frame '+f.frame;b.onclick=()=>load(i);$('tabs').appendChild(b)});
function gallery(){
  const q=$('q').value.trim().toLowerCase(),tx=$('fTex').checked,tl=$('fTile').checked;
  const list=D.tex.filter(t=>(t.kind=='tex'?tx:tl)&&(!q||t.name.includes(q)||String(t.id)==q));
  $('count').textContent=list.length+' of '+D.tex.length;
  $('grid').innerHTML=list.slice(0,600).map(t=>{const k=Math.max(1,Math.min(6,Math.floor(80/Math.max(t.w,t.h))));
    return `<div class="cell" data-id="${t.id}"><img src="${t.img}" width="${t.w*k}" height="${t.h*k}"><div>#${t.id}</div><div class="t">${fmt(t.pixels)} px</div></div>`}).join('');
  $('grid').querySelectorAll('.cell').forEach(el=>el.onclick=()=>{select(+el.dataset.id,false);scrollTo({top:0,behavior:'smooth'})});
}
['fTex','fTile','q'].forEach(i=>$(i).addEventListener('input',gallery));
$('summary').textContent=D.frames.length+' frames, '+D.tex.length+' textures and tiles, '+D.groups.filter(g=>g.length>1).length+' groups';
gallery();load(0);
</script></body></html>
"##;
