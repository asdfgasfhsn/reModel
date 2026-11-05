mod gpu_check;
mod music;
mod sound_render;
mod soundtrack;
mod texpack;

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
  --input SCRIPT       scripted inputs for `run`, e.g.
                       coin1@1100-1105,start@1300-1305,curving=0x40@1400-1500
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
  --texture-pack DIR   draw snapshots with the replacement PNGs in DIR (same
                       names as a dump; any size); see docs/texture-packs.md

commands:
  games                list supported games
  scan                 list the sets in the ROM directory (zips or folders,
                       as MAME keeps them) and check the supported ones
  verify               check every chip against known-good hashes
  boot                 print the boot record and system table seeds
  disasm ADDR [COUNT]  linear disassembly of the program ROM
  analyze [OUT]        recursive-descent listing (default out/GAME/program.lst)
  copro-disasm [FILE]  disassemble an MB86235 program image to FILE.lst
                       (default out/GAME/copro.bin; `run` also writes geo.bin)
  recomp               recompile the i960 program (code found by `analyze`
                       plus everything `run` executed) and the copro and
                       geometrizer programs `run` captured into out/recomp/;
                       the next build uses them in place of the interpreters
  gpu-check [DIR]      draw the frames captured with --capture in DIR
                       (default out/GAME/capture) with the software and GPU
                       rasterisers; time both and report how they differ
  lockstep [STEPS]     run the recompiled i960 code and the interpreter side
                       by side, block by block; report the first difference
  run [STEPS] [SNAP]   boot on the reference interpreter and report what
                       the game did (default 50M instructions); writes the
                       tile layer to out/GAME/frames/ every SNAP frames
                       (0 = final frame only)
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
    let mut args: Vec<String> = std::env::args().skip(1).collect();
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
            view.hud_windows = g.hud_windows.iter().fold(0, |m, &w| m | 1 << w);
        }
        let _ = VIEW.set(view);
    }
    let (Ok(dump_textures), Ok(texture_pack)) = (opt("--dump-textures"), opt("--texture-pack")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
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
    if cmd == "scan" {
        let sets = romset::sets_in(&rom_dir);
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
    if cmd == "games" {
        for g in games::ALL {
            println!("{:<10} {:<20} {:?}", g.id, g.title, g.board);
        }
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
            let out = PathBuf::from("out/recomp");
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
                for &addr in &trace.executed {
                    insns.entry(addr).or_insert_with(|| img.decode(addr));
                }
                let mut entries: BTreeSet<u32> = a.functions.iter().copied().collect();
                entries.extend(trace.indirect.iter().map(|&(_, t)| t));
                entries.extend(seeds);
                let hash = model2::program_hash(&roms);
                let path = out.join(format!("i960_{hash:016x}.rs"));
                let code = i960::codegen::program_module(&insns, &entries, hash);
                let _ = std::fs::write(&path, &code);
                let blocks = code.matches("\nfn b").count();
                println!("i960: {} instructions in {blocks} blocks -> {}", insns.len(), path.display());
                made += 1;
            }
            for name in ["copro", "geo"] {
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
                let path = out.join(format!("tgp_{hash:016x}.rs"));
                let _ = std::fs::write(&path, mb86235::codegen::program_module(&prog, &words));
                println!("{name}: {} of {} words recompiled -> {}", words.len(), prog.len(), path.display());
                made += 1;
            }
            if made == 0 {
                return ExitCode::FAILURE;
            }
        }
        "gpu-check" => {
            let default = PathBuf::from("out").join(roms.game.id).join("capture");
            let root = args.get(1).map(PathBuf::from).unwrap_or(default);
            let windows = roms.game.hud_windows.iter().fold(0, |m, &w| m | 1 << w);
            if let Err(e) = gpu_check::run(&root, windows) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        // Debug: run recompiled and interpreted i960s side by side, one
        // instruction at a time, and report the first that differs.
        "lockstep" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(50_000_000);
            let mut native = model2::Machine::new(&roms);
            let mut reference = model2::Machine::new(&roms);
            reference.disable_native();
            if !native.is_native() {
                eprintln!("error: this build has no recompiled i960 code for {}", roms.game.id);
                return ExitCode::FAILURE;
            }
            while native.cpu.icount < steps {
                let ip = native.cpu.ip;
                let before = native.cpu.r;
                let a = native.run_block();
                let mut b = Ok(());
                while b.is_ok() && reference.cpu.icount < native.cpu.icount {
                    b = reference.step();
                }
                let (n, r) = (&native.cpu, &reference.cpu);
                if a.is_err() || b.is_err() || n.r != r.r || n.ip != r.ip || n.ac != r.ac || n.pc != r.pc {
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
        "run" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(50_000_000);
            let snap = args.get(2).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(0);
            let script = match model2::input::InputScript::parse(input.as_deref().unwrap_or("")) {
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

/// Output view for snapshots (`--height`, `--aspect`, `--hud`); native when unset.
static VIEW: std::sync::OnceLock<model2::video::View> = std::sync::OnceLock::new();
/// `--save-state`: where, and at which frame.
static SAVE_STATE: std::sync::OnceLock<(PathBuf, u64)> = std::sync::OnceLock::new();
/// `--load-state`: the state to start from.
static LOAD_STATE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// Where `--audio` records the live sound.
static AUDIO: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// Where `--capture` saves snapshot frames.
static CAPTURE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn snapshot(m: &mut model2::Machine, path: &std::path::Path) {
    let view = VIEW.get().copied().unwrap_or_else(model2::video::View::native);
    m.bus.geo.sync();
    if let Some(root) = CAPTURE.get() {
        let frame = model2::capture::Frame::grab(&mut m.bus);
        if let Err(e) = frame.save(root, &m.bus.texture_rom) {
            eprintln!("capture: {}: {e}", root.display());
        }
    }
    let b = &m.bus;
    let geo = b.geo.engine();
    let (xoff, yoff) = b.sync_offsets();
    let src = model2::raster::Sources {
        texture_rom: &b.texture_rom,
        texture_point_ram: &geo.texture_point_ram,
        render_ram: b.render_ram(),
        palette: b.palette_ram(),
        xoff,
        yoff,
    };
    let mut polys = model2::raster::build(&geo.stream, &src);
    let mut packs = texpack::PACKS.lock().unwrap();
    let (rgb3d, covered) = match packs.as_mut() {
        Some(p) => {
            p.frame = m.frames;
            model2::raster::draw_enhanced(&mut polys, &src, &view, p)
        }
        None => model2::raster::draw_view(&mut polys, &src, &view),
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
    let png = png::encode_rgb(view.width, view.height, &rgb);
    let _ = std::fs::write(path, png);
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
    let mut m = model2::Machine::new(roms);
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
            Ok(()) => println!("state         loaded {} (frame {})", path.display(), m.frames),
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
        if snap > 0 && m.frames != last_frame && m.frames.is_multiple_of(snap) {
            let path = frames_dir.join(format!("{:06}.png", m.frames));
            snapshot(&mut m, &path);
        }
        last_frame = m.frames;
    }
    snapshot(&mut m, &frames_dir.join("final.png"));
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
    println!(
        "copro         program {} words, booted {}, fifo in {} out {}, empty reads {}",
        b.copro_program.len(),
        b.copro_booted,
        b.tgp.fifo_in.len(),
        b.tgp.fifo_out.len(),
        b.copro_empty_reads
    );
    println!("tgp           {} words executed, pc {:#05x}, fault {:x?}", b.tgp.steps, b.tgp.pc, b.tgp_fault);
    println!("copro results hash {:016x}", b.copro_result_hash);
    println!("state hash    {:016x} (ip {:#x})", state_hash, m.cpu.ip);
    let tgp_cov: Vec<u32> = (0..4096).filter(|&a| b.tgp.counts[a as usize] > 0).collect();
    println!("tgp coverage  {} of 4096 words executed", tgp_cov.len());
    let tgp_txt: String = tgp_cov.iter().map(|a| format!("{a:03x}\n")).collect();
    let _ = std::fs::write(dir.join("copro-coverage.txt"), tgp_txt);
    println!("geo           {} data words", b.geo_words);
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
    println!("hardware registers ({} touched):", b.io_log.len());
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
