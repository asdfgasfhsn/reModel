mod depth_check;
mod geo_check;
mod gpu_check;
mod light_report;
mod lighting_check;
mod music;
mod pack_check;
mod pack_lint;
mod recomp_crates;
mod run_ahead;
mod sharc;
mod sound_render;
mod soundtrack;
mod temporal_check;
mod temporal_probe;
mod texpack;
mod texpack_materials;
mod texpack_upscale;
mod tgp2a;
mod tgp_check;
mod tgp_disasm;

use analyze::{explore, read_boot, Image, Trace};
use romset::{games, RomSet};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: remodel [--game ID] [--rom DIR] <command>

options:
  --game ID            game manifest to use (default topskatr)
  --rom DIR            directory of unpacked chips (default rom)
  --geo-trace FRAME    record geometrizer port writes during FRAME of `run`
                       to out/GAME/geo-FRAME.txt (plus buffer RAM dump)
  --height N           snapshot height in pixels (enhancement; default 384)
  --aspect W:H         snapshot aspect ratio, e.g. 16:9 or 21:9 (default 4:3);
                       wider than 4:3 shows more of the 3D scene at the sides
  --hud center|edges   wide views: keep the HUD centred, or move each HUD
                       element to the nearest screen edge
  --aa N               snapshots and the GPU checks (gpu-check, depth-check,
                       pack-check): anti-aliasing with N samples a
                       pixel (4, 8, ...; default off). The software renderer
                       supersamples (draws ceil(sqrt(N)) times larger in each
                       direction and averages); gpu-check turns on the GPU's
                       multisampling to match (4 or 8)
  --depth              snapshots, gpu-check, pack-check, temporal-probe,
                       temporal-check: a depth buffer from each
                       vertex's 1/z, per window, instead of drawing in the
                       game's order (model2::raster::depth_value; default off)
  --input SCRIPT       scripted inputs for `run`, by the game's input names
                       (`remodel --game ID inputs` lists them), e.g. for
                       topskatr coin1@1100-1105,start@1300-1305,curving=0x40@1400-1500;
                       coin-start is the game's own coin-up and start
  --dump-textures DIR  at each `run` snapshot, write every distinct 3D texture
                       and tile pattern drawn to DIR as PNG, plus DIR/index.txt
  --interp             run the i960 on the interpreter even when this build
                       has recompiled code for the game
  --single-thread      run the geometrizer inline instead of on a second
                       thread (as a browser build must); same results
  --audio WAV          `run`: play the sound and music boards live, fed as
                       the game sends, and record the mix to WAV
  --save-state FILE@FRAME
                       `run`: save the machine (and with --audio the sound
                       boards) to FILE when FRAME starts
  --load-state FILE    `run`: start from a save state instead of booting
  --capture DIR        at each `run` snapshot, save what the renderers read
                       (raster stream, video RAMs) to DIR/FRAME/, for
                       `gpu-check`
  --capture-seq DIR    `run` with --geo native: save consecutive frames
                       (--seq FROM,COUNT: COUNT frames from FROM, default
                       0,300) with their motion data to DIR, as a sequence
                       for `temporal-check` (and `gpu-check`); the run stops
                       after the last
  --texture-pack DIR   draw snapshots and gpu-check with the replacement PNGs
                       in DIR (same names as a dump; any size); see
                       docs/texture-packs.md
  --tex-filter off|mip|2|4|8|16
                       filtering of texture pack images in snapshots and
                       gpu-check: off (default; bilinear), mipmaps
                       (trilinear), or mipmaps with up to N-times
                       anisotropic filtering (GPU only: software snapshots
                       draw it as mip)
  --materials          gpu-check, depth-check, pack-check: load the texture
                       pack's material maps onto the GPU (format 2 packs;
                       gpu::Quality::materials). Not drawn yet: pictures
                       are unchanged, gpu-check reports what they hold
  --temporal off|taa|N temporal-probe: temporal anti-aliasing (taa) or
                       upscaling from N% (50 to 99) for the GPU's 3D layer
                       (gpu::Quality::temporal; snapshots are software and
                       ignore it)
  --stipple blend|keep with --temporal: checkerboard translucency as a 50%
                       blend (default) or the pattern kept
  --shading flat|vertex|pixel
                       smooth shading (model2::light; docs/enhancements.md)
                       for snapshots, gpu-check, depth-check, pack-check,
                       temporal-probe and lighting-check: flat (default, the
                       original), vertex (brightness interpolated from
                       values at the corners: the game's formula with
                       smoothed normals where the side channel allows, else
                       means of the stream's lumas) or pixel (the game's
                       formula per pixel where the side channel allows,
                       else drawn as vertex)
  --crease auto|N      the crease angle for --shading in degrees, 0 to 180
                       (0: nothing smoothed; auto, the default: the game's)
  --shading-fine on|off
                       with --shading: blend the two colour columns around
                       a lit pixel's brightness instead of the game's 64
                       levels (no banding on large smooth surfaces; never
                       outside the polygon's hue). Default off
  --shading-balance on|off
                       with --shading: one brightness offset per smooth
                       surface keeps its mean the game's (default on); off
                       leaves the formula at the smoothed normals alone, so
                       bumpy surfaces come out brighter
  --light              record smooth shading's side channel each frame (the
                       native geometrizer's light hooks, or a scan of the
                       list on the chip): snapshots shade with it, `run`
                       reports its self-check over every frame, geo-check
                       compares the list scan with the native hooks; on
                       with --capture and --capture-seq (light.bin)
  --geo chip|native|check
                       the geometrizer: the uploaded program on its chip,
                       the 2B SHARC or 2C TGP (default; `sharc` and `tgp`
                       name it too), the native implementation where it
                       implements the program (docs/hw/geometrizer-2b.md,
                       docs/hw/geometrizer-2c.md), or both, compared frame
                       by frame (see geo-check)
  --tgp2a chip|hle     Model 2A's TGP: the uploaded program on the MB86234
                       interpreter (default), or the function model of
                       srallyc's program it replaced (docs/hw/model2a.md)
  --smooth T,T...      `run` with --geo native: smooth motion's in-between
                       frames (docs/hw/geometrizer.md, Smooth motion); at
                       each snapshot also draw the in-betweens at these
                       fractions (0 to 1) of the way from the frame before
                       to FRAME-tTT.png (and the frame before to
                       FRAME-prev.png), and time one in-between of every
                       frame (the state hash is the same as without)

commands:
  games                list supported games
  inputs               the game's inputs for --input (ports, channels, keys)
  scan                 list the sets in the ROM directory (zips or folders,
                       as MAME keeps them) and check the supported ones
  verify               check every chip against known-good hashes
  boot                 print the boot record and system table seeds
  disasm ADDR [COUNT]  linear disassembly of the program ROM
  analyze [OUT]        recursive-descent listing (default out/GAME/program.lst)
  copro-disasm [FILE]  disassemble an MB86235 program image to FILE.lst
                       (default out/GAME/copro.bin; `run` also writes geo.bin)
  sharc-disasm FILE [OUT]
                       disassemble an ADSP-21062 (Model 2B SHARC) program
                       image (48-bit words, 6 little-endian bytes each,
                       loaded at 0x20000) to OUT (default FILE.lst)
  tgp-disasm [FILE [OUT]]
                       list a Model 2A TGP (MB86234) program image (32-bit
                       little-endian words; default out/GAME/tgp.bin, which
                       `run` writes) with mnemonics, its header, dispatch
                       table and function entry points, to OUT (default
                       FILE.lst)
  tgp-check [FILE]     run a Model 2A TGP program image's functions on the
                       MB86234 interpreter with random arguments and compare
                       them with their maths and with the function model
                       (docs/hw/mb86234.md); fails on a mismatch
  sharc-check FILE...  for each SHARC image: undefined encodings, code
                       discovery from the vectors, loop sanity, and a host
                       boot and run to the first external access
  recomp               recompile the i960 program (code found by `analyze`
                       plus everything `run` executed) and the copro and
                       geometrizer programs `run` captured (MB86235, or
                       ADSP-21062 on Model 2B) into crates in
                       out/recomp/, and the workspace there that builds
                       them in (`make release RECOMP=1`)
  recomp-files         list the program crates in out/recomp/ that belong to
                       the game: its i960 program and the copro and
                       geometrizer programs `run` captured (for per-game
                       builds)
  recomp-workspace [--web DIR PROGRAM...]
                       rewrite out/recomp/'s Cargo workspace (the native
                       recompiled build, with every program there); with
                       --web, a workspace at DIR for the browser module with
                       just the PROGRAMs named (crates in out/recomp/)
  shot FILE [FRAME]    boot, run to FRAME (default: the manifest's attract
                       mode frame) and save the screen to FILE as PNG,
                       touching nothing in out/GAME (--input, --height and
                       --aspect apply)
  web-index [DIR]      list the supported games whose chips are all loose in
                       ROM/<set>/ (as the browser build fetches them; a
                       clone's may be in its parent's); with DIR, also
                       write DIR/games.json for its landing page
  rom-folder           the set folder in ROM with all the game's chips
                       loose: its own, or for a clone its parent's
  gpu-check [DIR]      draw the frames captured with --capture (or a
                       --capture-seq sequence) in DIR
                       (default out/GAME/capture) with the software and GPU
                       rasterisers; time both and report how they differ
  depth-check [DIR] [OUT]
                       draw the frames captured in DIR on the GPU with the
                       depth buffer off and on (default view 1440x1080; with
                       --height, --aspect, --hud that one; --aa for
                       multisampling), report where they differ and write
                       pictures of the differences (off | on | differences)
                       to OUT (default out/GAME/depth-check), and each
                       changed frame's depth-on picture (depth-FRAME-on.png).
                       --probe X,Y@FRAME (repeatable; only those frames):
                       every polygon at that pixel with its draw order,
                       group, object block, 1/z, depth and pass or fail
                       (docs/enhancements.md, Depth buffer); --against
                       DIR: compare the depth-on frames with an earlier
                       run's OUT, pictures in against-FRAME.png
  temporal-check SEQ [OUT]
                       check the motion data of a sequence from --capture-seq
                       without the GPU: whether frame N's list posed with
                       frame N-1's transforms gives frame N-1's positions,
                       and per pixel whether the motion vectors point at the
                       same surface in frame N-1's picture (software
                       renderer); draws a supersampled reference and the
                       baselines temporal anti-aliasing and upscaling are
                       measured against (default view 1080p 16:9; --height,
                       --aspect, --hud that one) every --every N frames
                       (default 10), pictures and report.txt to OUT (default
                       SEQ/check). With the GPU (unless --no-gpu): its
                       velocity target against those vectors, and the plain,
                       MSAA 4x, temporal (taa) and upscaled (67%, 50%)
                       frames drawn through the sequence, scored against the
                       reference with their frame-to-frame change, plus one
                       frame repeated (--tune NAME=V,..: the resolve's
                       constants, gpu::TemporalTuning::parse; --crop
                       X,Y,W,H: enlarged crops of every column; --ref-cache
                       DIR: keep the references between runs; --at F,..:
                       sample those frames too; --gpu-only:
                       skip the CPU checks; --inspect: the error above the
                       plain frame's by the resolve's rule, with
                       FRAME-COL-rules.png and -excess.png; --dump
                       X,Y,W,H[,SCALE]: the resolve's decisions at those
                       pixels on every frame, into OUT/dump.txt (TAA, or
                       the 67/50 column); --depth before the
                       command: the depth buffer in every column). Fails if the
                       motion data misses its marks or temporal scores worse
                       than the plain frame / naive upscale.
                       --bench [N]: time the GPU instead (plain, MSAA 4x,
                       taa, 67, 50) on the first N frames with 3D (default
                       30), and the temporal targets' memory
  pack-check [DIR] [OUT]
                       with --texture-pack: draw the frames captured in DIR
                       (default out/GAME/capture) on the GPU with the pack
                       images unfiltered, mipmapped and 16x anisotropic, in
                       software trilinear and as a supersampled software
                       reference (4x4; --aa N: N GPU samples and at least
                       that many for the reference); write them to OUT
                       (default out/GAME/pack-check) as FRAME-off, -mip,
                       -aniso16, -swmip and -ref.png, and report how far each
                       is from the reference and what they cost.
                       --identity (no --texture-pack needed): a pack
                       standing for the original textures (--texture-pack
                       DIR, normally the dump itself; none: each frame's own
                       textures) drawn on the GPU and in software, unfiltered
                       and mipmapped, against no pack, to OUT (default
                       out/GAME/pack-identity); splits the software's
                       differing pixels by cause and fails a frame on the
                       ones that mean a fault (one texel value, another
                       polygon), not on filter order (up to a ceiling)
  tile-check [DIR] [OUT]
                       with --texture-pack: draw the frames captured in DIR
                       (default out/GAME/capture) with the pack's tile images
                       live on the GPU (default view 1080p 16:9, HUD at the
                       edges; --height, --aspect, --hud) and with the
                       software snapshots' tile replacement over the same 3D
                       layer; write FRAME-gpu, -sw and -orig.png (no pack) to
                       OUT (default out/GAME/tile-check) and report where
                       they differ and what the 2D layers cost
  lighting-check [DIR] [OUT]
                       smooth shading (model2::light) on the frames
                       captured in DIR (default out/GAME/capture): each
                       drawn flat and vertex-shaded (--crease, default the
                       game's) on the software and GPU renderers (default
                       view 1440x1080; --height, --aspect, --hud that one);
                       checks that crease 0 draws the flat frame exactly,
                       that polygons left flat draw as flat, the brightness
                       of lit polygons and software against GPU, and with
                       a side channel (light.bin: `make lighting-capture`)
                       its self-check and the L0 pass against it
                       (--no-side: the stream alone); report.txt
                       and pictures (flat | vertex | differences, false-
                       colour layers) in OUT (default
                       out/scratch/lighting/GAME). --probe X,Y@FRAME
                       (repeatable): the polygons at that output pixel with
                       their corner values; --every N: pictures of every
                       Nth frame only; --bench [N]: GPU frame times flat
                       and vertex at 1080p and 4K over N frames (default
                       8). --shading pixel checks the per-pixel level;
                       --shading-fine on adds finer levels (pictures
                       -fine, -pixel-fine; K: against the game's levels);
                       --shading-balance off checks without the brightness
                       balance (pictures and reports -nobal; C report-only).
                       Fails if an invariant does
  temporal-probe [DIR] [OUT]
                       the GPU's temporal path (--temporal, default taa;
                       --stipple, --depth, --height, --aspect) on the frames
                       captured in DIR (default out/GAME/capture): the first
                       frame at 100% with the pattern kept must be the frame
                       without, byte for byte; writes off, jittered, blend
                       and keep pictures to OUT (default
                       out/scratch/temporal-probe)
  temporal-probe live FROM COUNT OUT
                       run (with --input) to frame FROM with the native
                       geometrizer and smooth motion's recording, then draw
                       COUNT frames with temporal on: the picture, a picture
                       of the motion vectors and the aux target's statistics
                       per frame, in OUT
  geo-check [STEPS] [FROM]
                       run (with --input, --load-state) with the geometrizer
                       chip (SHARC or TGP) and the native geometrizer fed the
                       same lists, compare their raster streams word by word
                       each frame and report the first difference; time both
                       (mean and worst, overall and from frame FROM)
  geo-fuzz [FRAMES] [SEED]
                       random display lists (every command, model paths no
                       game reaches) through the game's uploaded geometrizer
                       program on its chip and the native one; report the
                       first difference
  latency FRAME CHANGE [FRAMES]
                       input latency: two runs from FRAME (with --input,
                       or --load-state), one with CHANGE (an --input item
                       without frames, e.g. wheel=0xF0) held from FRAME;
                       for each frame after, when the game read it and
                       when its RAM, display list, polygons and picture
                       differ; the frames to the first picture difference
                       are the game's input lag (`make input-lag`)
  run-ahead-check [STEPS] [FRAMES] [EVERY]
                       run-ahead (docs/play.md): a plain run (default 450M
                       instructions, with --input, --load-state, --geo,
                       --smooth) beside one running FRAMES (default 1)
                       ahead; the real timeline's state hash must be the
                       same, and every EVERY-th frame (default 50) whose
                       inputs hold, the frame shown must be the plain run's
                       FRAMES later (picture, state hash, in-betweens);
                       times the copy and the frames ahead
  lockstep [STEPS]     run the recompiled i960 code and the interpreter side
                       by side, block by block; report the first difference
                       (Model 2B: also the SHARCs, the copro after every
                       block, both whole after every frame)
  run [STEPS] [SNAP]   boot on the reference interpreter and report what
                       the game did (default 50M instructions); writes the
                       tile layer to out/GAME/frames/ every SNAP frames
                       (0 = final frame only), and the uploaded copro and
                       geometrizer programs (copro.bin, geo.bin; on Model
                       2B sharc-copro.bin, sharc-geo.bin for sharc-disasm)
  sound-disasm ADDR [COUNT]
                       disassemble the sound board's 68000 program (its own
                       address space: ROM at 0x600000)
  sound-render [WAV] [SECONDS]
                       boot the sound board, replay the host bytes captured
                       by `run` (out/GAME/sound-tx.txt) and render the SCSP
                       output (default out/GAME/sound.wav; length: until 3 s
                       after the last byte)
  music-render [--soundtrack DIR] [WAV] [SECONDS]
                       replay out/GAME/sound-tx.txt (from `run`) into the
                       DSB2 MPEG music board; writes WAV (default
                       out/GAME/music.wav, 44.1 kHz stereo). With
                       --soundtrack, tracks listed in DIR/tracks.txt play
                       from their files (docs/mods.md)
  soundtrack-import SRC_DIR NAME
                       copy the audio files in SRC_DIR into soundtracks/NAME
                       (NAME with a `/` is used as a path), converting
                       anything but WAV and MP2 to WAV with ffmpeg, and
                       write a tracks.txt numbered from the file names
  texpack-upscale SRC DEST [--scale N] [--upscaler CMD] [--tiles MODE]
                  [--colour match|keep] [--min-uses N] [--max-side N]
                  [--drift N] [--work DIR] [--force] [--keep-work]
                       make texture pack DEST from dump SRC (from
                       --dump-textures; no ROM needed): transparent pixels
                       filled, borders padded as the texture wraps, run
                       through CMD (`{in}`, `{out}`, `{scale}` filled in;
                       default: built-in bicubic), cropped, the dump's
                       low-frequency colour put back (--colour keep: off),
                       alpha cut at 0.5.
                       Tiles (MODE): model, builtin, nearest or skip. Only
                       new or changed dumps; DEST/index.txt lists the pack
                       (docs/texture-packs.md)
  texpack-materials PACK [--dump DIR] [--normalgen CMD] [--normal-y up|down]
                  [--strength X] [--detail X] [--min-uses N] [--work DIR]
                  [--force] [--keep-work]
                       make normal maps (.n) for texture pack PACK from its
                       images, or dump DIR's where it has none (no ROM
                       needed): per surface (surf=) or image, wrap-aware,
                       neutral under cut-outs; built-in from luminance
                       (strength, detail in texels) or CMD (`{in}`, `{out}`
                       filled in; --normal-y: its green). Only new or
                       changed sources; hand-made or edited maps are kept.
                       A starting point to edit by hand
                       (docs/texture-packs.md, Material maps)
  pack-lint PACK [--dump DIR] [--max-side N]
                       check texture pack PACK against the pack format (no
                       ROM, no GPU): pack.txt, file names, index.txt, that
                       every PNG decodes within N pixels a side (default
                       8192), proportions of images and material maps, GPU
                       memory against the budgets; with --dump, files whose
                       key the dump never drew. Exits non-zero on errors
                       (docs/texture-packs.md, Pack format)
  mpeg-dump ADDR [SECONDS]
                       decode MPEG ROM frames from ADDR directly to
                       out/GAME/mpeg-ADDR.{wav,f32,mp2} (default 10 s)

