//! Model 2A's TGP in a `run` report: the upload and the calls the game
//! made to the stand-in (`model2::tgp2a`; `docs/hw/model2a.md`).

use model2::tgp2a::Kind;
use std::fmt::Write as _;
use std::path::Path;

/// Print the TGP section of a `run` report and write what a later look
/// wants to `dir`: the program as uploaded (`tgp.bin`, little-endian 32-bit
/// words, for `tgp-disasm`) and the call log (`tgp-calls.txt`).
pub fn report(dir: &Path, b: &model2::Model2Bus) {
    let geo_bad = b.geo.bad_commands;
    let Some(t) = b.tgp2a.as_deref() else { return };
    println!(
        "tgp (2A)      {}, program {} words uploaded (hash {:016x}), booted {}, replies waiting {}, empty reads {}{}",
        if t.cpu.is_some() {
            "MB86234 interpreter"
        } else if t.hle.is_some() {
            "function model (tgp2a_hle: not the chip)"
        } else {
            "stand-in"
        },
        t.program.len(),
        model2::tgp2a_hle::program_hash(&t.program),
        t.booted,
        t.cpu.as_ref().map_or(t.fifo_out.len(), |c| c.fifo_out.len()),
        t.empty_reads,
        t.first_empty.map_or(String::new(), |(f, ip)| format!(" (first: frame {f}, ip {ip:#x})"))
    );
    if let Some(c) = &t.cpu {
        let list = |m: &std::collections::BTreeMap<u32, u64>| {
            m.iter().take(12).map(|(w, n)| format!("{w:08x}:{n}")).collect::<Vec<_>>().join(" ")
        };
        println!(
            "tgp chip      {} words run, pc {:#05x}, {} stuck accesses; unknown words {} [{}]; unknown external addresses {} [{}]",
            c.steps,
            c.pc,
            t.runaways,
            c.unknown.len(),
            list(&c.unknown),
            c.unknown_ext.len(),
            list(&c.unknown_ext)
        );
    }
    println!("geometrizer   bad commands {geo_bad}");
    if !t.program.is_empty() {
        let bin: Vec<u8> = t.program.iter().flat_map(|w| w.to_le_bytes()).collect();
        let _ = std::fs::write(dir.join("tgp.bin"), bin);
        println!("tgp program written to {}/tgp.bin", dir.display());
    }
    println!("tgp calls     fn    calls  args:calls               reads:calls  empty reads  frames");
    for (p, f) in &t.functions {
        let hist = |h: &std::collections::BTreeMap<u32, u64>| {
            h.iter().take(6).map(|(n, c)| format!("{n}:{c}")).collect::<Vec<_>>().join(" ")
        };
        println!(
            "  {p:>3} ({p:#04x}) {:>7}  {:<24} {:<24} {:>8}  {}-{}",
            f.calls,
            hist(&f.args),
            hist(&f.replies),
            f.empty_reads,
            f.first_frame,
            f.last_frame
        );
    }
    let mut text = String::new();
    for &(frame, ip, kind, a, v) in &t.trace {
        let _ = match kind {
            Kind::Command => {
                writeln!(text, "{frame:>6} {ip:08x} port  fn {:>3} +{a:04x} {v:08x}", model2::tgp2a::port(a))
            }
            Kind::Header => writeln!(
                text,
                "{frame:>6} {ip:08x} call  +{a:04x} fn {:>3} {v:08x}",
                model2::tgp2a::header(v).unwrap_or(0)
            ),
            Kind::Data => writeln!(text, "{frame:>6} {ip:08x} data  +{a:04x} {v:08x}"),
            Kind::Reply => writeln!(text, "{frame:>6} {ip:08x} reply {v:08x}"),
            Kind::EmptyReply => writeln!(text, "{frame:>6} {ip:08x} reply (empty)"),
        };
    }
    let _ = std::fs::write(dir.join("tgp-calls.txt"), text);
    // The 0x12000000 video RAM, for looking at (game-derived: stays in out/).
    for (name, addr) in [("vram2a-12000000.bin", 0x1200_0000), ("vram2a-12800000.bin", 0x1280_0000)] {
        if let Some(m) = b.memory_at(addr) {
            let _ = std::fs::write(dir.join(name), m);
        }
    }
}
