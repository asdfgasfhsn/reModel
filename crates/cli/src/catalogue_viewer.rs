//! The catalogue's static viewer: `index.html` in the catalogue directory,
//! one file that opens from `file://` and reads the store's `thumbs/` and
//! `frames/` beside it. A gallery with filters by kind, screen and text,
//! a screens tab (the frames the screens were first seen in), a loads tab
//! (texture RAM uploads and what they fed), and for each entry where it
//! appears (the frame crop), what it appears with, its key, surface and
//! looks.

use crate::catalogue::{load_coverage, report, Index, Merged};
use model2::catalogue::{Ident, Kind, TILE_RECORD};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

fn js(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '<' => o.push_str("\\u003c"),
            c if (c as u32) < 32 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn entry_json(
    ix: &Index,
    id: &Ident,
    m: &Merged,
    partners: &HashMap<Ident, Vec<(Ident, u64)>>,
    users: &HashMap<u64, Vec<Ident>>,
) -> String {
    let sess = |si: usize| ix.sessions.get(si).map_or("", |s| s.id.as_str()).to_string();
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\"id\":{},\"k\":{},\"n\":{},\"w\":{},\"h\":{},\"s\":\"{:016x}\",\"d\":{},\"smp\":{},\"px\":{},\"best\":{},\"box\":[{},{},{},{}],\"img\":{},\"first\":{},\"last\":{},\"ns\":{},\"scr\":[{}],\"ld\":{},\"lds\":{},\"cnt\":{},\"wr\":{}",
        js(&id.to_string()),
        js(id.0.as_str()),
        js(&m.row.name),
        m.row.w,
        m.row.h,
        m.row.surface,
        js(&m.row.detail),
        m.samples,
        m.best_px,
        m.best.1,
        m.bbox[0],
        m.bbox[1],
        m.bbox[2],
        m.bbox[3],
        ix.picture_of(m).map_or("null".into(), |p| js(&p)),
        js(&format!("{} f{}", sess(m.first.0), m.first.1)),
        js(&format!("{} f{}", sess(m.last.0), m.last.1)),
        m.sessions.len(),
        m.screens.iter().map(|(g, n)| format!("[{g},{n}]")).collect::<Vec<_>>().join(","),
        m.loaded.map_or("null".into(), |(si, f)| js(&format!("{} f{f}", sess(si)))),
        m.loads,
        m.count,
        js(&m.wrap.iter().cloned().collect::<Vec<_>>().join(","))
    );
    let with: Vec<String> = match id.0 {
        Kind::Object => m.with.iter().map(|k| js(&Ident(Kind::Tex, *k).to_string())).collect(),
        Kind::Tex => users.get(&id.1).into_iter().flatten().map(|i| js(&i.to_string())).collect(),
        _ => Vec::new(),
    };
    let _ = write!(o, ",\"with\":[{}]", with.join(","));
    let ps: Vec<String> =
        partners.get(id).into_iter().flatten().take(10).map(|(p, w)| format!("[{},{w}]", js(&p.to_string()))).collect();
    let _ = write!(o, ",\"pairs\":[{}]}}", ps.join(","));
    o
}

