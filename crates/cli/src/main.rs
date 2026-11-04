mod png;

use analyze::{explore, read_boot, Image, Trace};
use romset::{games, RomSet};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: remodel [--game ID] [--rom DIR] <command>

options:
  --game ID            game manifest to use (default topskatr)
  --rom DIR            directory of unpacked chips (default rom)

commands:
  games                list supported games
  verify               check every chip against known-good hashes
  boot                 print the boot record and system table seeds
  disasm ADDR [COUNT]  linear disassembly of the program ROM
  analyze [OUT]        recursive-descent listing (default out/GAME/program.lst)
  copro-disasm [FILE]  disassemble the uploaded geometry program
                       (default out/GAME/copro.bin, written by `run`)
  run [STEPS] [SNAP]   boot on the reference interpreter and report what
                       the game did (default 50M instructions); writes the
                       tile layer to out/GAME/frames/ every SNAP frames
                       (0 = final frame only)
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
    let (Ok(game_id), Ok(rom_dir)) = (opt("--game"), opt("--rom")) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let game_id = game_id.unwrap_or_else(|| "topskatr".into());
    let rom_dir = PathBuf::from(rom_dir.unwrap_or_else(|| "rom".into()));
    let Some(cmd) = args.first().cloned() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
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
            copro_disasm(&bin, &dir.join("copro.lst"));
        }
        "run" => {
            let steps = args.get(1).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(50_000_000);
            let snap = args.get(2).and_then(|s| parse_num(s)).map(u64::from).unwrap_or(0);
            run(&roms, steps, snap);
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn snapshot(m: &model2::Machine, path: &std::path::Path) {
    let rgb = model2::video::render_tiles(m.bus.tile_ram(), m.bus.palette_ram());
    let png = png::encode_rgb(model2::video::WIDTH as u32, model2::video::HEIGHT as u32, &rgb);
    let _ = std::fs::write(path, png);
}

fn run(roms: &RomSet, steps: u64, snap: u64) {
    let mut m = model2::Machine::new(roms);
    let dir = PathBuf::from("out").join(roms.game.id);
    let eeprom_path = dir.join("eeprom.bin");
    if let Ok(bytes) = std::fs::read(&eeprom_path) {
        m.bus.eeprom.load(&bytes);
        println!("eeprom        loaded {}", eeprom_path.display());
    }
    let frames_dir = dir.join("frames");
    let _ = std::fs::remove_dir_all(&frames_dir);
    let _ = std::fs::create_dir_all(&frames_dir);
    let started = std::time::Instant::now();
    let mut stop = None;
    let mut last_frame = 0;
    for _ in 0..steps {
        if let Err(s) = m.step() {
            stop = Some(s);
            break;
        }
        if snap > 0 && m.frames != last_frame && m.frames.is_multiple_of(snap) {
            snapshot(&m, &frames_dir.join(format!("{:06}.png", m.frames)));
        }
        last_frame = m.frames;
    }
    snapshot(&m, &frames_dir.join("final.png"));
    let secs = started.elapsed().as_secs_f64();
    let b = &m.bus;

    match stop {
        Some(s) => println!("stopped       {s:?}"),
        None => println!("stopped       step limit"),
    }
    println!("instructions  {} ({:.1} M/s)", m.cpu.icount, m.cpu.icount as f64 / secs / 1e6);
    println!("emulated      {} frames ({:.2}s)", m.frames, m.frames as f64 / 57.524);
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
    let tgp_cov: Vec<u32> = (0..4096).filter(|&a| b.tgp.executed[a as usize]).collect();
    println!("tgp coverage  {} of 4096 words executed", tgp_cov.len());
    let tgp_txt: String = tgp_cov.iter().map(|a| format!("{a:03x}\n")).collect();
    let _ = std::fs::write(dir.join("copro-coverage.txt"), tgp_txt);
    println!("geo           {} data words", b.geo_words);
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
    let top = |log: &BTreeMap<u32, model2::bus::Access>, n: usize| {
        let mut v: Vec<_> = log.iter().collect();
        v.sort_by_key(|(_, a)| std::cmp::Reverse(a.reads + a.writes));
        v.into_iter().take(n).map(|(&k, &a)| (k, a)).collect::<Vec<_>>()
    };
    println!("hardware registers ({} touched):", b.io_log.len());
    for (a, e) in top(&b.io_log, 16) {
        println!("  {a:08x}  r {:>9}  w {:>9}  last {:#x}", e.reads, e.writes, e.last_write);
    }
    if !b.unmapped.is_empty() {
        println!("unmapped ({} addresses):", b.unmapped.len());
        for (a, e) in top(&b.unmapped, 12) {
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
    let cov: String = m.coverage.iter().map(|a| format!("{a:08x}\n")).collect();
    let ind: String = m.indirect.iter().map(|(s, t)| format!("{s:08x} {t:08x}\n")).collect();
    let _ = std::fs::write(dir.join("coverage.txt"), cov);
    let _ = std::fs::write(dir.join("indirect.txt"), ind);
    println!("trace written to {}/{{coverage,indirect}}.txt", dir.display());
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
