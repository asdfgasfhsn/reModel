use analyze::{explore, read_boot, Image};
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
  analyze [OUT]        recursive-descent listing (default out/program.lst)
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
            let a = explore(&img, &seeds);

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
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
