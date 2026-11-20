//! SPIKE `remodel authoring-spike MODE ...`: the times behind the Studio page's "save in an
//! editor, see it in the scene within a second" (docs/uplift/authoring.md). Offscreen GPU only,
//! a hard cap on GPU frames (`AS_GPU_CAP`, default 150), no window. Modes:
//!   cold CAPDIR FRAME PACK        cold pack load and first correct frame (also "today's way")
//!   one CAPDIR FRAME PACK TESTDIR swap one image: read + decode + upload + next frame
//!   pick CAPDIR FRAME             ID-pass cost and one-pixel read-back at native, 1080p, 4K
//!   poll DIR..                    cost of a file-time poll
//!   watch DIR                     save-detection rule against editor write patterns
//!   norm                          adapter steps (resize, alpha from the original, mask composite)
//!   jump STATE [FRAMES]           load a save state and draw its first frame
use model2::capture::{self, Frame};
use model2::enhance::{Rgba, TexturePack};
use model2::raster;
use model2::video::{self, View};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static GPU_FRAMES: AtomicU32 = AtomicU32::new(0);

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn count_frames(n: u32) {
    let total = GPU_FRAMES.fetch_add(n, Ordering::Relaxed) + n;
    let cap = std::env::var("AS_GPU_CAP").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
    if total > cap {
        eprintln!("GPU frame cap {cap} reached; stopping");
        std::process::exit(3);
    }
}

fn stats(v: &mut [f64]) -> String {
    if v.is_empty() {
        return "n/a".into();
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    format!("median {:.2} min {:.2} max {:.2} (n={})", v[v.len() / 2], v[0], v[v.len() - 1], v.len())
}

struct Scene<'a> {
    polys: Vec<raster::Poly>,
    placed: Vec<raster::Placed>,
    src: raster::Sources<'a>,
    layers: video::Layers2d,
    view: View,
    version: u64,
    names: Vec<(u64, String)>,
}

fn scene<'a>(f: &'a Frame, rom: &'a [u16], view: View, windows: (&[usize], &[usize])) -> Scene<'a> {
    let view = view.with_hud_windows(windows.0, windows.1);
    let src = f.sources(rom);
    let mut polys = raster::build_shaded(&f.stream, &src, None);
    video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
    let placed = raster::place(&mut polys, &view);
    let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
    let mut keys = raster::TexKeys::default();
    keys.begin(None);
    let mut names: Vec<(u64, String)> = Vec::new();
    for pl in &placed {
        let p = &polys[pl.poly];
        let Some(t) = &p.texture else { continue };
        if let Some(info) = raster::tex_info(p, t, &src, &mut keys) {
            if !names.iter().any(|(k, _)| *k == info.key) {
                names.push((info.key, info.name()));
            }
        }
    }
    Scene { polys, placed, src, layers, view, version: f.frame + 1, names }
}