debugging (environment variables for `run`):
  REMODEL_PROBE=X,Y    at each snapshot, list the 3D polygons covering
                       pixel (X, Y) with their texture header and texels
  REMODEL_WATCH=ADDR   log every memory write to ADDR's 32-bit word
";

fn parse_num(s: &str) -> Option<u32> {
    match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

fn main() -> ExitCode {
    recomp::register();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.is_empty() {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    }
    let mut opt = |flag: &str| -> Result<Option<String>, ()> {
        let Some(i) = args.iter().position(|a| a == flag) else { return Ok(None) };
        if i + 1 >= args.len() {
            return Err(());
        }
        let v = args.remove(i + 1);
        args.remove(i);
        Ok(Some(v))
    };
    let (Ok(game_id), Ok(rom_dir), Ok(input), Ok(geo_frame)) =
        (opt("--game"), opt("--rom"), opt("--input"), opt("--geo-trace"))
    else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let (Ok(height), Ok(aspect), Ok(hud)) = (opt("--height"), opt("--aspect"), opt("--hud")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if height.is_some() || aspect.is_some() || hud.is_some() {
        let height = height.and_then(|h| h.parse().ok()).unwrap_or(model2::video::HEIGHT);
        let aspect = match aspect.as_deref().map(|a| a.split_once(':').unwrap_or((a, "1"))) {
            Some((w, h)) => w.parse::<f32>().unwrap_or(4.0) / h.parse::<f32>().unwrap_or(3.0),
            None => 4.0 / 3.0,
        };
        let hud = if hud.as_deref() == Some("edges") { model2::video::Hud::Edges } else { model2::video::Hud::Center };
        let mut view = model2::video::View::new(height, aspect, hud);
        if let Ok(g) = games::find(game_id.as_deref().unwrap_or("topskatr")) {
            view = view.with_hud_windows(g.hud_windows, g.hud_element_windows);
        }
        let _ = VIEW.set(view);
    }
    let Ok(aa) = opt("--aa") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(a) = aa {
        let Some(n) = a.parse::<u32>().ok().filter(|n| (1..=64).contains(n)) else {
            eprintln!("error: --aa wants a number of samples a pixel, 1 to 64");
            return ExitCode::FAILURE;
        };
        let _ = AA.set(n);
    }
    let (Ok(dump_textures), Ok(texture_pack)) = (opt("--dump-textures"), opt("--texture-pack")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let pack_dir = texture_pack.clone().map(PathBuf::from);
    let Ok(tex_filter_opt) = opt("--tex-filter") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(f) = tex_filter_opt {
        let Some(f) = model2::enhance::TexFilter::parse(&f) else {
            eprintln!("error: --tex-filter wants off, mip, 2, 4, 8 or 16");
            return ExitCode::FAILURE;
        };
        let _ = TEX_FILTER.set(f);
    }
    let (Ok(temporal), Ok(stipple)) = (opt("--temporal"), opt("--stipple")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(t) = temporal {
        let Some(t) = gpu::Quality::parse_temporal(&t) else {
            eprintln!("error: --temporal wants off, taa or a render scale from 50 to 100");
            return ExitCode::FAILURE;
        };
        let _ = TEMPORAL.set(t);
    }
    if let Some(s) = stipple {
        let Some(s) = gpu::Stipple::parse(&s) else {
            eprintln!("error: --stipple wants blend or keep");
            return ExitCode::FAILURE;
        };
        let _ = STIPPLE.set(s);
    }
    let (Ok(shading), Ok(crease), Ok(fine), Ok(balance)) =
        (opt("--shading"), opt("--crease"), opt("--shading-fine"), opt("--shading-balance"))
    else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let fine = match fine.as_deref() {
        None | Some("off") => false,
        Some("on") => true,
        Some(_) => {
            eprintln!("error: --shading-fine wants on or off");
            return ExitCode::FAILURE;
        }
    };
    let _ = SHADING_FINE.set(fine);
    let balance = match balance.as_deref() {
        None | Some("on") => true,
        Some("off") => false,
        Some(_) => {
            eprintln!("error: --shading-balance wants on or off");
            return ExitCode::FAILURE;
        }
    };
    let _ = SHADING_BALANCE.set(balance);
    if shading.is_some() || crease.is_some() {
        let Some(s) = model2::light::Shading::parse(shading.as_deref().unwrap_or("vertex")) else {
            eprintln!("error: --shading wants flat, vertex or pixel");
            return ExitCode::FAILURE;
        };
        let Some(c) = model2::light::parse_crease(crease.as_deref().unwrap_or("auto")) else {
            eprintln!("error: --crease wants auto or degrees from 0 to 180");
            return ExitCode::FAILURE;
        };
        let _ = SHADING.set((if shading.is_some() { s } else { model2::light::Shading::Flat }, c));
    }
    let _ = GAME_ID.set(game_id.clone().unwrap_or_else(|| "topskatr".into()));
    if let Err(e) = texpack::configure(dump_textures, texture_pack) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let game_id = game_id.unwrap_or_else(|| "topskatr".into());
    let rom_dir = PathBuf::from(rom_dir.unwrap_or_else(|| "rom".into()));
    let Ok(audio) = opt("--audio") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(a) = audio {
        let _ = AUDIO.set(PathBuf::from(a));
    }
    let (Ok(save_state), Ok(load_state)) = (opt("--save-state"), opt("--load-state")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(v) = save_state {
        let Some((file, frame)) = v.rsplit_once('@').and_then(|(f, n)| Some((f, n.parse::<u64>().ok()?))) else {
            eprintln!("error: --save-state wants FILE@FRAME");
            return ExitCode::FAILURE;
        };
        let _ = SAVE_STATE.set((PathBuf::from(file), frame));
    }
    if let Some(f) = load_state {
        let _ = LOAD_STATE.set(PathBuf::from(f));
    }
    let Ok(capture) = opt("--capture") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(c) = capture {
        let _ = CAPTURE.set(PathBuf::from(c));
    }
    let (Ok(capture_seq), Ok(seq)) = (opt("--capture-seq"), opt("--seq")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(c) = capture_seq {
        let range = match seq.as_deref().map(|v| v.split_once(',').map(|(a, b)| (a.parse::<u64>(), b.parse::<u64>()))) {
            None => (0, 300),
            Some(Some((Ok(a), Ok(b)))) => (a, b),
            _ => {
                eprintln!("error: --seq wants FROM,COUNT (frames)");
                return ExitCode::FAILURE;
            }
        };
        let _ = CAPTURE_SEQ.set((PathBuf::from(c), range.0, range.1));
    }
    let Ok(geo_mode) = opt("--geo") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(g) = geo_mode {
        let Some(mode) = model2::geo::GeoMode::parse(&g) else {
            eprintln!("error: --geo wants chip, native or check");
            return ExitCode::FAILURE;
        };
        let _ = GEO_MODE.set(mode);
    }
    let Ok(tgp2a_mode) = opt("--tgp2a") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    match tgp2a_mode.as_deref() {
        None | Some("chip") => {}
        Some("hle") => {
            let _ = TGP2A_HLE.set(true);
        }
        Some(_) => {
            eprintln!("error: --tgp2a wants chip or hle");
            return ExitCode::FAILURE;
        }
    }
    let Ok(smooth) = opt("--smooth") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if let Some(v) = smooth {
        let ts: Option<Vec<f32>> =
            v.split(',').map(|t| t.trim().parse::<f32>().ok().filter(|t| (0.0..=1.0).contains(t))).collect();
        let Some(ts) = ts else {
            eprintln!("error: --smooth wants fractions from 0 to 1, e.g. 0.25,0.5,0.75");
            return ExitCode::FAILURE;
        };
        if GEO_MODE.get() != Some(&model2::geo::GeoMode::Native) {
            eprintln!("error: --smooth needs --geo native (in-betweens come from the native geometrizer)");
            return ExitCode::FAILURE;
        }
        let _ = SMOOTH.set(ts);
    }
    if CAPTURE_SEQ.get().is_some() && GEO_MODE.get() != Some(&model2::geo::GeoMode::Native) {
        eprintln!("error: --capture-seq needs --geo native (motion data comes from the native geometrizer)");
        return ExitCode::FAILURE;
    }
    let Ok(soundtrack) = opt("--soundtrack") else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let mut flag = |name: &str| match args.iter().position(|a| a == name) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    let (interp, single_thread) = (flag("--interp"), flag("--single-thread"));
    if flag("--depth") {
        let _ = DEPTH.set(true);
    }
    if flag("--materials") {
        let _ = MATERIALS.set(true);
    }
    if flag("--light") || CAPTURE.get().is_some() || CAPTURE_SEQ.get().is_some() {
        let _ = LIGHT.set(true);
    }
    let Some(cmd) = args.first().cloned() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if cmd == "soundtrack-import" {
        let (Some(src), Some(name)) = (args.get(1), args.get(2)) else {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        };
        let dest = if name.contains('/') { PathBuf::from(name) } else { PathBuf::from("soundtracks").join(name) };
        return match soundtrack::import(Path::new(src), &dest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "texpack-upscale" {
        return match texpack_upscale::main(&args[1..], &game_id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "texpack-materials" {
        return match texpack_materials::main(&args[1..], &game_id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "pack-lint" {
        return match pack_lint::main(&args[1..]) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "sharc-disasm" || cmd == "sharc-check" {
        let r = match (cmd.as_str(), args.get(1)) {
            ("sharc-disasm", Some(file)) => {
                let out = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{file}.lst")));
                sharc::disasm(Path::new(file), &out)
            }
            ("sharc-check", Some(_)) => sharc::check(&args[1..], 10_000_000),
            _ => {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            }
        };
        return match r {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "temporal-check" {
        let Some(root) = args.get(1).map(PathBuf::from) else {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        };
        let every = match args.iter().position(|a| a == "--every") {
            Some(i) => match args.get(i + 1).and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0) {
                Some(n) => {
                    args.drain(i..i + 2);
                    n
                }
                None => {
                    eprintln!("error: --every wants a number of frames");
                    return ExitCode::FAILURE;
                }
            },
            None => 10,
        };
        // Flags: --no-gpu, --gpu-only, --bench [FRAMES], --tune K=V,..., --crop X,Y,W,H
        // (repeatable), --ref-cache DIR.
        let mut take = |name: &str| -> Option<Option<String>> {
            let i = args.iter().position(|a| a == name)?;
            let v = args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
            args.drain(i..i + 1 + v.is_some() as usize);
            Some(v)
        };
        let no_gpu = take("--no-gpu").is_some();
        let gpu_only = take("--gpu-only").is_some();
        let bench = take("--bench");
        let tune = take("--tune").flatten();
        let ref_cache = take("--ref-cache").flatten().map(PathBuf::from);
        let at: Vec<u64> =
            take("--at").flatten().unwrap_or_default().split(',').filter_map(|x| x.parse().ok()).collect();
        let inspect = take("--inspect").is_some();
        let dump = match take("--dump")
            .map(|d| d.unwrap_or_default().split(',').filter_map(|x| x.parse().ok()).collect::<Vec<usize>>())
        {
            None => None,
            Some(v) => match v[..] {
                [x, y, w, h] => Some((x, y, w, h, 100)),
                [x, y, w, h, s @ (100 | 67 | 50)] => Some((x, y, w, h, s)),
                _ => {
                    eprintln!("error: --dump wants X,Y,W,H[,SCALE] in output pixels (scale 100, 67 or 50)");
                    return ExitCode::FAILURE;
                }
            },
        };
        let mut crops = Vec::new();
        while let Some(c) = take("--crop") {
            let v: Vec<usize> = c.unwrap_or_default().split(',').filter_map(|x| x.parse().ok()).collect();
            let [x, y, w, h] = v[..] else {
                eprintln!("error: --crop wants X,Y,W,H in output pixels");
                return ExitCode::FAILURE;
            };
            crops.push((x, y, w, h));
        }
        let tuning = match tune.map(|t| gpu::TemporalTuning::parse(&t)).transpose() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: --tune: {e}");
                return ExitCode::FAILURE;
            }
        };
        let out = args.get(2).map(PathBuf::from).unwrap_or_else(|| root.join("check"));
        let view = VIEW
            .get()
            .copied()
            .unwrap_or_else(|| model2::video::View::new(1080, 16.0 / 9.0, model2::video::Hud::Edges));
        if let Some(n) = bench {
            let frames = n.and_then(|n| n.parse().ok()).unwrap_or(30);
            return match temporal_check::bench(&root, &view, frames, 8) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        let depth = DEPTH.get().copied().unwrap_or(false);
        let opts = temporal_check::Options {
            view,
            every,
            out,
            gpu: !no_gpu,
            gpu_only,
            tuning,
            crops,
            ref_cache,
            at,
            depth,
            inspect,
            dump,
        };
        return match temporal_check::run(&root, &opts) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "tgp-disasm" {
        // A 2A game's uploaded TGP program (`run` writes out/GAME/tgp.bin).
        let file =
            args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("out").join(&game_id).join("tgp.bin"));
        let out = args.get(2).map(PathBuf::from).unwrap_or_else(|| file.with_extension("lst"));
        return match tgp_disasm::disasm(&file, &out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if cmd == "scan" {
        let sets = romset::sets_with_clones(&rom_dir);
        if sets.is_empty() {
            eprintln!("no sets in {} (expected <set>.zip or <set>/)", rom_dir.display());
            return ExitCode::FAILURE;
        }
        let mut supported = 0;
        for set in &sets {
            match games::find(set) {
                Ok(g) => {
                    supported += 1;
                    let state = match RomSet::load(g, &rom_dir, true) {
                        Ok(_) => "ok".to_string(),
                        Err(e) => format!("bad: {e}"),
                    };
                    println!("{set:<12} {:<34} {state}", g.title);
                }
                Err(_) => println!("{set:<12} {:<34} not supported yet", "-"),
            }
        }
        println!("{} sets, {supported} supported", sets.len());
        return ExitCode::SUCCESS;
    }
    if cmd == "web-index" {
        let games = web_index(&rom_dir);
        if let Some(dir) = args.get(1).map(PathBuf::from) {
            let path = dir.join("games.json");
            if let Err(e) = std::fs::write(&path, games_json(&games, &dir)) {
                eprintln!("error: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {} ({} games)", path.display(), games.len());
        }
        for g in games {
            println!("{}", g.id);
        }
        return ExitCode::SUCCESS;
    }
    if cmd == "recomp-workspace" {
        let out = Path::new(recomp_crates::DIR);
        let r = match args.get(1).map(String::as_str) {
            None => recomp_crates::programs(out)
                .and_then(|names| recomp_crates::write_workspace(out, out, &names, recomp_crates::Front::Native)),
            Some("--web") if args.len() > 2 => {
                recomp_crates::write_workspace(Path::new(&args[2]), out, &args[3..], recomp_crates::Front::Web)
            }
            _ => {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = r {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if cmd == "rom-folder" {
        // The set folder in --rom with all of the game's chips (its own or
        // its parent's), for the browser build's pages.
        let Ok(g) = games::find(&game_id) else {
            eprintln!("error: unknown game '{game_id}'");
            return ExitCode::FAILURE;
        };
        return match romset::rom_folder(g, &rom_dir) {
            Some(set) => {
                println!("{set}");
                ExitCode::SUCCESS
            }
            None => ExitCode::FAILURE,
        };
    }
    if cmd == "games" {
        for g in games::ALL {
            // Last, the set it is a clone of ("-" for none).
            println!("{:<10} {:<20} {:<12} {}", g.id, g.title, format!("{:?}", g.board), g.parent.unwrap_or("-"));
        }
        return ExitCode::SUCCESS;
    }
    if cmd == "inputs" {
        let Ok(g) = games::find(&game_id) else {
            eprintln!("error: unknown game '{game_id}'");
            return ExitCode::FAILURE;
        };
        let c = &g.controls;
        println!("{} inputs (name: port bit / channel centre):", g.id);
        for b in c.buttons {
            println!("  {:<10} port {} bit 0x{:02x}", b.name, b.port, b.bit);
        }
        for a in c.analogs {
            println!("  {:<10} analog channel {}, centre 0x{:02x}", a.name, a.channel, a.centre);
        }
        println!("coin-start = {}", c.coin_start);
        println!("player bindings (keys / pad):\n{}", romset::Bindings::defaults(c).describe().trim_end());
        return ExitCode::SUCCESS;
    }
    let verify = cmd == "verify";
    let roms = match games::find(&game_id).and_then(|g| RomSet::load(g, &rom_dir, verify)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let img = Image { base: 0, bytes: &roms.program };

    match cmd.as_str() {
        "verify" => {
            for r in roms.game.files {
                println!("ok  {:<14} {:>8x}  {:?}", r.name, r.size, r.role);
            }
            println!("{}: all {} chips match", roms.game.id, roms.game.files.len());
        }
        "boot" => {
            let Some(b) = read_boot(&img) else {
                eprintln!("error: boot record out of range");
                return ExitCode::FAILURE;
            };
            println!("prcb        {:#x}", b.prcb);
            println!("reset ip    {:#x}", b.reset_ip);
            println!("int table   {:#x}", b.int_table);
            println!("sysproc     {:#x}", b.sysproc_table);
            println!("fault table {:#x}", b.fault_table);
            let mut by_target: BTreeMap<u32, Vec<String>> = BTreeMap::new();
            for (k, a) in &b.seeds {
                by_target.entry(*a).or_default().push(format!("{k:?}"));
            }
            for (a, ks) in by_target {
                let shown = if ks.len() > 4 {
                    format!("{} .. {} ({} entries)", ks[0], ks[ks.len() - 1], ks.len())
                } else {
                    ks.join(", ")
                };
                println!("  {a:#08x}  {shown}");
            }
        }
        "tgp-check" => {
            // A 2A game's uploaded TGP program on the interpreter, with the
            // board's function ROM.
            let file =
                args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("out").join(&game_id).join("tgp.bin"));
            let bytes = match std::fs::read(&file) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: {}: {e} (`make run GAME={game_id}` writes it)", file.display());
                    return ExitCode::FAILURE;
                }
            };
            let program: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
            let tables: Vec<u32> = roms
                .bank(romset::Role::CoproData)
                .as_chunks::<4>()
                .0
                .iter()
                .take(0x1_0000)
                .map(|c| u32::from_le_bytes(*c))
                .collect();
            match tgp_check::check(&program, tables) {
                Ok(true) => {}
                Ok(false) => return ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "disasm" => {
            let Some(mut pc) = args.get(1).and_then(|s| parse_num(s)) else {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            };
            let count = args.get(2).and_then(|s| parse_num(s)).unwrap_or(32);
            for _ in 0..count {
                let i = img.decode(pc);
                println!("{:08x}  {:08x}  {i}", i.addr, i.word);
                pc = i.next();
            }
        }
        "analyze" => {
            let default_out = format!("out/{}/program.lst", roms.game.id);
            let out = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(&default_out));
            let Some(b) = read_boot(&img) else {
                eprintln!("error: boot record out of range");
                return ExitCode::FAILURE;
            };
            let seeds: Vec<u32> = b.seeds.iter().map(|s| s.1).collect();
            // Use the interpreter's trace from `remodel run`, when present.
            let dir = PathBuf::from("out").join(roms.game.id);
            let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
            let trace = Trace::parse(&read("coverage.txt"), &read("indirect.txt"));
            let a = explore(&img, &seeds, &trace);

            let mut text = String::new();
            for (addr, i) in &a.insns {
                if a.functions.contains(addr) {
                    let _ = writeln!(text, "\nsub_{addr:x}:");
                } else if a.labels.contains(addr) {
                    let _ = writeln!(text, "loc_{addr:x}:");
                }
                let _ = writeln!(text, "  {addr:08x}  {i}");
            }
            if let Some(dir) = out.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&out, text) {
                eprintln!("error: {}: {e}", out.display());
                return ExitCode::FAILURE;
            }

            let code_bytes: u32 = a.insns.values().map(|i| i.size).sum();
            println!("instructions  {}", a.insns.len());
            println!(
                "code bytes    {code_bytes:#x} ({:.1}% of {:#x})",
                100.0 * code_bytes as f64 / roms.program.len() as f64,
                roms.program.len()
            );
            println!("functions     {} ({} via lda code pointers)", a.functions.len(), a.speculative.len());
            println!("indirect      {} (jump tables / function pointers to resolve)", a.indirect.len());
            if !trace.executed.is_empty() {
                let confirmed = trace.executed.iter().filter(|e| a.insns.contains_key(e)).count();
                let spec_hit = a.speculative.iter().filter(|p| trace.executed.contains(p)).count();
                println!(
                    "trace         {} executed, {} in listing; {} indirect pairs; {} of {} pointer guesses ran",
                    trace.executed.len(),
                    confirmed,
                    trace.indirect.len(),
                    spec_hit,
                    a.speculative.len()
                );
            }
            println!("invalid stops {}", a.invalid.len());
            for &p in a.invalid.iter().take(8) {
                println!("  invalid at {p:#x}");
            }

            // Bucket static data references by 1MB region: a first, empirical
            // memory map of what the game touches.
            let mut regions: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
            for (&t, &n) in &a.data_refs {
                let e = regions.entry(t >> 20).or_default();
                e.0 += 1;
                e.1 += n;
            }
            println!("static data references by 1MB region:");
            for (r, (addrs, refs)) in regions {
                println!("  {:#010x}  {addrs:>5} addrs  {refs:>6} refs", r << 20);
            }
            println!("listing written to {}", out.display());
        }
        "copro-disasm" => {
            let dir = PathBuf::from("out").join(roms.game.id);
            let input = args.get(1).map(PathBuf::from).unwrap_or_else(|| dir.join("copro.bin"));
            let Ok(bin) = std::fs::read(&input) else {
                eprintln!("error: {}: not found (run `make run` first)", input.display());
                return ExitCode::FAILURE;
            };
            copro_disasm(&bin, &input.with_extension("lst"));
        }
        "recomp" => {
            let dir = PathBuf::from("out").join(roms.game.id);
            let out = PathBuf::from(recomp_crates::DIR);
            let _ = std::fs::create_dir_all(&out);
            let mut made = 0;
            {
                let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
                let trace = Trace::parse(&read("coverage.txt"), &read("indirect.txt"));
                if trace.executed.is_empty() {
                    eprintln!("i960: no coverage.txt in {} (run `make run` first)", dir.display());
                }
                let Some(b) = read_boot(&img) else {
                    eprintln!("error: boot record out of range");
                    return ExitCode::FAILURE;
                };
                let seeds: Vec<u32> = b.seeds.iter().map(|s| s.1).collect();
                let a = explore(&img, &seeds, &trace);
                // Only code the interpreter ran or control flow reaches for
                // certain; `lda` pointer guesses may be data.
                let mut insns = a.insns;
                insns.retain(|addr, _| !a.speculative.contains(addr) || trace.executed.contains(addr));
                // Instructions are word aligned: discovery can decode data
                // at an odd address (schamp's 0x20E), and a block there
                // would take the table slot of the word it sits in.
                insns.retain(|addr, _| addr & 3 == 0);
                // Code run from RAM (waverunr copies routines to work RAM)
                // stays on the interpreter: only program ROM is compiled.
                for &addr in trace.executed.iter().filter(|&&a| img.contains(a)) {
                    insns.entry(addr).or_insert_with(|| img.decode(addr));
                }
                let mut entries: BTreeSet<u32> = a.functions.iter().copied().collect();
                entries.extend(trace.indirect.iter().map(|&(_, t)| t).filter(|&t| img.contains(t)));
                entries.extend(seeds);
                let hash = model2::program_hash(&roms);
                let code = i960::codegen::program_module(&insns, &entries, hash);
                let path = match recomp_crates::write_program(&out, &format!("i960_{hash:016x}"), &code) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                let blocks = code.matches("\nfn b").count();
                println!("i960: {} instructions in {blocks} blocks -> {}", insns.len(), path.display());
                made += 1;
            }
            // Model 2B's copro and geometrizer are SHARCs: their programs
            // are ADSP-21062 code.
            let sharc = model2::geo::Core::of(roms.game.board) == model2::geo::Core::Sharc;
            let tgp_programs: &[&str] = if sharc { &[] } else { &["copro", "geo"] };
            for name in ["copro", "geo"].into_iter().filter(|_| sharc) {
                let p = match sharc_program(&dir, name) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("{name}: {e} (run `make run` first)");
                        continue;
                    }
                };
                let (prog, words, found, ran) = (&p.image, &p.words, &p.found, &p.ran);
                let extra = ran.difference(found).count();
                for note in adsp2106x::codegen::check(prog, words) {
                    println!("{name}: {note}");
                }
                let code = adsp2106x::codegen::program_module(prog, words);
                let krate = p.krate();
                // Crates were named after the whole image before they were
                // named after their code: that one is out of date.
                let old = out.join(format!("sharc_{:016x}", adsp2106x::cpu::program_hash(prog)));
                if old != out.join(&krate) && old.is_dir() {
                    let _ = std::fs::remove_dir_all(&old);
                }
                let path = match recomp_crates::add_role(&out, &krate, name)
                    .and_then(|()| recomp_crates::write_program(&out, &krate, &code))
                {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                println!(
                    "{name}: SHARC, {} of {} words recompiled ({} found statically, {extra} more from {} executed) -> {}",
                    words.len(),
                    prog.len(),
                    found.len(),
                    ran.len(),
                    path.display()
                );
                made += 1;
            }
            for &name in tgp_programs {
                let (Ok(bin), Ok(cov)) = (
                    std::fs::read(dir.join(format!("{name}.bin"))),
                    std::fs::read_to_string(dir.join(format!("{name}-coverage.txt"))),
                ) else {
                    eprintln!(
                        "{name}: no {name}.bin / {name}-coverage.txt in {} (run `make run` first)",
                        dir.display()
                    );
                    continue;
                };
                let prog: Vec<u64> = bin.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
                let words: Vec<u32> = cov.lines().filter_map(|l| u32::from_str_radix(l.trim(), 16).ok()).collect();
                let hash = mb86235::cpu::program_hash(&prog);
                let code = mb86235::codegen::program_module(&prog, &words);
                let path = match recomp_crates::write_program(&out, &format!("tgp_{hash:016x}"), &code) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                println!("{name}: {} of {} words recompiled -> {}", words.len(), prog.len(), path.display());
                made += 1;
            }
            if made == 0 {
                return ExitCode::FAILURE;
            }
            // Every game's programs, not just this one's: the native build
            // has them all.
            if let Err(e) = recomp_crates::programs(&out)
                .and_then(|names| recomp_crates::write_workspace(&out, &out, &names, recomp_crates::Front::Native))
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
            println!("workspace: {}/Cargo.toml", out.display());
        }
        "recomp-files" => {
            let dir = PathBuf::from("out").join(roms.game.id);
            let mut hashes = vec![format!("i960_{:016x}", model2::program_hash(&roms))];
            let sharc = model2::geo::Core::of(roms.game.board) == model2::geo::Core::Sharc;
            for name in ["copro", "geo"] {
                if sharc {
                    if let Ok(p) = sharc_program(&dir, name) {
                        hashes.push(p.krate());
                    }
                } else if let Ok(bin) = std::fs::read(dir.join(format!("{name}.bin"))) {
                    let prog: Vec<u64> = bin.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
                    hashes.push(format!("tgp_{:016x}", mb86235::cpu::program_hash(&prog)));
                }
            }
            hashes.dedup();
            let out = Path::new(recomp_crates::DIR);
            // Moves loose files from the older layout into crates.
            let have = recomp_crates::programs(out).unwrap_or_default();
            for f in hashes.iter().filter(|f| have.contains(f)) {
                println!("{f}");
            }
        }
        "shot" => {
            let Some(path) = args.get(1).map(PathBuf::from) else {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            };
            let frame = args.get(2).and_then(|s| parse_num(s)).map_or(roms.game.attract_frame, u64::from);
            let script = match model2::input::InputScript::parse(input.as_deref().unwrap_or(""), &roms.game.controls) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: --input: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mut m = machine(&roms);
            if interp {
                m.disable_native();
            }
            m.bus.geo.set_threaded(!single_thread);
            while m.frames < frame {
                let last = m.frames;
                if let Err(s) = m.run_slice(u64::MAX) {
                    eprintln!("error: stopped at frame {}: {s:?}", m.frames);
                    return ExitCode::FAILURE;
                }
                if m.frames != last {
                    script.apply(m.frames, &mut m.bus);
                }
            }
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            snapshot(&mut m, &path);
            println!("frame {frame} -> {}", path.display());
        }
        "gpu-check" => {
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            let (aa, depth) = (AA.get().copied().unwrap_or(1), DEPTH.get().copied().unwrap_or(false));
            let light = light_options(roms.game);
            if let Err(e) = gpu_check::run(&root, windows, aa, depth, pack_dir.as_deref(), tex_filter(), light.as_ref())
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "temporal-probe" => {
            let quality = gpu::Quality {
                depth: DEPTH.get().copied().unwrap_or(false),
                temporal: TEMPORAL.get().copied().unwrap_or(100),
                stipple: STIPPLE.get().copied().unwrap_or_default(),
                ..Default::default()
            };
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            let view = VIEW.get().copied();
            let result = if args.get(1).map(String::as_str) == Some("live") {
                let (Some(from), Some(count), Some(out)) =
                    (args.get(2).and_then(|s| parse_num(s)), args.get(3).and_then(|s| parse_num(s)), args.get(4))
                else {
                    eprint!("{USAGE}");
                    return ExitCode::FAILURE;
                };
                match model2::input::InputScript::parse(input.as_deref().unwrap_or(""), &roms.game.controls) {
                    Ok(script) => {
                        let mut m = machine(&roms);
                        m.bus.geo.set_threaded(!single_thread);
                        temporal_probe::live(
                            &mut m,
                            &script,
                            from.into(),
                            count.into(),
                            windows,
                            view,
                            quality,
                            Path::new(out),
                        )
                    }
                    Err(e) => Err(format!("--input: {e}")),
                }
            } else {
                let default = PathBuf::from("out").join(roms.game.id).join("capture");
                let root = args.get(1).map(PathBuf::from).unwrap_or(default);
                let out = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("out/scratch/temporal-probe"));
                let light = light_options(roms.game);
                let lq = gpu_light(light.as_ref());
                let quality = gpu::Quality {
                    shading: lq.shading,
                    shading_fine: lq.shading_fine,
                    shading_balance: lq.shading_balance,
                    ..quality
                };
                temporal_probe::captures(&root, windows, view, quality, &out, light.as_ref())
            };
            if let Err(e) = result {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "pack-check" if args.iter().any(|a| a == "--identity") => {
            args.retain(|a| a != "--identity");
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let out = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out").join(roms.game.id).join("pack-identity"));
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            let (aa, depth) = (AA.get().copied().unwrap_or(1), DEPTH.get().copied().unwrap_or(false));
            if let Err(e) =
                pack_check::identity(&root, windows, VIEW.get().copied(), aa, depth, pack_dir.as_deref(), &out)
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "pack-check" => {
            let Some(pack) = &pack_dir else {
                eprintln!("error: pack-check needs --texture-pack DIR");
                return ExitCode::FAILURE;
            };
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let out = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out").join(roms.game.id).join("pack-check"));
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            let (aa, depth) = (AA.get().copied().unwrap_or(1), DEPTH.get().copied().unwrap_or(false));
            let light = light_options(roms.game);
            if let Err(e) = pack_check::run(&root, windows, VIEW.get().copied(), aa, depth, pack, &out, light.as_ref())
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "tile-check" => {
            let Some(pack) = &pack_dir else {
                eprintln!("error: tile-check needs --texture-pack DIR");
                return ExitCode::FAILURE;
            };
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let out = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out").join(roms.game.id).join("tile-check"));
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            let light = light_options(roms.game);
            if let Err(e) = pack_check::run_tiles(&root, windows, VIEW.get().copied(), pack, &out, light.as_ref()) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "depth-check" => {
            let (mut probes, mut against) = (Vec::new(), None);
            while let Some(i) = args.iter().position(|a| a == "--probe" || a == "--against") {
                let (Some(v), true) = (args.get(i + 1).cloned(), args[i] == "--probe") else {
                    against = args.get(i + 1).map(PathBuf::from);
                    args.drain(i..(i + 2).min(args.len()));
                    continue;
                };
                let Some(p) = depth_check::Probe::parse(&v) else {
                    eprintln!("error: --probe wants X,Y@FRAME (output pixel, captured frame)");
                    return ExitCode::FAILURE;
                };
                probes.push(p);
                args.drain(i..i + 2);
            }
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let out = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out").join(roms.game.id).join("depth-check"));
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            let (aa, against) = (AA.get().copied().unwrap_or(1), against.as_deref());
            let light = light_options(roms.game);
            if let Err(e) =
                depth_check::run(&root, windows, VIEW.get().copied(), aa, &out, &probes, against, light.as_ref())
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "lighting-check" => {
            let mut args = args.clone();
            let mut probes = Vec::new();
            while let Some(i) = args.iter().position(|a| a == "--probe") {
                let Some(p) = args.get(i + 1).and_then(|v| depth_check::Probe::parse(v)) else {
                    eprintln!("error: --probe wants X,Y@FRAME (output pixel, captured frame)");
                    return ExitCode::FAILURE;
                };
                probes.push(p);
                args.drain(i..i + 2);
            }
            let bench = match args.iter().position(|a| a == "--bench") {
                Some(i) => {
                    let n = args.get(i + 1).and_then(|v| v.parse::<usize>().ok());
                    args.drain(i..i + 1 + n.is_some() as usize);
                    Some(n.unwrap_or(8))
                }
                None => None,
            };
            let every = match args.iter().position(|a| a == "--every") {
                Some(i) => match args.get(i + 1).and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0) {
                    Some(n) => {
                        args.drain(i..i + 2);
                        n
                    }
                    None => {
                        eprintln!("error: --every wants a number of frames");
                        return ExitCode::FAILURE;
                    }
                },
                None => 1,
            };
            let side = match args.iter().position(|a| a == "--no-side") {
                Some(i) => {
                    args.remove(i);
                    false
                }
                None => true,
            };
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let out = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out/scratch/lighting").join(roms.game.id));
            let crease = SHADING.get().map_or(model2::light::CREASE_AUTO, |s| s.1);
            // `--shading pixel` checks the per-pixel level; anything else vertex.
            let level = match SHADING.get() {
                Some((model2::light::Shading::Pixel, _)) => model2::light::Shading::Pixel,
                _ => model2::light::Shading::Vertex,
            };
            let opts = model2::light::Options::new(level, crease, &roms.game.shading, roms.game.hud_windows)
                .with_fine(SHADING_FINE.get().copied().unwrap_or(false))
                .with_balance(SHADING_BALANCE.get().copied().unwrap_or(true));
            let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
            match lighting_check::run(&root, windows, VIEW.get().copied(), opts, &out, &probes, bench, every, side) {
                Ok(true) => {}
                Ok(false) => return ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        // Debug: run recompiled and interpreted i960s side by side, one
        // instruction at a time, and report the first that differs.
        "lockstep" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(50_000_000);
            let mut native = model2::Machine::new(&roms);
            let mut reference = model2::Machine::new(&roms);
            reference.disable_native();
            reference.disable_native_sharc();
            if !native.is_native() {
                eprintln!("error: this build has no recompiled i960 code for {}", roms.game.id);
                return ExitCode::FAILURE;
            }
            // Model 2B: the SHARCs too, the copro after every block, the
            // geometrizer (inline, so its frame has run) every frame.
            native.bus.geo.set_threaded(false);
            reference.bus.geo.set_threaded(false);
            let (mut sharcs_reported, mut checked_frame) = (false, 0);
            while native.cpu.icount < steps {
                let ip = native.cpu.ip;
                let before = native.cpu.r;
                let a = native.run_block();
                let mut b = Ok(());
                while b.is_ok() && reference.cpu.icount < native.cpu.icount {
                    b = reference.step();
                }
                let (n, r) = (&native.cpu, &reference.cpu);
                if a.is_err()
                    || b.is_err()
                    || n.r != r.r
                    || n.ip != r.ip
                    || n.ac != r.ac
                    || n.pc != r.pc
                    || n.locals != r.locals
                {
                    println!("diverged after {} instructions, in the block at {ip:#x}:", r.icount);
                    let mut pc = ip;
                    for _ in 0..a.as_ref().map_or(1, |&n| n) {
                        let i = img.decode(pc);
                        println!("  {pc:08x}  {i}");
                        pc = i.next();
                    }
                    println!("  results: recompiled {a:?}, interpreter {b:?}");
                    println!(
                        "  ip {:#x} / {:#x}  ac {:#x} / {:#x}  pc {:#x} / {:#x}",
                        n.ip, r.ip, n.ac, r.ac, n.pc, r.pc
                    );
                    if n.locals != r.locals {
                        println!("  cached frames: recompiled {:x?}", n.locals.sets());
                        println!("               interpreter {:x?}", r.locals.sets());
                    }
                    for (i, ((&was, &got), &want)) in before.iter().zip(&n.r).zip(&r.r).enumerate() {
                        if got != want || was != want {
                            let name = i960::reg_name(i as u8);
                            println!(
                                "  {name:>4}: before {was:#010x}  recompiled {got:#010x}  interpreter {want:#010x}"
                            );
                        }
                    }
                    return ExitCode::FAILURE;
                }
                if let Err(e) = sharc_lockstep(&native, &reference, &mut sharcs_reported, &mut checked_frame) {
                    println!("diverged after {} i960 instructions (frame {}): {e}", r.icount, native.frames);
                    return ExitCode::FAILURE;
                }
            }
            println!("{} instructions identical", native.cpu.icount);
        }
        // Debug: boot out/GAME/geo.bin, pulse F0, feed hex words, trace.
        "geo-trace-words" => {
            let dir = PathBuf::from("out").join(roms.game.id);
            let bin = std::fs::read(dir.join("geo.bin")).unwrap_or_default();
            let prog: Vec<u64> = bin.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
            let words: Vec<u32> = args[1..].iter().filter_map(|s| u32::from_str_radix(s, 16).ok()).collect();
            geo_trace_words(&prog, &words);
        }
        "geo-check" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(750_000_000);
            let from = args.get(2).and_then(|s| parse_num(s)).map_or(0, u64::from);
            let script = match model2::input::InputScript::parse(input.as_deref().unwrap_or(""), &roms.game.controls) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: --input: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mut m = model2::Machine::new(&roms);
            if interp {
                m.disable_native();
            }
            m.bus.geo.set_threaded(!single_thread);
            m.bus.geo.set_mode(model2::geo::GeoMode::Check);
            m.bus.geo.set_light(LIGHT.get().is_some());
            if let Err(e) = geo_check::run(&mut m, &roms, steps, from, &script, LOAD_STATE.get()) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "geo-fuzz" => {
            let frames = args.get(1).and_then(|s| parse_num(s)).map_or(200, u64::from);
            let seed = args.get(2).and_then(|s| parse_num(s)).map_or(1, u64::from);
            if let Err(e) = geo_check::fuzz(&roms, frames, seed) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "run-ahead-check" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map_or(450_000_000, u64::from);
            let frames = args.get(2).and_then(|s| parse_num(s)).unwrap_or(1);
            let every = args.get(3).and_then(|s| parse_num(s)).map_or(50, u64::from).max(1);
            match run_ahead::check(&roms, steps, frames, every, input.as_deref().unwrap_or(""), interp, single_thread) {
                Ok(true) => {}
                Ok(false) => return ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "latency" => {
            let (Some(at), Some(change)) = (args.get(1).and_then(|s| parse_num(s)), args.get(2)) else {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            };
            let frames = args.get(3).and_then(|s| parse_num(s)).map_or(12, u64::from);
            if let Err(e) =
                latency(&roms, at as u64, change, frames, input.as_deref().unwrap_or(""), interp, single_thread)
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "run" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(50_000_000);
            let snap = args.get(2).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(0);
            let script = match model2::input::InputScript::parse(input.as_deref().unwrap_or(""), &roms.game.controls) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: --input: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let geo_frame = geo_frame.and_then(|s| parse_num(&s)).map(u64::from);
            run(&roms, steps, snap, &script, geo_frame, interp, single_thread);
        }
        "sound-disasm" => {
            let Some(mut pc) = args.get(1).and_then(|s| parse_num(s)) else {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            };
            let count = args.get(2).and_then(|s| parse_num(s)).unwrap_or(32);
            let rom = roms.region16be(romset::Role::AudioCpu);
            let fetch = |a: u32| {
                let o = a.wrapping_sub(0x60_0000) as usize;
                rom.get(o..o + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
            };
            for _ in 0..count {
                let (text, len) = m68k::disasm(pc, fetch);
                let words: Vec<String> = (0..len / 2).map(|i| format!("{:04x}", fetch(pc + 2 * i))).collect();
                println!("{pc:06x}  {:<20}  {text}", words.join(" "));
                pc += len;
            }
        }
        "sound-render" => {
            let dir = PathBuf::from("out").join(roms.game.id);
            let tx_path = dir.join("sound-tx.txt");
            let tx = match std::fs::read_to_string(&tx_path) {
                Ok(t) => sound_render::parse_tx(&t),
                Err(e) => {
                    eprintln!("error: {}: {e} (run `make run` first)", tx_path.display());
                    return ExitCode::FAILURE;
                }
            };
            let wav = args.get(1).map(PathBuf::from).unwrap_or_else(|| dir.join("sound.wav"));
            let seconds = args.get(2).and_then(|s| s.parse::<f64>().ok());
            println!("host bytes    {} from {}", tx.len(), tx_path.display());
            if let Err(e) = sound_render::render(&roms, &tx, seconds, &wav) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        "music-render" | "mpeg-dump" => {
            let dir = PathBuf::from("out").join(roms.game.id);
            let r = if cmd == "music-render" {
                let wav = args.get(1).map(PathBuf::from).unwrap_or_else(|| dir.join("music.wav"));
                let secs = args.get(2).and_then(|s| s.parse().ok());
                music::music_render(&roms, &dir.join("sound-tx.txt"), &wav, secs, soundtrack.as_deref().map(Path::new))
            } else {
                let Some(addr) = args.get(1).and_then(|s| parse_num(s)) else {
                    eprint!("{USAGE}");
                    return ExitCode::FAILURE;
                };
                let secs = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
                music::mpeg_dump(&roms, addr, secs, &dir)
            };
            if let Err(e) = r {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// The supported games whose chips are all in `rom_dir/<set>/` with the
/// right sizes: what the browser build can serve, one file per chip.
fn web_index(rom_dir: &Path) -> Vec<&'static romset::Game> {
    games::ALL
        .iter()
        .copied()
        .filter(|g| {
            if romset::rom_folder(g, rom_dir).is_some() {
                return true;
            }
            let missing: Vec<_> = g
                .files
                .iter()
                .filter(|f| {
                    std::fs::metadata(rom_dir.join(g.id).join(f.name)).map_or(true, |m| m.len() != f.size as u64)
                })
                .map(|f| f.name)
                .collect();
            if missing.len() < g.files.len() {
                eprintln!("{}: left out, {}/{}/ lacks {}", g.id, rom_dir.display(), g.id, missing.join(", "));
            }
            false
        })
        .collect()
}

/// `s` as a JSON string.
fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

/// The browser build's game list (`web-index DIR`): for each game its
/// title, year, board, page, poster (`DIR/posters/<set>.webp`, else its
/// parent set's, else `posters/placeholder.webp`, if there), bezel
/// (`DIR/bezels/<set>.webp`, else its parent set's, else null), screenshot
/// (if `DIR/model2/<set>/shot.png` exists) and controls summary.
fn games_json(games: &[&romset::Game], dir: &Path) -> String {
    let mut out = String::from("{\"games\": [");
    for (i, g) in games.iter().enumerate() {
        let page = format!("model2/{}/", g.id);
        let shot = format!("{page}shot.png");
        let shot = if dir.join(&shot).is_file() { json_str(&shot) } else { "null".into() };
        // A clone shows its parent's poster (the same game).
        let poster = std::iter::once(g.id)
            .chain(g.parent)
            .map(|set| format!("posters/{set}.webp"))
            .chain(["posters/placeholder.webp".into()])
            .find(|p| dir.join(p).is_file())
            .map_or("null".into(), |p| json_str(&p));
        let bezel = std::iter::once(g.id)
            .chain(g.parent)
            .map(|set| format!("bezels/{set}.webp"))
            .find(|p| dir.join(p).is_file())
            .map_or("null".into(), |p| json_str(&p));
        let controls: Vec<String> = g
            .controls
            .summary()
            .iter()
            .map(|(keys, label)| {
                let keys: Vec<String> = keys.iter().map(|k| json_str(k)).collect();
                format!("{{\"keys\": [{}], \"label\": {}}}", keys.join(", "), json_str(label))
            })
            .collect();
        let _ = write!(
            out,
            "{}\n  {{\"id\": {}, \"title\": {}, \"year\": {}, \"board\": {}, \"page\": {}, \"poster\": {}, \"bezel\": {}, \"shot\": {}, \"controls\": [{}]}}",
            if i > 0 { "," } else { "" },
            json_str(g.id),
            json_str(g.title),
            g.year,
            json_str(g.board.name()),
            json_str(&page),
            poster,
            bezel,
            shot,
            controls.join(", ")
        );
    }
    out + "\n]}\n"
}

/// Output view for snapshots (`--height`, `--aspect`, `--hud`); native when unset.
static VIEW: std::sync::OnceLock<model2::video::View> = std::sync::OnceLock::new();
/// `--aa`: samples a pixel for snapshots (supersampled) and gpu-check.
static AA: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
/// `--tex-filter`: filtering of texture pack images.
static TEX_FILTER: std::sync::OnceLock<model2::enhance::TexFilter> = std::sync::OnceLock::new();

fn tex_filter() -> model2::enhance::TexFilter {
    TEX_FILTER.get().copied().unwrap_or_default()
}

/// `--materials`: the texture pack's material maps on the GPU.
static MATERIALS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `--depth`: the depth buffer for snapshots and gpu-check.
static DEPTH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
/// `--shading`, `--crease`: smooth shading for snapshots and the GPU
/// tools (shading, crease in degrees or `light::CREASE_AUTO`).
static SHADING: std::sync::OnceLock<(model2::light::Shading, u8)> = std::sync::OnceLock::new();
/// `--shading-fine`: finer levels for `--shading`.
static SHADING_FINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
/// `--shading-balance`: the brightness balance for `--shading`.
static SHADING_BALANCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
/// `--game`, for snapshots' shading rules.
static GAME_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// `--light` (or a capture): record smooth shading's side channel.
static LIGHT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Smooth shading's options for `game` from `--shading` and `--crease`
/// (None: off).
fn light_options(game: &romset::Game) -> Option<model2::light::Options> {
    let &(shading, crease) = SHADING.get()?;
    let o = model2::light::Options::new(shading, crease, &game.shading, game.hud_windows)
        .with_fine(SHADING_FINE.get().copied().unwrap_or(false))
        .with_balance(SHADING_BALANCE.get().copied().unwrap_or(true));
    o.on().then_some(o)
}

/// The GPU's quality for drawing polygons built with `light`: shading on
/// (whatever `Poly::lit` holds is drawn) or off; and `--materials`.
fn gpu_light(light: Option<&model2::light::Options>) -> gpu::Quality {
    let materials = MATERIALS.get().copied().unwrap_or(false);
    let q = match light {
        Some(o) => gpu::Quality {
            shading: o.shading,
            crease: o.crease,
            shading_fine: o.fine,
            shading_balance: o.balance,
            ..Default::default()
        },
        None => gpu::Quality::default(),
    };
    gpu::Quality { materials, ..q }
}

/// `--temporal`, `--stipple`: temporal anti-aliasing on the GPU.
static TEMPORAL: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
static STIPPLE: std::sync::OnceLock<gpu::Stipple> = std::sync::OnceLock::new();

/// Linear supersampling factor for `samples` a pixel: the smallest k
/// with k * k >= samples (1 for off).
fn supersample_factor(samples: u32) -> usize {
    (1..).find(|k| k * k >= samples.max(1)).unwrap_or(1) as usize
}
/// `--geo`: how the 2B geometrizer runs.
static GEO_MODE: std::sync::OnceLock<model2::geo::GeoMode> = std::sync::OnceLock::new();
/// `--tgp2a hle`: Model 2A's TGP answered by the function model.
static TGP2A_HLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `--smooth`: the in-betweens to draw at each snapshot.
static SMOOTH: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
/// Smooth motion's in-betweens drawn in a run: how many, host time in all
/// and the slowest (seconds).
static SMOOTH_TIME: std::sync::Mutex<(u64, f64, f64)> = std::sync::Mutex::new((0, 0.0, 0.0));

/// A machine for `roms` with the command line's geometrizer mode.
fn machine(roms: &RomSet) -> model2::Machine {
    let mut m = model2::Machine::new(roms);
    m.bus.geo.set_mode(GEO_MODE.get().copied().unwrap_or_default());
    if let Some(t) = m.bus.tgp2a.as_deref_mut() {
        t.use_hle = TGP2A_HLE.get().copied().unwrap_or(false);
    }
    m.bus.geo.set_smooth(SMOOTH.get().is_some() || CAPTURE_SEQ.get().is_some());
    m.bus.geo.set_light(LIGHT.get().is_some());
    // For the frame before each snapshot (`snapshot`).
    m.bus.geo.set_keep_last(SMOOTH.get().is_some());
    m
}

/// The last frame's in-between at `t`, timed (None: the frame's own
/// stream is what shows then).
fn in_between(m: &mut model2::Machine, t: f32) -> Option<Vec<u32>> {
    m.bus.geo.sync();
    let frame = m.bus.geo.engine().smooth_frame()?;
    let start = std::time::Instant::now();
    let stream = frame.render(t);
    if stream.is_some() {
        let secs = start.elapsed().as_secs_f64();
        let mut st = SMOOTH_TIME.lock().unwrap();
        *st = (st.0 + 1, st.1 + secs, st.2.max(secs));
    }
    stream
}
/// `--save-state`: where, and at which frame.
static SAVE_STATE: std::sync::OnceLock<(PathBuf, u64)> = std::sync::OnceLock::new();
/// `--load-state`: the state to start from.
static LOAD_STATE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// Where `--audio` records the live sound.
static AUDIO: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// Where `--capture` saves snapshot frames.
static CAPTURE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// `--capture-seq DIR` with `--seq FROM,COUNT`: where to save a sequence,
/// its first frame and how many.
static CAPTURE_SEQ: std::sync::OnceLock<(PathBuf, u64, u64)> = std::sync::OnceLock::new();

fn snapshot(m: &mut model2::Machine, path: &std::path::Path) {
    m.bus.geo.sync();
    if let Some(root) = CAPTURE.get() {
        let frame = model2::capture::Frame::grab(&mut m.bus);
        if let Err(e) = frame.save(root, &m.bus.texture_rom) {
            eprintln!("capture: {}: {e}", root.display());
        }
    }
    draw_png(m, None, path);
    let Some(ts) = SMOOTH.get() else { return };
    let Some(frame) = m.bus.geo.engine().smooth_frame() else {
        println!("smooth        {}: no in-betweens (the native geometrizer didn't run the frame)", path.display());
        return;
    };
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let p = &frame.plan;
    println!(
        "smooth        {}: {} transforms, {} matched, {} drawn at this frame's{}",
        path.display(),
        p.transforms().len(),
        p.matched,
        p.snapped,
        if p.cut { ", a cut: nothing blended" } else { "" }
    );
    for &t in ts {
        let stream = in_between(m, t);
        draw_png(m, stream.as_deref(), &path.with_file_name(format!("{stem}-t{:03.0}.png", t * 100.0)));
    }
    // The frame before as it was drawn, to compare with t = 0.
    let prev = m.bus.geo.last_frame().0.to_vec();
    draw_png(m, Some(&prev), &path.with_file_name(format!("{stem}-prev.png")));
}

/// `--capture-seq`: add the machine's current frame and its motion data
/// to the sequence (started on first use).
fn capture_seq(
    m: &mut model2::Machine,
    seq: &mut Option<model2::capture::SeqWriter>,
    root: &Path,
    game: &str,
) -> std::io::Result<()> {
    let frame = model2::capture::Frame::grab(&mut m.bus);
    if seq.is_none() {
        *seq = Some(model2::capture::SeqWriter::new(root, game, &m.bus.texture_rom)?);
    }
    let smooth = m.bus.geo.engine().smooth_frame();
    seq.as_mut().unwrap().push(&frame, smooth.as_deref())
}

/// Draw the machine's current frame to a PNG at `path`, with `stream` for
/// its polygons if given, else the geometrizer's last frame's.
fn draw_png(m: &model2::Machine, stream: Option<&[u32]>, path: &std::path::Path) {
    let (view, rgb) = render(m, stream, path);
    let png = png::encode_rgb(view.width, view.height, &rgb);
    let _ = std::fs::write(path, png);
}

/// The machine's current frame as `draw_png` draws it (`path` names it
/// for `REMODEL_PROBE`'s report): the view, and its RGB pixels.
fn render(m: &model2::Machine, stream: Option<&[u32]>, path: &std::path::Path) -> (model2::video::View, Vec<u8>) {
    let out_view = VIEW.get().copied().unwrap_or_else(model2::video::View::native);
    // Anti-aliasing: draw and compose larger, average down at the end.
    let k = supersample_factor(AA.get().copied().unwrap_or(1));
    let view = out_view.supersampled(k);
    let depth = DEPTH.get().copied().unwrap_or(false);
    let b = &m.bus;
    let geo = b.geo.engine();
    let stream = stream.unwrap_or(&geo.stream);
    let (xoff, yoff) = b.sync_offsets();
    let src = model2::raster::Sources {
        texture_rom: &b.texture_rom,
        texture_point_ram: &geo.texture_point_ram,
        render_ram: b.render_ram(),
        palette: b.palette_ram(),
        xoff,
        yoff,
    };
    let light = games::find(GAME_ID.get().map_or("topskatr", |s| s.as_str())).ok().and_then(light_options);
    // The side channel goes with the engine's own stream.
    let side = if std::ptr::eq(stream, &geo.stream[..]) { geo.light_frame() } else { None };
    let mut polys = model2::raster::build_shaded_with(stream, &src, side.as_deref(), light.as_ref());
    model2::video::HudLayout::new(b.tile_ram(), b.palette_ram(), &view).anchor(&mut polys, &view);
    let mut packs = texpack::PACKS.lock().unwrap();
    let (rgb3d, covered) = match packs.as_mut() {
        Some(p) => {
            p.frame = m.frames;
            model2::raster::draw_enhanced_filtered(&mut polys, &src, &view, p, depth, tex_filter())
        }
        None => model2::raster::draw_view_depth(&mut polys, &src, &view, depth),
    };
    if let Some((x, y)) = std::env::var("REMODEL_PROBE").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
    }) {
        println!("probe {x},{y} in {}:", path.display());
        for line in model2::raster::probe(&polys, &src, x, y) {
            println!("  {line}");
        }
    }
    let layer3d = Some((&rgb3d[..], &covered[..]));
    let rgb = match packs.as_mut() {
        Some(p) => {
            let rgb = model2::video::compose_enhanced(b.tile_ram(), b.palette_ram(), layer3d, &view, p);
            p.finish_snapshot();
            rgb
        }
        None => model2::video::compose(b.tile_ram(), b.palette_ram(), layer3d, &view),
    };
    (out_view, model2::video::downsample(&rgb, view.width, view.height, k))
}

/// `latency`: run to frame `at`, then on as two machines, one with
/// `change` held from there on, and report per frame where they differ.
/// Frame i is the i-th after the change took effect: the game reads its
/// inputs, then at the vblank that ends it the display list goes to the
/// geometrizer, whose polygons a player sees drawn next (native player;
/// the browser's geometry worker adds about a frame).
fn latency(
    roms: &RomSet,
    at: u64,
    change: &str,
    frames: u64,
    spec: &str,
    interp: bool,
    single_thread: bool,
) -> Result<(), String> {
    use model2::input::InputScript;
    let controls = &roms.game.controls;
    let base = InputScript::parse(spec, controls).map_err(|e| format!("--input: {e}"))?;
    let changed = InputScript::parse(&format!("{spec},{change}@{at}-{}", u32::MAX), controls)?;
    let channel = change.split_once('=').and_then(|(n, _)| controls.analog(n)).map(|a| a.channel as usize);
    let new = |roms| {
        let mut m = machine(roms);
        if interp {
            m.disable_native();
        }
        m.bus.geo.set_threaded(!single_thread);
        m
    };
    let mut a = new(roms);
    if let Ok(bytes) = std::fs::read(PathBuf::from("out").join(roms.game.id).join("eeprom.bin")) {
        a.bus.eeprom.load(&bytes);
    }
    if let Some(path) = LOAD_STATE.get() {
        let d = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        model2::load_session(&mut a, None, &d)?;
    }
    base.apply(a.frames, &mut a.bus);
    if a.frames > at {
        return Err(format!("the run starts at frame {}, after {at}", a.frames));
    }
    while a.frames < at {
        let f = a.frames;
        a.run_slice(u64::MAX).map_err(|s| format!("stopped: {s:?}"))?;
        if a.frames != f {
            base.apply(a.frames, &mut a.bus);
        }
    }
    let mut b = new(roms);
    b.load_state(&a.save_state())?;
    base.apply(at, &mut a.bus);
    changed.apply(at, &mut b.bus);
    println!("{} from frame {at}: {change}", roms.game.id);
    println!(
        "frame  read{}  RAM  list  polygons (words differing)  picture (pixels differing)",
        if channel.is_some() { " (at, value)" } else { "" }
    );
    let (mut first_ram, mut first_list, mut first_poly, mut first_picture) = (None, None, None, None);
    for i in 0..frames {
        let start = b.bus.now;
        a.run_slice(u64::MAX).map_err(|s| format!("stopped: {s:?}"))?;
        b.run_slice(u64::MAX).map_err(|s| format!("stopped: {s:?}"))?;
        a.bus.geo.sync();
        b.bus.geo.sync();
        let read = channel.map_or(String::new(), |c| {
            let (when, v) = b.bus.analog_reads[c];
            if when >= start {
                let into = (when - start) as f64 / (b.bus.now - start).max(1) as f64;
                format!("  {:3.0}% {v:#04x}", into * 100.0)
            } else {
                "  -         ".into()
            }
        });
        let ((ra, la), (rb, lb)) = (a.bus.ram_hashes(), b.bus.ram_hashes());
        let (sa, sb) = (&a.bus.geo.engine().stream, &b.bus.geo.engine().stream);
        let poly = sa.iter().zip(sb).filter(|(x, y)| x != y).count() + sa.len().abs_diff(sb.len());
        let mark = |d: bool, first: &mut Option<u64>| {
            if d && first.is_none() {
                *first = Some(i);
            }
            if d {
                "yes"
            } else {
                "-"
            }
        };
        // The whole picture as drawn (tiles and polygons), pixel by pixel.
        let (pa, pb) = (render(&a, None, Path::new("a")).1, render(&b, None, Path::new("b")).1);
        let pixels = pa.chunks(3).zip(pb.chunks(3)).filter(|(x, y)| x != y).count();
        let polygons = format!("{} ({poly})", mark(poly > 0, &mut first_poly));
        println!(
            "{i:5}{read}  {:>3}  {:>4}  {polygons:>17}  {:>7} ({pixels})",
            mark(ra != rb, &mut first_ram),
            mark(la != lb, &mut first_list),
            mark(pixels > 0, &mut first_picture)
        );
        let f = a.frames;
        base.apply(f, &mut a.bus);
        changed.apply(f, &mut b.bus);
    }
    let show = |f: Option<u64>| f.map_or("not within the run".into(), |f| format!("frame {f}"));
    println!(
        "first difference: RAM {}, display list {}, polygons {}, picture {}",
        show(first_ram),
        show(first_list),
        show(first_poly),
        show(first_picture)
    );
    if let Some(f) = first_poly {
        println!("drawn after {} frames from the input (native player; browser: about one more)", f + 1);
    }
    match first_picture {
        Some(f) => println!(
            "input lag     {f} frame{}: the picture first changes {}; run-ahead of up to {f} hides that many",
            if f == 1 { "" } else { "s" },
            if f == 0 {
                "in the first frame that can read it"
            } else {
                "that many frames after the first that can read it"
            }
        ),
        None => println!("input lag     not seen within {frames} frames"),
    }
    Ok(())
}

fn run(
    roms: &RomSet,
    steps: u64,
    snap: u64,
    script: &model2::input::InputScript,
    geo_frame: Option<u64>,
    interp: bool,
    single_thread: bool,
) {
    let mut m = machine(roms);
    if interp {
        m.disable_native();
    }
    m.bus.geo.set_threaded(!single_thread);
    println!("i960          {}", if m.is_native() { "recompiled" } else { "interpreter" });
    let dir = PathBuf::from("out").join(roms.game.id);
    let eeprom_path = dir.join("eeprom.bin");
    if let Ok(bytes) = std::fs::read(&eeprom_path) {
        m.bus.eeprom.load(&bytes);
        println!("eeprom        loaded {}", eeprom_path.display());
    }
    let frames_dir = dir.join("frames");
    let _ = std::fs::remove_dir_all(&frames_dir);
    let _ = std::fs::create_dir_all(&frames_dir);
    if let Some(a) = std::env::var("REMODEL_WATCH").ok().and_then(|v| parse_num(&v)) {
        m.bus.watch = Some((a, Vec::new()));
    }
    let mut live = AUDIO.get().map(|_| model2::audio::Live::new(roms));
    if let Some(path) = LOAD_STATE.get() {
        let loaded = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|d| model2::load_session(&mut m, live.as_mut(), &d));
        match loaded {
            // A state is saved as its frame starts, before that frame's
            // scripted inputs are applied: apply them now, as the
            // uninterrupted run did.
            Ok(()) => {
                script.apply(m.frames, &mut m.bus);
                println!("state         loaded {} (frame {})", path.display(), m.frames)
            }
            Err(e) => {
                eprintln!("error: {}: {e}", path.display());
                return;
            }
        }
    }
    let (mut pcm, mut link, mut audio_secs) = (Vec::new(), Vec::new(), 0.0);
    let started = std::time::Instant::now();
    let mut stop = None;
    let mut last_frame = 0;
    let mut seq = None;
    let mut light = light_report::Acc::default();
    while m.cpu.icount < steps {
        if let Err(s) = m.run_slice(steps - m.cpu.icount) {
            stop = Some(s);
            break;
        }
        if let Some(live) = &mut live {
            // `Live` consumes the link log; keep it for sound-tx.txt.
            link.extend_from_slice(&m.bus.uart_log);
            let t = std::time::Instant::now();
            live.run(&mut m.bus, &mut pcm);
            audio_secs += t.elapsed().as_secs_f64();
        }
        if m.frames != last_frame {
            if LIGHT.get().is_some() {
                light.frame(&mut m);
            }
            if let Some((path, frame)) = SAVE_STATE.get().filter(|s| s.1 == m.frames) {
                let data = model2::save_session(&mut m, live.as_ref());
                match std::fs::write(path, &data) {
                    Ok(()) => {
                        println!("state         saved {} at frame {frame} ({} KB)", path.display(), data.len() / 1024)
                    }
                    Err(e) => eprintln!("error: {}: {e}", path.display()),
                }
            }
            script.apply(m.frames, &mut m.bus);
            if geo_frame == Some(m.frames) {
                m.bus.geo_trace = Some(Vec::new());
            } else if let Some(t) = m.bus.geo_trace.take() {
                let text: String = t.iter().map(|(a, v)| format!("{a:08x} {v:08x}\n")).collect();
                let _ = std::fs::write(dir.join(format!("geo-{}.txt", m.frames - 1)), text);
                let _ = std::fs::write(dir.join(format!("buffer-{}.bin", m.frames - 1)), m.bus.buffer_ram());
                println!("geo trace     frame {}: {} writes", m.frames - 1, t.len());
            }
        }
        if let Some((root, from, count)) = CAPTURE_SEQ.get().filter(|s| m.frames != last_frame && m.frames >= s.1) {
            if m.frames >= from + count {
                println!("sequence      {count} frames from {from} -> {}", root.display());
                break;
            }
            if let Err(e) = capture_seq(&mut m, &mut seq, root, roms.game.id) {
                eprintln!("error: --capture-seq: {}: {e}", root.display());
                return;
            }
        }
        if snap > 0 && m.frames != last_frame && m.frames.is_multiple_of(snap) {
            let path = frames_dir.join(format!("{:06}.png", m.frames));
            snapshot(&mut m, &path);
        } else if SMOOTH.get().is_some() && m.frames != last_frame {
            // Time an in-between of every frame (drawn nowhere): what a
            // front end adds per display refresh.
            in_between(&mut m, 0.5);
        }
        last_frame = m.frames;
    }
    snapshot(&mut m, &frames_dir.join("final.png"));
    if LIGHT.get().is_some() {
        print!("{}", light.report());
        m.bus.geo.sync();
        print!("{}", light_report::scan_report(&m.bus.geo.engine().check.light));
    }
    if let Some(p) = texpack::PACKS.lock().unwrap().as_ref() {
        println!("textures      {}", p.summary());
    }
    let secs = started.elapsed().as_secs_f64();
    if let (Some(path), Some(_)) = (AUDIO.get(), &live) {
        m.bus.uart_log = link;
        let len = pcm.len() as f64 / 2.0 / model2::audio::RATE as f64;
        match music::write_wav(path, model2::audio::RATE, 2, &pcm) {
            Ok(()) => println!(
                "audio         {len:.1}s live mix -> {} ({audio_secs:.2}s host: {:.1}x real time)",
                path.display(),
                len / audio_secs.max(1e-9)
            ),
            Err(e) => eprintln!("audio: {}: {e}", path.display()),
        }
    }
    let state_hash = m.bus.state_hash();
    if let Some(t) = m.bus.tgp2a.as_deref_mut() {
        t.finish();
    }
    let b = &m.bus;
    let geo = b.geo.engine();

    match stop {
        Some(s) => println!("stopped       {s:?}"),
        None => println!("stopped       step limit"),
    }
    println!("instructions  {} ({:.1} M/s)", m.cpu.icount, m.cpu.icount as f64 / secs / 1e6);
    let emulated = m.frames as f64 / 57.524;
    println!("emulated      {} frames ({emulated:.2}s)", m.frames);
    println!(
        "host time     {secs:.2}s ({:.2}x real time), geometrizer {:.2}s {} (slowest frame {:.1} ms), {:.2}s waited for",
        emulated / secs,
        geo.busy.as_secs_f64(),
        if single_thread { "inline" } else { "on its thread" },
        geo.slowest.as_secs_f64() * 1e3,
        b.geo.waited.as_secs_f64()
    );
    println!("ip            {:#x}  priority {}  icr {:#010x}", m.cpu.ip, m.cpu.priority(), m.cpu.icr);
    println!("irq           request {:#x} enable {:#x}", b.irq_request, b.irq_enable);
    println!("coverage      {} distinct instructions", m.coverage.len());
    println!("indirect      {} distinct (source, target) pairs", m.indirect.len());
    if SMOOTH.get().is_some() {
        let (n, total, worst) = *SMOOTH_TIME.lock().unwrap();
        println!(
            "smooth        {n} in-betweens, {:.2} ms each (slowest {:.2} ms); recording and matching {:.3} ms a frame",
            total * 1e3 / n.max(1) as f64,
            worst * 1e3,
            geo.smooth_busy().as_secs_f64() * 1e3 / m.frames.max(1) as f64
        );
    }
    println!("copro results hash {:016x}", b.copro_result_hash);
    println!("state hash    {:016x} (ip {:#x})", state_hash, m.cpu.ip);
    println!("geo           {} data words", b.geo_words);
    match &b.sharc {
        Some(c) => report_2b(&dir, c, &b.geo, geo),
        None if b.tgp2a.is_some() => tgp2a::report(&dir, b),
        None => report_2c(&dir, b, geo),
    }
    let fed: String = geo.fed.iter().map(|w| format!("{w:08x}\n")).collect();
    let _ = std::fs::write(dir.join("geo-last-list.txt"), fed);
    let raster: String = geo.stream.iter().map(|w| format!("{w:08x}\n")).collect();
    let _ = std::fs::write(dir.join("raster-last.txt"), raster);
    if !b.copro_empty_by_ip.is_empty() {
        let first = b.copro_empty_by_ip.keys().next();
        println!(
            "copro waits   {} sites read results that don't exist yet (lowest {first:x?}):",
            b.copro_empty_by_ip.len()
        );
        let mut v: Vec<_> = b.copro_empty_by_ip.iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        for (ip, n) in v.into_iter().take(6) {
            println!("  {ip:08x} {n:>9}");
        }
    }
    if !m.cpu.unhandled_iac.is_empty() {
        println!("unhandled IAC {:x?}", m.cpu.unhandled_iac);
    }
    println!("hot spots (current frame):");
    for (a, n) in m.hot_spots(8) {
        let insn = i960::decode(a, |x| i960::cpu::Bus::fetch(&m.bus, x));
        println!("  {a:08x} {n:>9}  {insn}");
    }
    let top = |log: &mut dyn Iterator<Item = (&u32, &model2::bus::Access)>, n: usize| {
        let mut v: Vec<_> = log.collect();
        v.sort_by_key(|&(&k, a)| (std::cmp::Reverse(a.reads + a.writes), k));
        v.into_iter().take(n).map(|(&k, &a)| (k, a)).collect::<Vec<_>>()
    };
    let tx = b.uart_log.iter().filter(|e| !e.1).count();
    println!(
        "sound link    {tx} data bytes, {} control writes (out/{}/sound-tx.txt)",
        b.uart_log.len() - tx,
        roms.game.id
    );
    let text: String = b
        .uart_log
        .iter()
        .map(|&(t, c, v)| format!("{:>6} {} {v:02x}\n", t / model2::bus::FRAME_CYCLES, if c { "ctl " } else { "data" }))
        .collect();
    let _ = std::fs::write(dir.join("sound-tx.txt"), text);
    if let Some((a, log)) = &b.watch {
        println!("watch {a:#010x}: {} writes", log.len());
        for &(t, ip, addr, v, n) in log.iter().take(64) {
            println!("  frame {:>6} ip {ip:#08x} {addr:#010x} <- {v:#x} ({n} bytes)", t / model2::bus::FRAME_CYCLES);
        }
    }
    // Every register and unmapped address, not just the busiest.
    let _ = std::fs::create_dir_all(&dir);
    let all: String = b
        .io_log
        .iter()
        .map(|(a, e)| (a, e, ""))
        .chain(b.unmapped.iter().map(|(a, e)| (a, e, " unmapped")))
        .map(|(a, e, u)| format!("{a:08x}  r {:>9}  w {:>9}  last {:#x}{u}\n", e.reads, e.writes, e.last_write))
        .collect();
    let _ = std::fs::write(dir.join("io.txt"), all);
    println!("hardware registers ({} touched, all in {}/io.txt):", b.io_log.len(), dir.display());
    for (a, e) in top(&mut b.io_log.iter(), 16) {
        println!("  {a:08x}  r {:>9}  w {:>9}  last {:#x}", e.reads, e.writes, e.last_write);
    }
    if !b.unmapped.is_empty() {
        println!("unmapped ({} addresses):", b.unmapped.len());
        for (a, e) in top(&mut b.unmapped.iter(), 12) {
            println!("  {a:08x}  r {:>9}  w {:>9}  last {:#x}", e.reads, e.writes, e.last_write);
        }
    }

    let _ = std::fs::create_dir_all(&dir);
    let blank = b.eeprom.data.iter().filter(|&&w| w == 0xffff).count();
    println!("eeprom        {} of 64 words programmed, changed this run: {}", 64 - blank, b.eeprom.dirty);
    if let Some(d) = &b.drive {
        println!("drive board   {}", d.describe());
    }
    if let Some(g) = &b.gun {
        println!("gun interface {} configuration bytes, counters {:02x?}", g.config_bytes, &g.counters[..9]);
    }
    if b.eeprom.dirty {
        let _ = std::fs::write(&eeprom_path, b.eeprom.save());
    }
    // Raw video memory for offline inspection (game-derived: stays in out/).
    let vram = dir.join("vram");
    let _ = std::fs::create_dir_all(&vram);
    let _ = std::fs::write(vram.join("tiles.bin"), b.tile_ram());
    let _ = std::fs::write(vram.join("palette.bin"), b.palette_ram());
    let _ = std::fs::write(vram.join("render.bin"), b.render_ram());
    let cov: String = m.coverage.iter().map(|a| format!("{a:08x}\n")).collect();
    let ind: String = m.indirect.iter().map(|(s, t)| format!("{s:08x} {t:08x}\n")).collect();
    // Recompiled runs only see the interpreter's share of the code: keep the
    // full trace the recompiler was built from.
    if !m.is_native() {
        let _ = std::fs::write(dir.join("coverage.txt"), cov);
        let _ = std::fs::write(dir.join("indirect.txt"), ind);
        println!("trace written to {}/{{coverage,indirect}}.txt", dir.display());
    }
    println!("frames written to {}", frames_dir.display());
    if !b.copro_program.is_empty() {
        // Little-endian 64-bit words, in upload order.
        let bin: Vec<u8> = b.copro_program.iter().flat_map(|w| w.to_le_bytes()).collect();
        let _ = std::fs::write(dir.join("copro.bin"), &bin);
        println!("copro program written to {}/copro.bin ({} words)", dir.display(), b.copro_program.len());
    }
}

fn copro_disasm(bin: &[u8], out: &std::path::Path) {
    use mb86235::Flow;
    let words: Vec<u64> = bin.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
    let insns: Vec<mb86235::Insn> = words.iter().enumerate().map(|(a, &w)| mb86235::decode(a as u32, w)).collect();
    let mut subs = std::collections::BTreeSet::from([0u32]);
    let mut locs = std::collections::BTreeSet::new();
    let (mut unresolved, mut invalid) = (0, 0);
    for i in &insns {
        match i.flow() {
            Flow::Call(Some(t)) => {
                subs.insert(t);
            }
            Flow::Jump(Some(t)) | Flow::Branch(Some(t)) => {
                locs.insert(t);
            }
            Flow::Call(None) | Flow::Jump(None) | Flow::Branch(None) => unresolved += 1,
            Flow::Invalid => invalid += 1,
            _ => {}
        }
    }
    let mut text = String::new();
    for i in &insns {
        if subs.contains(&i.addr) {
            let _ = writeln!(text, "\nsub_{:03x}:", i.addr);
        } else if locs.contains(&i.addr) {
            let _ = writeln!(text, "loc_{:03x}:", i.addr);
        }
        let _ = writeln!(text, "  {:03x}  {:016x}  {i}", i.addr, i.word);
    }
    let _ = std::fs::write(out, text);
    let unsupported = insns.iter().filter(|i| matches!(i.transfer, mb86235::Transfer::Unsupported(_))).count();
    println!("words         {}", insns.len());
    println!("subroutines   {}", subs.len());
    println!("branch labels {}", locs.len());
    println!("computed      {unresolved} branches/calls with register or memory targets");
    println!("invalid       {invalid}   unsupported transfers {unsupported}");
    println!("listing written to {}", out.display());
}

fn geo_trace_words(prog: &[u64], words: &[u32]) {
    use mb86235::cpu::{External, Tgp};
    struct Nul;
    impl External for Nul {
        fn read(&mut self, _: u32) -> u32 {
            0
        }
        fn write(&mut self, _: u32, _: u32) {}
    }
    let mut t = Tgp::new();
    t.boot(prog);
    t.run(&mut Nul, 100_000);
    t.ext_flags = 1;
    t.run(&mut Nul, 256);
    t.ext_flags = 0;
    t.fifo_in.extend(words.iter().copied());
    for _ in 0..60 {
        let pc = t.pc;
        let i = mb86235::decode(pc, t.program[pc as usize]);
        t.step(&mut Nul);
        println!(
            "{pc:03x} {:<60} AA0={:08x} AA1={:08x} AA3={:08x} AR0={:x} AR1={:x} fifo={}",
            i.to_string(),
            t.aa[0],
            t.aa[1],
            t.aa[3],
            t.ar[0],
            t.ar[1],
            t.fifo_in.len()
        );
    }
}

/// The MB86235 copro and geometrizer in a `run` report (Model 2C).
fn report_2c(dir: &std::path::Path, b: &model2::Model2Bus, geo: &model2::geo::Engine) {
    println!(
        "copro         program {} words, booted {}, fifo in {} out {}, empty reads {}",
        b.copro_program.len(),
        b.copro_booted,
        b.tgp.fifo_in.len(),
        b.tgp.fifo_out.len(),
        b.copro_empty_reads
    );
    println!("tgp           {} words executed, pc {:#05x}, fault {:x?}", b.tgp.steps, b.tgp.pc, b.tgp_fault);
    let tgp_cov: Vec<u32> = (0..4096).filter(|&a| b.tgp.counts[a as usize] > 0).collect();
    println!("tgp coverage  {} of 4096 words executed", tgp_cov.len());
    let tgp_txt: String = tgp_cov.iter().map(|a| format!("{a:03x}\n")).collect();
    let _ = std::fs::write(dir.join("copro-coverage.txt"), tgp_txt);
    println!(
        "geometrizer   program {} words, booted {}, frames {}, last list {} words -> {} raster words, bad cmds {}, fault {:x?}, steps {}, pc {:#05x}",
        b.geo.program.len(),
        b.geo.booted,
        geo.frames,
        geo.list_words,
        geo.stream.len(),
        b.geo.bad_commands,
        geo.fault,
        geo.tgp.steps,
        geo.tgp.pc
    );
    match (geo.mode, &geo.native_tgp) {
        (model2::geo::GeoMode::Chip, _) => {}
        (mode, Some(n)) => println!("geometrizer   {mode:?}: native {:?}, stopped {:?}", n.program, n.stop),
        (mode, None) => println!("geometrizer   {mode:?}: no native implementation of the program, on the TGP"),
    }
    let mut geo_hot: Vec<(usize, u64)> = geo.tgp.counts.iter().copied().enumerate().filter(|c| c.1 > 0).collect();
    geo_hot.sort_by_key(|&(a, c)| (std::cmp::Reverse(c), a));
    println!("geometrizer   hot spots (whole run):");
    for &(a, c) in geo_hot.iter().take(8) {
        println!("  {a:03x} {c:>12} ({:.1}%)", c as f64 * 100.0 / geo.tgp.steps.max(1) as f64);
    }
    if let Some((frame, fed)) = &geo.first_error {
        let text: String = fed.iter().map(|w| format!("{w:08x}\n")).collect();
        let _ = std::fs::write(dir.join("geo-error-list.txt"), text);
        let tr: String = geo.first_error_trace.iter().map(|(pc, f)| format!("{pc:03x} {f}\n")).collect();
        let _ = std::fs::write(dir.join("geo-error-pcs.txt"), tr);
        println!("geometrizer   first command-check error at geo frame {frame} ({} words fed)", fed.len());
    }
    if !b.geo.program.is_empty() {
        let bin: Vec<u8> = b.geo.program.iter().flat_map(|w| w.to_le_bytes()).collect();
        let _ = std::fs::write(dir.join("geo.bin"), &bin);
        let cov: String = (0..4096).filter(|&a| geo.tgp.counts[a] > 0).map(|a| format!("{a:03x}\n")).collect();
        let _ = std::fs::write(dir.join("geo-coverage.txt"), cov);
    }
}

/// `lockstep` on Model 2B: the copro SHARCs' registers, pipeline and
/// counters must match after every i960 block, and after each frame both
/// SHARCs' whole state (memory included) and the geometrizer's output.
/// `reported`: whether the SHARCs' recompiled code has been announced;
/// `checked`: the last frame compared.
fn sharc_lockstep(
    native: &model2::Machine,
    reference: &model2::Machine,
    reported: &mut bool,
    checked: &mut u64,
) -> Result<(), String> {
    let (Some(n), Some(r)) = (native.bus.sharc.as_deref(), reference.bus.sharc.as_deref()) else { return Ok(()) };
    let (nc, rc) = (&n.cpu, &r.cpu);
    if nc.steps != rc.steps || nc.pc != rc.pc || nc.r != rc.r || n.fault != r.fault {
        return Err(format!(
            "copro SHARC: recompiled {} steps at pc {:#x}, interpreter {} steps at pc {:#x}; faults {:x?} / {:x?}",
            nc.steps, nc.pc, rc.steps, rc.pc, n.fault, r.fault
        ));
    }
    if !*reported && n.booted && native.bus.geo.booted {
        *reported = true;
        let geo = native.bus.geo.engine().sharc_is_native();
        println!("SHARCs: copro {}, geometrizer {}", native_name(n.is_native()), native_name(geo));
    }
    if native.frames == *checked {
        return Ok(());
    }
    *checked = native.frames;
    if nc.digest() != rc.digest() || state::to_bytes(nc) != state::to_bytes(rc) {
        return Err(format!("copro SHARC state differs at the frame's end (pc {:#x}, {} steps)", nc.pc, nc.steps));
    }
    let (ng, rg) = (native.bus.geo.engine(), reference.bus.geo.engine());
    if let (Some(a), Some(b)) = (ng.sharc.as_deref(), rg.sharc.as_deref()) {
        if a.digest() != b.digest() || state::to_bytes(a) != state::to_bytes(b) || ng.stream != rg.stream {
            return Err(format!(
                "geometrizer SHARC state or output differs after frame {}: recompiled {} steps at pc {:#x}, interpreter {} at {:#x}",
                ng.frames, a.steps, a.pc, b.steps, b.pc
            ));
        }
    }
    Ok(())
}

fn native_name(native: bool) -> &'static str {
    if native {
        "recompiled"
    } else {
        "interpreted (no recompiled code in this build)"
    }
}

/// A captured SHARC program (`run` writes `sharc-ROLE.bin` and its
/// coverage) and the words `recomp` compiles of it: what static discovery
/// finds plus everything `run` executed.
struct SharcProgram {
    image: Vec<u64>,
    words: BTreeSet<u32>,
    found: BTreeSet<u32>,
    ran: BTreeSet<u32>,
}

impl SharcProgram {
    /// Its crate in out/recomp, named after the compiled words
    /// (`adsp2106x::cpu::code_hash`): uploads with the same code and
    /// different data share it.
    fn krate(&self) -> String {
        let words: Vec<u32> = self.words.iter().copied().collect();
        format!("sharc_{:016x}", adsp2106x::cpu::code_hash(&self.image, &words))
    }
}

fn sharc_program(dir: &Path, role: &str) -> Result<SharcProgram, String> {
    let image = sharc::load(&dir.join(format!("sharc-{role}.bin")))?;
    let found = sharc::discover(&image).code;
    let cov = std::fs::read_to_string(dir.join(format!("sharc-{role}-coverage.txt"))).unwrap_or_default();
    let ran: BTreeSet<u32> = cov.lines().filter_map(|l| u32::from_str_radix(l.trim(), 16).ok()).collect();
    let words = found
        .union(&ran)
        .copied()
        .filter(|&a| a.checked_sub(0x2_0000).is_some_and(|k| (k as usize) < image.len()))
        .collect();
    Ok(SharcProgram { image, words, found, ran })
}

/// The SHARC copro and geometrizer in a `run` report (Model 2B), and
/// their uploaded programs as images for `sharc-disasm`.
fn report_2b(
    dir: &std::path::Path,
    c: &model2::sharc::Copro,
    front: &model2::geo::Geometrizer,
    geo: &model2::geo::Engine,
) {
    println!(
        "copro         SHARC, booted {}, fifo in {} out {}, {} instructions, pc {:#x}, fault {:x?}, host errors {}, empty reads {}",
        c.booted,
        c.fifo_in.len(),
        c.fifo_out.len(),
        c.cpu.steps,
        c.cpu.pc,
        c.fault,
        c.host_errors,
        c.empty_reads
    );
    for (a, n) in &c.other {
        println!("copro         unknown external access {a:#x}: {n}");
    }
    let hot = |name: &str, cpu: &adsp2106x::cpu::Sharc| {
        let total = cpu.steps.max(1) as f64;
        let spots: Vec<String> = model2::sharc::hot_spots(cpu, 8)
            .iter()
            .map(|&(a, n)| format!("{a:05x} {:.1}%", n as f64 * 100.0 / total))
            .collect();
        println!("{name:<13} hot spots: {}", spots.join(", "));
        let misses: u64 = cpu.misses.values().sum();
        if !cpu.misses.is_empty() {
            let mut top: Vec<_> = cpu.misses.iter().collect();
            top.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
            let top: Vec<String> = top.iter().take(8).map(|(a, n)| format!("{a:05x} x{n}")).collect();
            println!(
                "{name:<13} recompiled code missed {misses} instructions at {} addresses: {}",
                cpu.misses.len(),
                top.join(", ")
            );
        }
        let overwritten = cpu.overwritten_code();
        if !overwritten.is_empty() {
            println!("{name:<13} {} compiled words overwritten (interpreted): {overwritten:05x?}", overwritten.len());
        }
        // Every word executed, for `remodel recomp`.
        let cov: String = (0x2_0000..0x4_0000u32)
            .filter(|&a| cpu.count_at(a).is_some_and(|n| n > 0))
            .map(|a| format!("{a:05x}\n"))
            .collect();
        let file = if name == "copro" { "sharc-copro-coverage.txt" } else { "sharc-geo-coverage.txt" };
        let _ = std::fs::write(dir.join(file), cov);
    };
    println!("copro         {}", if c.is_native() { "recompiled" } else { "interpreter" });
    hot("copro", &c.cpu);
    match (geo.mode, &geo.native) {
        (model2::geo::GeoMode::Chip, _) => {}
        (mode, Some(n)) => println!("geometrizer   {mode:?}: native {:?}, stopped {:?}", n.program, n.stop),
        (mode, None) => println!("geometrizer   {mode:?}: no native implementation of the program, on the SHARC"),
    }
    match &geo.sharc {
        Some(s) => {
            println!(
                "geometrizer   SHARC, booted {}, frames {}, last list {} words -> {} raster words, bad cmds {}, fault {:x?}, {} instructions, pc {:#x}",
                front.booted,
                geo.frames,
                geo.list_words,
                geo.stream.len(),
                front.bad_commands,
                geo.fault,
                s.steps,
                s.pc
            );
            for (a, n) in &geo.sharc_other {
                println!("geometrizer   unknown external access {a:#x}: {n}");
            }
            println!("geometrizer   {}", if geo.sharc_is_native() { "recompiled" } else { "interpreter" });
            hot("geometrizer", s);
        }
        None => println!("geometrizer   SHARC (on another worker)"),
    }
    for (name, program) in [("sharc-copro.bin", &c.program), ("sharc-geo.bin", &front.program)] {
        if !program.is_empty() {
            let bin: Vec<u8> =
                model2::sharc::image(program).iter().flat_map(|w| w.to_le_bytes().into_iter().take(6)).collect();
            let _ = std::fs::write(dir.join(name), &bin);
            println!("program       {} ({} words)", dir.join(name).display(), bin.len() / 6);
        }
    }
}