pub fn write(ix: &Index) -> Result<PathBuf, String> {
    let mut partners: HashMap<Ident, Vec<(Ident, u64)>> = HashMap::new();
    for (&(a, b), &w) in &ix.edges {
        partners.entry(a).or_default().push((b, w));
        partners.entry(b).or_default().push((a, w));
    }
    for v in partners.values_mut() {
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    let mut users: HashMap<u64, Vec<Ident>> = HashMap::new();
    for (id, m) in ix.of_kind(Kind::Object) {
        for k in &m.with {
            users.entry(*k).or_default().push(*id);
        }
    }
    let mut data = String::from("{\"game\":");
    data.push_str(&js(&ix.game));
    data.push_str(",\"entries\":[");
    for (i, (id, m)) in ix.entries.iter().enumerate() {
        if i > 0 {
            data.push(',');
        }
        data.push_str(&entry_json(ix, id, m, &partners, &users));
    }
    data.push_str("],\"screens\":[");
    for (g, s) in ix.screens.iter().enumerate() {
        let _ = write!(
            data,
            "{}{{\"g\":{g},\"samples\":{},\"img\":{}}}",
            if g > 0 { "," } else { "" },
            s.samples,
            s.image.as_ref().map_or("null".into(), |(_, n)| js(n))
        );
    }
    data.push_str("],\"loads\":[");
    let mut first = true;
    for (si, s) in ix.sessions.iter().enumerate() {
        for l in s.loads.iter().take(3000) {
            let g = l
                .screen
                .and_then(|ls| ix.screens.iter().position(|sc| sc.members.contains(&(si, ls))))
                .map_or("null".to_string(), |g| g.to_string());
            let _ = write!(
                data,
                "{}[{},{},{},{},{},{g}]",
                if first { "" } else { "," },
                l.frame,
                js(&l.dest),
                l.len,
                js(&format!("{:016x}", l.hash)),
                l.source.map_or("null".into(), |o| js(&format!("{:#x}", 0x0200_0000u64 + o as u64))),
            );
            first = false;
        }
    }
    let c = load_coverage(ix);
    let _ = write!(
        data,
        "],\"loadsum\":{{\"loads\":{},\"bytes\":{},\"sourced\":{},\"drawn\":{},\"drawn_bytes\":{}}},\"report\":{},\"tiles\":\"",
        c.loads,
        c.bytes,
        c.sourced,
        c.drawn,
        c.drawn_bytes,
        js(&report(ix))
    );
    // The tile atlas, base64: key and RGBA of each pattern once.
    let atlas = std::fs::read(ix.dir.join("thumbs").join("tiles.dat")).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut packed = Vec::new();
    for rec in atlas.as_chunks::<TILE_RECORD>().0 {
        if seen.insert(u64::from_le_bytes(rec[..8].try_into().unwrap())) {
            packed.extend_from_slice(rec);
        }
    }
    data.push_str(&base64(&packed));
    data.push_str("\"}");
    let html = PAGE.replacen("/*DATA*/", &data, 1);
    let path = ix.dir.join("index.html");
    std::fs::write(&path, html).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                o.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

const PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>reModel catalogue</title>
<style>
:root{--bg:#14161a;--fg:#e6e8eb;--dim:#9aa3ad;--card:#1e2228;--line:#2e343c;--acc:#5ab0ff}
@media (prefers-color-scheme: light){:root{--bg:#f4f5f7;--fg:#1b1e22;--dim:#5b6570;--card:#fff;--line:#d5d9de;--acc:#0a6bd6}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.4 system-ui,sans-serif}
header{padding:10px 16px;border-bottom:1px solid var(--line);display:flex;flex-wrap:wrap;gap:8px 16px;align-items:center;position:sticky;top:0;background:var(--bg);z-index:5}
h1{font-size:16px;margin:0}button,select,input{background:var(--card);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:4px 10px;font:inherit}
button.on{border-color:var(--acc);color:var(--acc)}main{padding:12px 16px}
#grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(104px,1fr));gap:8px}
.card{background:var(--card);border:1px solid var(--line);border-radius:8px;padding:6px;cursor:pointer;text-align:center;overflow:hidden}
.card:hover{border-color:var(--acc)}.card .t{height:80px;display:flex;align-items:center;justify-content:center}
.card img,.card canvas{max-width:92px;max-height:76px;image-rendering:pixelated;background:repeating-conic-gradient(#8883 0 25%,#0000 0 50%) 0 0/10px 10px}
.card small{display:block;color:var(--dim);font-size:11px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
#detail{position:fixed;inset:0 0 0 auto;width:min(560px,100%);background:var(--bg);border-left:1px solid var(--line);overflow:auto;padding:16px;display:none;z-index:9}
#detail.open{display:block}#detail h2{margin:0 0 6px;font-size:15px;word-break:break-all}
dl{display:grid;grid-template-columns:auto 1fr;gap:2px 12px;margin:8px 0}dt{color:var(--dim)}dd{margin:0;word-break:break-all}
.frame{position:relative;display:inline-block;max-width:100%;border:1px solid var(--line)}.frame img{display:block;max-width:100%}
.frame i{position:absolute;border:2px solid #ff3b6b;box-shadow:0 0 0 1px #000a}
.row{display:flex;flex-wrap:wrap;gap:6px;margin:6px 0}.row .card{width:84px}
table{border-collapse:collapse;width:100%}td,th{border-bottom:1px solid var(--line);padding:3px 8px;text-align:left;vertical-align:top}
.scr{display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:10px}.scr .card img{max-width:100%;max-height:none;background:none}
pre{white-space:pre-wrap;background:var(--card);padding:10px;border-radius:8px;border:1px solid var(--line)}
a{color:var(--acc);cursor:pointer}.x{float:right}
</style></head><body>
<header><h1>reModel catalogue <span id="game"></span></h1>
<span id="tabs"></span><span id="filters"></span><input id="q" placeholder="name, key, detail" size="18"><select id="sort"></select><span id="count" style="color:var(--dim)"></span></header>
<main><div id="grid"></div><div id="screens" class="scr" hidden></div><div id="loads" hidden></div><pre id="report" hidden></pre></main>
<aside id="detail"></aside>
<script>
const D=/*DATA*/;
const $=s=>document.querySelector(s),E={};D.entries.forEach(e=>E[e.id]=e);
const tiles={};{const b=atob(D.tiles),n=b.length/264|0;for(let i=0;i<n;i++){let o=i*264,k='';for(let j=7;j>=0;j--)k+=b.charCodeAt(o+j).toString(16).padStart(2,'0');tiles[k]=o+8}}
const tileBytes=(()=>{const b=atob(D.tiles),a=new Uint8ClampedArray(b.length);for(let i=0;i<b.length;i++)a[i]=b.charCodeAt(i);return a})();
function thumb(e,big){
 if(e.k==='tile'){const c=document.createElement('canvas');c.width=8;c.height=8;const o=tiles[e.id.slice(5)];
  if(o!==undefined){const id=new ImageData(tileBytes.slice(o,o+256),8,8);c.getContext('2d').putImageData(id,0,0)}
  c.style.width=(big?128:64)+'px';c.style.height=c.style.width;return c}
 const i=new Image();i.src='thumbs/'+e.n+'.png';i.loading='lazy';if(big)i.style.maxWidth='256px';return i}
let st={tab:'gallery',kind:'all',screen:null,q:'',sort:'samples'};
const KINDS=['all','tex','tile','layout','object'];
function el(t,c,h){const e=document.createElement(t);if(c)e.className=c;if(h!==undefined)e.innerHTML=h;return e}
function esc(s){return String(s).replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]))}
function card(e){const c=el('div','card');c.append(el('div','t'));c.firstChild.append(thumb(e));
 c.append(el('small','',esc(e.k==='tex'?e.w+'x'+e.h:e.k==='layout'?e.d.match(/cells=\S+/)[0]:e.k==='object'?e.cnt+' polys':'8x8')));
 c.append(el('small','',e.smp+' smp'));c.onclick=()=>show(e);c.title=e.n;return c}
function lnk(id){const e=E[id];if(!e)return el('span','',id);const c=card(e);return c}
function show(e){const d=$('#detail');d.innerHTML='';d.className='open';
 const x=el('a','x','close');x.onclick=()=>d.className='';d.append(x,el('h2','',esc(e.n)));d.append(thumb(e,true));
 const dl=el('dl');const add=(k,v)=>{dl.append(el('dt','',k),el('dd','',v))};
 add('id',e.id);add('size',e.w+' x '+e.h);add('surface (artwork)',e.s);add('detail',esc(e.d));if(e.wr)add('wrap',esc(e.wr)+' (rr repeat, mr/rm/mm mirrored in u, v, both)');
 add('seen',e.smp+' samples, '+e.ns+' session(s); best '+e.px+' px at frame '+e.best);add('first',esc(e.first));add('last',esc(e.last));
 add('loaded',e.ld?esc(e.ld)+' ('+e.lds+' loads overlap its texels)':(e.k==='tex'?'no load seen (before observing began, or a loaded state)':'-'));
 const same=D.entries.filter(o=>o.k==='tex'&&e.k==='tex'&&o.s===e.s&&o.id!==e.id);if(same.length)add('same artwork',same.length+' other key(s): other colours or luma tables');
 d.append(dl);
 if(same.length){const r=el('div','row');same.slice(0,24).forEach(o=>r.append(lnk(o.id)));d.append(el('b','','Same artwork under other keys'),r)}
 if(e.img){d.append(el('b','','Where it appears (best sighting)'));const f=el('div','frame');const im=new Image();im.src='frames/'+e.img;f.append(im);
  if(e.box[2]>e.box[0]){const b=el('i');b.style.left=e.box[0]/496*100+'%';b.style.top=e.box[1]/384*100+'%';b.style.width=(e.box[2]-e.box[0]+1)/496*100+'%';b.style.height=(e.box[3]-e.box[1]+1)/384*100+'%';f.append(b)}d.append(f)}
 if(e.scr.length){d.append(el('div','','<b>Screens</b> (click to filter): '));const r=el('div');e.scr.forEach(([g,n])=>{const a=el('a','','screen '+g+' ('+n+') ');a.onclick=()=>{st.screen=g;st.tab='gallery';render();d.className=''};r.append(a)});d.append(r)}
 if(e.with.length){d.append(el('b','',e.k==='object'?'Textures it shows':'Objects that show it'));const r=el('div','row');e.with.slice(0,40).forEach(i=>r.append(lnk(i)));d.append(r)}
 if(e.pairs.length){d.append(el('b','','Touches (same object or layer)'));const r=el('div','row');e.pairs.forEach(([i,w])=>{const c=lnk(i);c.append(el('small','','touch '+w));r.append(c)});d.append(r)}
}
function filtered(){const q=st.q.toLowerCase();let a=D.entries.filter(e=>(st.kind==='all'||e.k===st.kind)&&(st.screen===null||e.scr.some(s=>s[0]===st.screen))&&(!q||(e.n+' '+e.id+' '+e.d).toLowerCase().includes(q)));
 const f={samples:(a,b)=>b.smp-a.smp,pixels:(a,b)=>b.px-a.px,rare:(a,b)=>a.smp-b.smp,name:(a,b)=>a.n<b.n?-1:1,loaded:(a,b)=>(b.lds)-(a.lds)};a.sort(f[st.sort]);return a}
function render(){['grid','screens','loads','report'].forEach(i=>$('#'+i).hidden=true);
 $('#tabs').innerHTML='';[['gallery','Gallery'],['screens','Screens'],['loads','Loads'],['report','Report']].forEach(([t,n])=>{const b=el('button',st.tab===t?'on':'',n);b.onclick=()=>{st.tab=t;render()};$('#tabs').append(b)});
 $('#filters').innerHTML='';KINDS.forEach(k=>{const n=k==='all'?D.entries.length:D.entries.filter(e=>e.k===k).length;const b=el('button',st.kind===k?'on':'',k+' '+n);b.onclick=()=>{st.kind=k;render()};$('#filters').append(b)});
 if(st.screen!==null){const b=el('button','on','screen '+st.screen+' x');b.onclick=()=>{st.screen=null;render()};$('#filters').append(b)}
 if(st.tab==='gallery'){const a=filtered();$('#count').textContent=a.length+' shown';const g=$('#grid');g.hidden=false;g.innerHTML='';a.slice(0,1500).forEach(e=>g.append(card(e)));if(a.length>1500)g.append(el('div','','... '+(a.length-1500)+' more: narrow the filter'))}
 else if(st.tab==='screens'){const s=$('#screens');s.hidden=false;s.innerHTML='';D.screens.slice().sort((a,b)=>b.samples-a.samples).forEach(sc=>{const c=el('div','card');if(sc.img){const i=new Image();i.src='frames/'+sc.img;c.append(i)}c.append(el('small','','screen '+sc.g+': '+sc.samples+' samples, '+D.entries.filter(e=>e.scr.some(x=>x[0]===sc.g)).length+' entries'));c.onclick=()=>{st.screen=sc.g;st.tab='gallery';render()};s.append(c)});$('#count').textContent=D.screens.length+' screens'}
 else if(st.tab==='loads'){const l=$('#loads');l.hidden=false;const L=D.loadsum;l.innerHTML='<p>'+L.loads+' loads, '+(L.bytes/1048576).toFixed(1)+' MB; ROM address found for '+L.sourced+'; '+L.drawn+' under a drawn texture ('+(L.drawn_bytes/1048576).toFixed(1)+' MB). Showing the first '+D.loads.length+'.</p><table><tr><th>frame<th>destination<th>bytes<th>data hash<th>ROM source<th>screen</tr>'+D.loads.map(r=>'<tr><td>'+r[0]+'<td>'+esc(r[1])+'<td>'+r[2]+'<td>'+r[3]+'<td>'+(r[4]||'-')+'<td>'+(r[5]===null?'-':r[5])).join('')+'</table>';$('#count').textContent=''}
 else{$('#report').hidden=false;$('#report').textContent=D.report}}
$('#game').textContent=D.game;$('#q').oninput=e=>{st.q=e.target.value;render()};
['samples:most seen','pixels:most pixels','rare:rarest first','name:name','loaded:most loads'].forEach(s=>{const[v,n]=s.split(':');$('#sort').append(new Option(n,v))});$('#sort').onchange=e=>{st.sort=e.target.value;render()};
render();
</script></body></html>
"##;