fn target(gpu: &gpu::Renderer, view: &View) -> gpu::wgpu::TextureView {
    gpu.device()
        .create_texture(&gpu::wgpu::TextureDescriptor {
            label: Some("authoring bench"),
            size: gpu::wgpu::Extent3d { width: view.width as u32, height: view.height as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::wgpu::TextureDimension::D2,
            format: gpu::wgpu::TextureFormat::Rgba8Unorm,
            usage: gpu::wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn wait(g: &gpu::Renderer) {
    let _ = g.device().poll(gpu::wgpu::PollType::Wait { submission_index: None, timeout: None });
}

/// One frame into `tv`, waiting for the GPU (no read-back): ms.
fn draw_wait(gpu: &mut gpu::Renderer, s: &Scene, tv: &gpu::wgpu::TextureView) -> f64 {
    count_frames(1);
    let t = Instant::now();
    gpu.set_ram_versions(s.version, s.version);
    gpu.present(&s.polys, &s.placed, &s.src, &s.view, &s.layers, tv, gpu::wgpu::TextureFormat::Rgba8Unorm);
    wait(gpu);
    ms(t.elapsed())
}

fn hash(rgb: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in rgb {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn quality(mip: bool) -> gpu::Quality {
    gpu::Quality {
        tex_filter: if mip { model2::enhance::TexFilter::Mip } else { model2::enhance::TexFilter::Off },
        ..Default::default()
    }
}

fn evict_cache(dir: &Path) {
    let script = "import os,sys\nfor r,_,fs in os.walk(sys.argv[1]):\n for f in fs:\n  fd=os.open(os.path.join(r,f),os.O_RDONLY)\n  os.posix_fadvise(fd,0,0,os.POSIX_FADV_DONTNEED)\n  os.close(fd)\n";
    let _ = std::process::Command::new("python3").arg("-I").arg("-c").arg(script).arg(dir).status();
}

pub fn run(roms: &crate::RomSet, args: &[String], windows: (&[usize], &[usize])) -> Result<(), String> {
    let a = |i: usize| args.get(i).cloned().ok_or_else(|| format!("missing argument {i}"));
    let mode = a(1)?;
    match mode.as_str() {
        "cold" => cold(&a(2)?, a(3)?.parse().map_err(|_| "frame")?, Path::new(&a(4)?), windows),
        "one" => one(&a(2)?, a(3)?.parse().map_err(|_| "frame")?, Path::new(&a(4)?), Path::new(&a(5)?), windows),
        "pick" => pick(&a(2)?, a(3)?.parse().map_err(|_| "frame")?, windows),
        "poll" => poll(&args[2..]),
        "watch" => watch(Path::new(&a(2)?)),
        "norm" => norm(),
        "decodeall" => decodeall(Path::new(&a(2)?)),
        "jump" => jump(roms, &a(2)?, args.get(3).and_then(|v| v.parse().ok()).unwrap_or(3)),
        _ => Err("unknown mode".into()),
    }
}

fn load<'a>(root: &str, n: u64) -> Result<(Frame, Vec<u16>), String> {
    let rom = capture::load_texture_rom(Path::new(root)).map_err(|e| e.to_string())?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.join("meta.txt").exists()).collect();
    dirs.sort();
    for d in dirs {
        let f = Frame::load(&d).map_err(|e| e.to_string())?;
        if f.frame == n {
            return Ok((f, rom));
        }
    }
    Err(format!("frame {n} not in {root}"))
}

// ---------------------------------------------------------------- cold
fn cold(root: &str, frame: u64, pack: &Path, windows: (&[usize], &[usize])) -> Result<(), String> {
    let (f, rom) = load(root, frame)?;
    let view = View::new(1080, 16.0 / 9.0, video::Hud::Edges);
    let s = scene(&f, &rom, view, windows);
    let mut gpu = gpu::Renderer::new()?;
    println!("gpu {} | frame {frame}: {} placed polygons, {} distinct textures", gpu.adapter(), s.placed.len(), s.names.len());
    gpu.set_quality(quality(false));
    let tv = target(&gpu, &s.view);
    // Warm the pipelines with no pack, and time a frame with no pack.
    let _ = draw_wait(&mut gpu, &s, &tv);
    let nopack: Vec<f64> = (0..2).map(|_| draw_wait(&mut gpu, &s, &tv)).collect();
    println!("no pack, 1080p frame (GPU wait, no read-back): {:?} ms", nopack);

    let in_pack = |dir: &Path, names: &[(u64, String)]| -> Vec<(u64, String)> {
        names.iter().filter(|(_, n)| dir.join(format!("{n}.png")).exists()).cloned().collect()
    };
    let need = in_pack(pack, &s.names);
    println!("textures the frame draws that the pack has: {} of {}", need.len(), s.names.len());
    // Serial breakdown of what the frame needs: read, decode, upload, per image.
    for cold_cache in [true, false] {
        if cold_cache {
            evict_cache(pack);
        }
        let (mut t_read, mut t_dec, mut bytes_png, mut texels) = (Duration::ZERO, Duration::ZERO, 0usize, 0usize);
        let mut imgs = Vec::new();
        for (k, n) in &need {
            let p = pack.join(format!("{n}.png"));
            let t = Instant::now();
            let b = std::fs::read(&p).map_err(|e| e.to_string())?;
            t_read += t.elapsed();
            let t = Instant::now();
            let im = png::decode(&b)?;
            t_dec += t.elapsed();
            bytes_png += b.len();
            texels += im.width * im.height;
            imgs.push((*k, Rgba { width: im.width, height: im.height, data: im.rgba }));
        }
        let t = Instant::now();
        for (k, im) in &imgs {
            gpu.replace_pack_image(*k, im);
        }
        wait(&gpu);
        let t_up = t.elapsed();
        println!(
            "serial breakdown ({}) for the {} images the frame needs ({:.1} MB of PNG, {:.1} Mpx): read {:.1} ms, PNG decode {:.1} ms, upload (create+write, then GPU idle) {:.1} ms",
            if cold_cache { "files evicted from the page cache first" } else { "files in the page cache" },
            imgs.len(), bytes_png as f64 / 1e6, texels as f64 / 1e6, ms(t_read), ms(t_dec), ms(t_up)
        );
        for (k, _) in &imgs {
            gpu.drop_pack_image(*k);
        }
    }
    // The index, split: readdir + name grammar, manifest, regions (decoded on the calling thread).
    {
        evict_cache(pack);
        let t = Instant::now();
        let mut n = 0;
        let mut names = std::collections::HashSet::new();
        for e in std::fs::read_dir(pack).map_err(|e| e.to_string())?.flatten() {
            let name = e.file_name();
            if let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".png")) {
                if model2::pack::parse_name(stem).is_ok() {
                    names.insert(stem.to_string());
                    n += 1;
                }
            }
        }
        let t_list = ms(t.elapsed());
        let t = Instant::now();
        let _ = model2::pack::Manifest::for_dir(pack);
        let t_man = ms(t.elapsed());
        let t = Instant::now();
        let r = model2::regions::RegionSet::load_dir(pack, &mut |p| {
            let b = std::fs::read(p).ok()?;
            let im = png::decode(&b).ok()?;
            Some(Arc::new(Rgba { width: im.width, height: im.height, data: im.rgba }))
        });
        let t_reg = ms(t.elapsed());
        println!("index split (directory just evicted): readdir + name grammar of {n} files {t_list:.1} ms, manifest {t_man:.2} ms, regions (13 images decoded on the calling thread) {t_reg:.1} ms ({})", r.map(|r| r.is_empty()).map_or("error".to_string(), |e| format!("empty={e}")));
    }
    // The player's way, as a whole: open, set_pack, frames until everything the frame needs is drawn.
    for (label, evict, preload) in [("A: cold (page cache evicted), player defaults (preload on)", true, true), ("B: page cache warm, player defaults (preload on)", false, true)] {
        if evict {
            evict_cache(pack);
        }
        let t0 = Instant::now();
        let dp = model2::dirpack::DirPack::open(pack)?;
        let t_open = ms(t0.elapsed());
        let n_images = dp.len();
        let t = Instant::now();
        gpu.set_pack(Some(Box::new(dp) as Box<dyn TexturePack + Send>));
        gpu.set_pack_fixes(model2::packlog::Fixes { surface: true, hold: true, preload });
        let t_set = ms(t.elapsed());
        let mut times = Vec::new();
        let (mut prev_tiles, mut still) = (u32::MAX, 0);
        for i in 0..40 {
            let d = draw_wait(&mut gpu, &s, &tv);
            let at = ms(t0.elapsed());
            let st = gpu.pack_stats();
            let held: std::collections::HashSet<u64> = gpu.pack_keys().into_iter().collect();
            let have = need.iter().filter(|(k, _)| held.contains(k)).count();
            times.push((i, d, at, st.replaced, st.uploaded, st.tiles, have));
            if have == need.len() && st.uploaded == 0 && st.tiles == prev_tiles {
                still += 1;
            } else {
                still = 0;
            }
            prev_tiles = st.tiles;
            if still >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        println!("{label}: open (readdir {} names, manifest, regions) {:.1} ms, set_pack {:.2} ms", n_images, t_open, t_set);
        for (i, d, at, rep, up, tiles, have) in &times {
            println!("   frame {i}: {d:.1} ms, {at:.0} ms after open; polygons replaced {rep}, uploaded {up}, tiles replaced {tiles}, needed images held {have}/{}", need.len());
        }
        if let Some(r) = times.iter().find(|r| r.6 == need.len()) {
            println!("   first frame with all {} needed textures drawn: frame {} at {:.0} ms after open", need.len(), r.0, r.2);
        }
        let held = gpu.pack_keys().len();
        let st = gpu.pack_stats();
        println!("   at the end: {held} images on the GPU ({} MB), preloaded in all {}", st.bytes >> 20, gpu.preloaded());
    }
    // today's way when a file changes: drop everything and start again.
    println!("(today's way when the pack changes is the line above: the player only re-reads on a change of pack directory, i.e. `set_pack(DirPack::open)`)");
    Ok(())
}

// ---------------------------------------------------------------- one
fn one(root: &str, frame: u64, pack: &Path, testdir: &Path, windows: (&[usize], &[usize])) -> Result<(), String> {
    let (f, rom) = load(root, frame)?;
    let view = View::new(1080, 16.0 / 9.0, video::Hud::Edges);
    let s = scene(&f, &rom, view, windows);
    let mut gpu = gpu::Renderer::new()?;
    println!("gpu {}", gpu.adapter());
    let tv = target(&gpu, &s.view);
    // Pick the biggest-on-screen texture in the pack: the one the frame draws most.
    let mut cover: HashMap<u64, f64> = HashMap::new();
    {
        let mut keys = raster::TexKeys::default();
        keys.begin(None);
        for pl in &s.placed {
            let p = &s.polys[pl.poly];
            let Some(t) = &p.texture else { continue };
            if let Some(info) = raster::tex_info(p, t, &s.src, &mut keys) {
                let pts = &pl.points;
                let mut ar = 0.0f64;
                for k in 0..pts.len() {
                    let (a, b) = (&pts[k], &pts[(k + 1) % pts.len()]);
                    ar += a.x as f64 * b.y as f64 - b.x as f64 * a.y as f64;
                }
                *cover.entry(info.key).or_default() += (ar / 2.0).abs();
            }
        }
    }
    let mut cands: Vec<&(u64, String)> = s.names.iter().filter(|(_, n)| pack.join(format!("{n}.png")).exists()).collect();
    cands.sort_by(|a, b| cover[&b.0].partial_cmp(&cover[&a.0]).unwrap());
    println!("frame draws {} textures the pack has; largest by area:", cands.len());
    for (k, n) in cands.iter().take(5) {
        println!("   {n}  {:.0} px", cover[k]);
    }
    let (key, name) = cands.first().map(|c| (c.0, c.1.clone())).ok_or("no pack texture in this frame")?;
    let orig_bytes = std::fs::read(pack.join(format!("{name}.png"))).map_err(|e| e.to_string())?;

    let mut base = 0;
    for mip in [false, true] {
        let full = !mip;
        gpu.set_quality(quality(mip));
        println!("--- tex_filter {} ---", if mip { "mip (premultiplied mip chain built per upload)" } else { "off" });
        if full {
            gpu.set_pack(Some(Box::new(model2::dirpack::DirPack::open(pack)?) as Box<dyn TexturePack + Send>));
            gpu.set_pack_fixes(model2::packlog::Fixes { surface: true, hold: true, preload: false });
            for _ in 0..4 {
                draw_wait(&mut gpu, &s, &tv);
                std::thread::sleep(Duration::from_millis(40));
            }
            let steady: Vec<f64> = (0..2).map(|_| draw_wait(&mut gpu, &s, &tv)).collect();
            println!("steady frame {:?} ms", steady);
            count_frames(1);
            gpu.set_ram_versions(s.version, s.version);
            base = hash(&gpu.frame(&s.polys, &s.placed, &s.src, &s.view, &s.layers));
        } else {
            draw_wait(&mut gpu, &s, &tv);
        }
        let sizes: &[usize] = if full { &[64, 512, 2048] } else { &[512, 2048] };
        for &side in sizes {
            let file = testdir.join(format!("test_{side}.png"));
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let (mut r, mut d, mut u, mut fr, mut tot) = (vec![], vec![], vec![], vec![], vec![]);
            for _ in 0..(if full { 4 } else { 3 }) {
                // the editor's save: the file is on disk, the clock starts
                let t0 = Instant::now();
                let b = std::fs::read(&file).map_err(|e| e.to_string())?;
                let t1 = Instant::now();
                let im = png::decode(&b)?;
                let img = Rgba { width: im.width, height: im.height, data: im.rgba };
                let t2 = Instant::now();
                gpu.replace_pack_image(key, &img);
                wait(&gpu);
                let t3 = Instant::now();
                let _ = draw_wait(&mut gpu, &s, &tv);
                let t4 = Instant::now();
                r.push(ms(t1 - t0));
                d.push(ms(t2 - t1));
                u.push(ms(t3 - t2));
                fr.push(ms(t4 - t3));
                tot.push(ms(t4 - t0));
            }
            let mut changed = String::new();
            if full {
                count_frames(1);
                gpu.set_ram_versions(s.version, s.version);
                let after = hash(&gpu.frame(&s.polys, &s.placed, &s.src, &s.view, &s.layers));
                changed = format!("; picture changed: {}", after != base);
            }
            println!(
                "swap {side}x{side} ({} KB PNG): read {} | decode {} | upload {} | next frame {} | TOTAL {}{changed}",
                bytes.len() / 1024, stats(&mut r), stats(&mut d), stats(&mut u), stats(&mut fr), stats(&mut tot)
            );
            // put the original back: the picture must return exactly
            let im = png::decode(&orig_bytes)?;
            gpu.replace_pack_image(key, &Rgba { width: im.width, height: im.height, data: im.rgba });
            if full {
                count_frames(1);
                gpu.set_ram_versions(s.version, s.version);
                let back = hash(&gpu.frame(&s.polys, &s.placed, &s.src, &s.view, &s.layers));
                println!("   original put back: picture identical to the baseline: {}", back == base);
            }
        }
        if !full {
            continue;
        }
        // colour variants: the same decoded image uploaded under K keys
        let file = testdir.join("test_512.png");
        let b = std::fs::read(&file).map_err(|e| e.to_string())?;
        for k in [4usize, 12] {
            let t0 = Instant::now();
            let im = png::decode(&b)?;
            let img = Rgba { width: im.width, height: im.height, data: im.rgba };
            let t1 = Instant::now();
            for i in 0..k {
                gpu.replace_pack_image(key.wrapping_add(1 + i as u64), &img);
            }
            wait(&gpu);
            let t2 = Instant::now();
            println!("one 512x512 image under {k} keys: decode once {:.1} ms + {k} uploads {:.1} ms ({:.1} ms each)", ms(t1 - t0), ms(t2 - t1), ms(t2 - t1) / k as f64);
            for i in 0..k {
                gpu.drop_pack_image(key.wrapping_add(1 + i as u64));
            }
        }
        // the cheap-but-async way: drop the cache entry, the pack's worker decodes the file again
        let file = testdir.join("test_512.png");
        let saved = std::fs::read(pack.join(format!("{name}.png"))).map_err(|e| e.to_string())?;
        let mut rec = vec![];
        for _ in 0..2 {
            std::fs::copy(&file, pack.join(format!("{name}.png"))).map_err(|e| e.to_string())?;
            let t0 = Instant::now();
            gpu.drop_pack_image(key);
            let mut frames_without = 0;
            loop {
                draw_wait(&mut gpu, &s, &tv);
                if gpu.pack_keys().contains(&key) {
                    break;
                }
                frames_without += 1;
                if frames_without > 8 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            rec.push((ms(t0.elapsed()), frames_without));
            std::fs::write(pack.join(format!("{name}.png")), &saved).map_err(|e| e.to_string())?;
            let im = png::decode(&orig_bytes)?;
            gpu.replace_pack_image(key, &Rgba { width: im.width, height: im.height, data: im.rgba });
        }
        println!("invalidate + async worker decode (512x512): (ms to the frame with the new image, frames drawn without it) {:?}", rec);
        let _ = std::fs::write(pack.join(format!("{name}.png")), &saved);
    }
    Ok(())
}

// ---------------------------------------------------------------- pick
fn pick(root: &str, frame: u64, windows: (&[usize], &[usize])) -> Result<(), String> {
    let (f, rom) = load(root, frame)?;
    let mut gpu = gpu::Renderer::new()?;
    println!("gpu {}", gpu.adapter());
    let comic = Arc::new(model2::look::Look::comic());
    for (name, view, np, nl) in [
        ("native 496x384", View::native(), 0usize, 1usize),
        ("1080p 1920x1080", View::new(1080, 16.0 / 9.0, video::Hud::Edges), 3, 3),
        ("4K 3840x2160", View::new(2160, 16.0 / 9.0, video::Hud::Edges), 2, 2),
    ] {
        let s = scene(&f, &rom, view, windows);
        let (w, h) = (s.view.width as u32, s.view.height as u32);
        let tv = target(&gpu, &s.view);
        gpu.set_quality(crate::gpu_light(None));
        gpu.pick_blocks = false;
        gpu.set_look(None);
        // the first draw of each is a warm-up (pipelines, targets): not in the numbers
        let mut plain: Vec<f64> = (0..np).map(|_| draw_wait(&mut gpu, &s, &tv)).skip(1).collect();
        gpu.set_look(Some(comic.clone()));
        let mut look: Vec<f64> = (0..nl).map(|_| draw_wait(&mut gpu, &s, &tv)).skip(1).collect();
        println!("{name}: frame, no look {} | with the Comic look (ID pass + ink) {}", stats(&mut plain), stats(&mut look));
        // one pixel / 3x3 reads from the last ID pass (GPU idle); cursor path.
        let mut r1 = vec![];
        let mut r3 = vec![];
        let mut hits = 0;
        for i in 0..60u32 {
            let (x, y) = (50 + (i * 37) % (w - 100), 50 + (i * 53) % (h - 100));
            let t = Instant::now();
            let v = gpu.read_ids_rect((w, h), x, y, 1, 1);
            r1.push(ms(t.elapsed()));
            hits += (v[0][0] != 0) as u32;
            let t = Instant::now();
            let _ = gpu.read_ids_rect((w, h), x - 1, y - 1, 3, 3);
            r3.push(ms(t.elapsed()));
        }
        println!("   read 1 pixel of the ID target: {} ; 3x3 block: {} ; {} of 60 positions hit a polygon", stats(&mut r1), stats(&mut r3), hits);
        if w <= 1920 {
            let t = Instant::now();
            let _ = gpu.read_ids(w, h);
            println!("   (whole ID target for reference: {:.1} ms)", ms(t.elapsed()));
        }
        gpu.set_look(None);
    }
    Ok(())
}

// ---------------------------------------------------------------- poll
fn poll(dirs: &[String]) -> Result<(), String> {
    let roots: Vec<PathBuf> = dirs.iter().map(PathBuf::from).collect();
    let mut known: HashMap<PathBuf, (u64, std::time::SystemTime)> = HashMap::new();
    let scan = |known: &mut HashMap<PathBuf, (u64, std::time::SystemTime)>| -> usize {
        let mut changed = 0;
        for r in &roots {
            for e in std::fs::read_dir(r).into_iter().flatten().flatten() {
                let Ok(m) = e.metadata() else { continue };
                let v = (m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH));
                if known.insert(e.path(), v) != Some(v) {
                    changed += 1;
                }
            }
        }
        changed
    };
    let t = Instant::now();
    let n = scan(&mut known);
    println!("first scan: {} files in {} dirs, {:.2} ms", n, roots.len(), ms(t.elapsed()));
    let mut v: Vec<f64> = (0..200).map(|_| { let t = Instant::now(); scan(&mut known); ms(t.elapsed()) }).collect();
    println!("full poll (readdir + stat every file, all directories): {}", stats(&mut v));
    // directory mtime only
    let mut v: Vec<f64> = (0..200).map(|_| { let t = Instant::now(); for r in &roots { let _ = std::fs::metadata(r).and_then(|m| m.modified()); } ms(t.elapsed()) }).collect();
    println!("directory mtimes only (catches create/rename-in, not an in-place rewrite): {}", stats(&mut v));
    // stat only the files in the scene (e.g. 40 of them)
    let some: Vec<PathBuf> = known.keys().take(40).cloned().collect();
    let mut v: Vec<f64> = (0..200).map(|_| { let t = Instant::now(); for p in &some { let _ = std::fs::metadata(p); } ms(t.elapsed()) }).collect();
    println!("stat of 40 named files only: {}", stats(&mut v));
    Ok(())
}

// ---------------------------------------------------------------- watch
#[derive(Clone, Copy, PartialEq, Debug)]
struct Stat(u64, std::time::SystemTime);

fn stat_of(p: &Path) -> Option<Stat> {
    let m = std::fs::metadata(p).ok()?;
    Some(Stat(m.len(), m.modified().ok()?))
}

/// The rule: a changed file is read when its mtime is `settle` old (nobody wrote in that long);
/// the decode must succeed, else the old image is kept, the failure reported, and the file left
/// alone until its size or time changes again.
struct Watch {
    file: PathBuf,
    accepted: Option<Stat>,
    failed: Option<Stat>,
    settle: Duration,
    naive: bool,
}

enum Event {
    Swapped(Instant, usize),
    Rejected(String),
}

impl Watch {
    fn poll(&mut self) -> Option<Event> {
        let now = std::time::SystemTime::now();
        let s = stat_of(&self.file)?;
        if Some(s) == self.accepted || Some(s) == self.failed {
            return None;
        }
        if !self.naive && now.duration_since(s.1).unwrap_or_default() < self.settle {
            return None; // still being written, look again next poll
        }
        let bytes = std::fs::read(&self.file).ok()?;
        match png::decode(&bytes) {
            Ok(im) => {
                self.accepted = Some(s);
                Some(Event::Swapped(Instant::now(), im.width))
            }
            Err(e) => {
                self.failed = Some(s);
                Some(Event::Rejected(e))
            }
        }
    }
}

fn watch(dir: &Path) -> Result<(), String> {
    let base = std::fs::read(dir.join("sample.png")).map_err(|e| format!("{}/sample.png: {e}", dir.display()))?;
    let target = dir.join("tex_0000000000000001_128x128.png");
    type Writer = fn(&Path, &[u8]) -> Instant;
    let direct: Writer = |p, b| {
        std::fs::write(p, b).unwrap();
        Instant::now()
    };
    let rename: Writer = |p, b| {
        let tmp = p.with_extension("png.tmp");
        std::fs::write(&tmp, b).unwrap();
        std::fs::rename(&tmp, p).unwrap();
        Instant::now()
    };
    let steps: Writer = |p, b| {
        use std::io::Write;
        let mut f = std::fs::File::create(p).unwrap();
        let third = b.len() / 3;
        f.write_all(&b[..third]).unwrap();
        f.flush().unwrap();
        std::thread::sleep(Duration::from_millis(180));
        f.write_all(&b[third..2 * third]).unwrap();
        f.flush().unwrap();
        std::thread::sleep(Duration::from_millis(180));
        f.write_all(&b[2 * third..]).unwrap();
        drop(f);
        Instant::now()
    };
    let delete_create: Writer = |p, b| {
        let _ = std::fs::remove_file(p);
        std::thread::sleep(Duration::from_millis(60));
        std::fs::write(p, b).unwrap();
        Instant::now()
    };
    let backup_rename: Writer = |p, b| {
        // write the new file under a backup name, move the old away, move the new in
        let bak = p.with_extension("png~");
        let tmp = p.with_extension("png.new");
        std::fs::write(&tmp, b).unwrap();
        let _ = std::fs::rename(p, &bak);
        std::thread::sleep(Duration::from_millis(40));
        std::fs::rename(&tmp, p).unwrap();
        let _ = std::fs::remove_file(&bak);
        Instant::now()
    };
    let cases: [(&str, Writer); 5] = [
        ("one write() of the final name", direct),
        ("temp file then rename over it (atomic)", rename),
        ("three writes, 180 ms apart (half-written PNG for 360 ms)", steps),
        ("delete, then create", delete_create),
        ("new file, old moved to a backup, new renamed in", backup_rename),
    ];
    for (poll_ms, settle_ms, naive) in [(250u64, 100u64, false), (100, 100, false), (250, 0, true)] {
        println!("=== poll every {poll_ms} ms, settle {settle_ms} ms{} ===", if naive { " (NAIVE: read on first change, no settle)" } else { "" });
        let _ = std::fs::remove_file(&target);
        std::fs::write(&target, &base).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<(Instant, bool)>();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop2, tp) = (stop.clone(), target.clone());
        let poller = std::thread::spawn(move || {
            let mut wt = Watch { file: tp.clone(), accepted: stat_of(&tp), failed: None, settle: Duration::from_millis(settle_ms), naive };
            while !stop2.load(Ordering::Relaxed) {
                match wt.poll() {
                    Some(Event::Swapped(t, _)) => {
                        let _ = tx.send((t, true));
                    }
                    Some(Event::Rejected(_)) => {
                        let _ = tx.send((Instant::now(), false));
                    }
                    None => {}
                }
                std::thread::sleep(Duration::from_millis(poll_ms));
            }
        });
        std::thread::sleep(Duration::from_millis(500));
        let mut rng = 12345u64;
        for (label, w) in cases {
            let (mut lat, mut early, mut rejects, mut missed) = (vec![], 0, 0, 0);
            for _ in 0..12 {
                while rx.try_recv().is_ok() {}
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                std::thread::sleep(Duration::from_millis(300 + (rng >> 33) % poll_ms));
                let (tp, imgc) = (target.clone(), base.clone());
                let started = Instant::now();
                let h = std::thread::spawn(move || w(&tp, &imgc));
                let done = h.join().unwrap();
                let mut swapped = None;
                let end = Instant::now() + Duration::from_millis(2500);
                while Instant::now() < end {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok((t, true)) => {
                            swapped = Some(t);
                            break;
                        }
                        Ok((_, false)) => rejects += 1,
                        Err(_) => {}
                    }
                }
                match swapped {
                    Some(t) if t >= done => lat.push(ms(t - done)),
                    Some(t) => {
                        early += 1;
                        lat.push(-ms(done - t));
                    }
                    None => missed += 1,
                }
                let _ = started;
                std::thread::sleep(Duration::from_millis(300));
            }
            println!("  {label}: file finished -> accepted: {} ms; accepted before the writer was done {early}; failed decodes reported {rejects}; never accepted {missed}", stats(&mut lat));
        }
        stop.store(true, Ordering::Relaxed);
        poller.join().unwrap();
    }
    Ok(())
}

fn decodeall(dir: &Path) -> Result<(), String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    files.sort();
    let (mut tex, mut tile) = ((0usize, 0usize, 0.0f64), (0usize, 0usize, 0.0f64));
    for f in &files {
        let t = Instant::now();
        let b = std::fs::read(f).map_err(|e| e.to_string())?;
        let im = png::decode(&b)?;
        let d = ms(t.elapsed());
        let is_tile = f.file_name().unwrap().to_string_lossy().starts_with("tile_");
        let acc = if is_tile { &mut tile } else { &mut tex };
        acc.0 += 1;
        acc.1 += im.width * im.height;
        acc.2 += d;
    }
    println!("whole pack decoded single-threaded (page cache warm): textures+regions {} files {:.1} Mpx {:.0} ms ({:.1} ns/px); tiles {} files {:.2} Mpx {:.0} ms", tex.0, tex.1 as f64 / 1e6, tex.2, tex.2 * 1e6 / tex.1 as f64, tile.0, tile.1 as f64 / 1e6, tile.2);
    Ok(())
}

// ---------------------------------------------------------------- norm
fn lanczos(x: f32) -> f32 {
    if x == 0.0 {
        1.0
    } else if x.abs() >= 3.0 {
        0.0
    } else {
        let p = std::f32::consts::PI * x;
        3.0 * p.sin() * (p / 3.0).sin() / (p * p)
    }
}

/// Separable Lanczos3 on premultiplied RGBA f32.
fn resize(src: &Rgba, nw: usize, nh: usize) -> Rgba {
    let pass = |inp: &[f32], iw: usize, ih: usize, ow: usize, horiz: bool| -> Vec<f32> {
        let (n_in, n_out) = if horiz { (iw, ow) } else { (ih, nh) };
        let scale = n_in as f32 / n_out as f32;
        let support = if scale > 1.0 { 3.0 * scale } else { 3.0 };
        let mut taps: Vec<(usize, Vec<f32>)> = Vec::with_capacity(n_out);
        for o in 0..n_out {
            let c = (o as f32 + 0.5) * scale;
            let (lo, hi) = (((c - support).floor() as i64).max(0) as usize, ((c + support).ceil() as i64).min(n_in as i64) as usize);
            let mut w: Vec<f32> = (lo..hi).map(|i| lanczos((i as f32 + 0.5 - c) / scale.max(1.0))).collect();
            let sum: f32 = w.iter().sum();
            w.iter_mut().for_each(|v| *v /= sum);
            taps.push((lo, w));
        }
        if horiz {
            let mut out = vec![0f32; ow * ih * 4];
            for y in 0..ih {
                for (o, (lo, w)) in taps.iter().enumerate() {
                    let mut acc = [0f32; 4];
                    for (k, wt) in w.iter().enumerate() {
                        let i = (y * iw + lo + k) * 4;
                        for c in 0..4 {
                            acc[c] += inp[i + c] * wt;
                        }
                    }
                    out[(y * ow + o) * 4..(y * ow + o) * 4 + 4].copy_from_slice(&acc);
                }
            }
            out
        } else {
            let mut out = vec![0f32; iw * nh * 4];
            for (o, (lo, w)) in taps.iter().enumerate() {
                for x in 0..iw {
                    let mut acc = [0f32; 4];
                    for (k, wt) in w.iter().enumerate() {
                        let i = ((lo + k) * iw + x) * 4;
                        for c in 0..4 {
                            acc[c] += inp[i + c] * wt;
                        }
                    }
                    out[(o * iw + x) * 4..(o * iw + x) * 4 + 4].copy_from_slice(&acc);
                }
            }
            out
        }
    };
    let pm: Vec<f32> = src
        .data
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as f32 / 255.0;
            [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, p[3] as f32]
        })
        .collect();
    let h = pass(&pm, src.width, src.height, nw, true);
    let v = pass(&h, nw, src.height, nw, false);
    let data = v
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3].clamp(0.0, 255.0);
            let inv = if a > 0.0 { 255.0 / a } else { 0.0 };
            [(p[0] * inv).clamp(0.0, 255.0) as u8, (p[1] * inv).clamp(0.0, 255.0) as u8, (p[2] * inv).clamp(0.0, 255.0) as u8, a as u8]
        })
        .collect();
    Rgba { width: nw, height: nh, data }
}

/// Alpha from the original (nearest-upscaled cut-out) and the visible-texel mask composite.
fn alpha_and_mask(img: &mut Rgba, orig_alpha: &[u8], ow: usize, oh: usize) {
    for y in 0..img.height {
        let oy = y * oh / img.height;
        for x in 0..img.width {
            let ox = x * ow / img.width;
            let visible = orig_alpha[oy * ow + ox] >= 128;
            let p = &mut img.data[(y * img.width + x) * 4..(y * img.width + x) * 4 + 4];
            if visible {
                p[3] = 255;
            } else {
                p.copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
}

fn norm() -> Result<(), String> {
    let mk = |s: usize| Rgba {
        width: s,
        height: s,
        data: (0..s * s * 4).map(|i| ((i * 2654435761usize) >> 13) as u8 ^ (i as u8)).collect(),
    };
    for side in [64usize, 512, 2048] {
        // original texture side/4 (a 4x pack) with a cut-out
        let (ow, oh) = (side / 4, side / 4);
        let oa: Vec<u8> = (0..ow * oh).map(|i| if i % 7 == 0 { 0 } else { 255 }).collect();
        let exact = mk(side);
        let half = mk(side / 2);
        let double = mk(side * 2);
        let reps = if side >= 2048 { 3 } else { 10 };
        let time = |f: &mut dyn FnMut()| -> Vec<f64> { (0..reps).map(|_| { let t = Instant::now(); f(); ms(t.elapsed()) }).collect() };
        let mut v_masks = time(&mut || { let mut im = exact.clone(); alpha_and_mask(&mut im, &oa, ow, oh); });
        let mut v_up = time(&mut || { let _ = resize(&half, side, side); });
        let mut v_down = time(&mut || { let _ = resize(&double, side, side); });
        println!(
            "{side}x{side}: alpha+mask on a right-sized image {} | resize {}x{} up to {side} (Lanczos3, f32, one thread) {} | resize {}x{} down to {side} {}",
            stats(&mut v_masks), side / 2, side / 2, stats(&mut v_up), side * 2, side * 2, stats(&mut v_down)
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- jump
fn jump(roms: &crate::RomSet, state: &str, rounds: usize) -> Result<(), String> {
    let t = Instant::now();
    let mut m = crate::machine(roms);
    println!("{}: construct machine (ROM already in memory in the player; the file reads are in here) {:.0} ms", roms.game.id, ms(t.elapsed()));
    let t = Instant::now();
    let data = std::fs::read(state).map_err(|e| e.to_string())?;
    let t_read = ms(t.elapsed());
    let mut gpu = gpu::Renderer::new()?;
    gpu.set_quality(crate::gpu_light(None));
    let view = View::new(1080, 16.0 / 9.0, video::Hud::Edges).with_hud_windows(roms.game.hud_windows, roms.game.hud_element_windows);
    // warm the GPU on the live (boot) frame, so its pipelines are not in the numbers
    for _ in 0..2 {
        let f0 = m.frames;
        while m.frames == f0 {
            m.run_slice(u64::MAX).map_err(|s| format!("{s:?}"))?;
        }
    }
    count_frames(1);
    let _ = gpu.frame_bus(&mut m.bus, &view);
    println!("geometrizer mode {:?}", crate::GEO_MODE.get().copied().unwrap_or_default());
    for r in 0..rounds {
        let t0 = Instant::now();
        model2::load_session(&mut m, None, &data)?;
        let t1 = Instant::now();
        let f0 = m.frames;
        while m.frames == f0 {
            m.run_slice(u64::MAX).map_err(|s| format!("{s:?}"))?;
        }
        let t2 = Instant::now();
        if r > 0 {
            println!("round {r}: (CPU only) load_session {:.1} ms | run to the next frame {:.1} ms", ms(t1 - t0), ms(t2 - t1));
            continue;
        }
        count_frames(1);
        let rgb = gpu.frame_bus(&mut m.bus, &view);
        let t3 = Instant::now();
        println!(
            "round {r}: state file {} KB (read {:.1} ms) | load_session {:.1} ms | run to the next frame {:.1} ms | GPU draw at 1080p incl. read-back {:.1} ms | jump to picture {:.1} ms (picture hash {:016x})",
            data.len() / 1024, t_read, ms(t1 - t0), ms(t2 - t1), ms(t3 - t2), ms(t3 - t0), hash(&rgb)
        );
    }
    Ok(())
}
