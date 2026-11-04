//! `sound-render`: replay a captured host command stream into the sound
//! board and write the result as a WAV file.

use romset::{Role, RomSet};
use sound::board::SAMPLE_RATE;
use sound::SoundBoard;
use std::path::Path;

/// Main-board video frame rate; `sound-tx.txt` is stamped in frames.
const FRAME_HZ: f64 = 57.524;

/// Parse `sound-tx.txt`: one UART write per line, `<frame> data|ctl <hex>`.
/// Returns the data bytes with their frame numbers.
pub fn parse_tx(text: &str) -> Vec<(u64, u8)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let frame = f.next()?.parse().ok()?;
            (f.next()? == "data").then_some(())?;
            Some((frame, u8::from_str_radix(f.next()?, 16).ok()?))
        })
        .collect()
}

fn frame_to_sample(frame: u64) -> u64 {
    (frame as f64 * SAMPLE_RATE as f64 / FRAME_HZ) as u64
}

/// Render `tx` through a freshly reset board. `seconds` = total length
/// (default: until 3 s after the last byte).
pub fn render(roms: &RomSet, tx: &[(u64, u8)], seconds: Option<f64>, wav: &Path) -> Result<(), String> {
    let prog = roms.region16be(Role::AudioCpu);
    let samples = roms.region16be(Role::Samples);
    if prog.is_empty() {
        return Err(format!("{}: no sound program in the manifest", roms.game.id));
    }
    let last = tx.last().map_or(0, |&(f, _)| frame_to_sample(f));
    let total = match seconds {
        Some(s) => (s * SAMPLE_RATE as f64) as u64,
        None => last + 3 * SAMPLE_RATE as u64,
    };
    let mut board = SoundBoard::new(prog, samples);
    let trace = std::env::var_os("REMODEL_SOUND_TRACE").is_some();
    if trace {
        board.bus.scsp.key_log = Some(Vec::new());
    }
    println!("reset         SSP {:#x} PC {:#x}", board.cpu.a[7], board.cpu.pc);
    let started = std::time::Instant::now();
    let mut out = Vec::with_capacity(total as usize * 2);
    let mut next = 0;
    let mut first_irq_on = None;
    let mut slot_peak = [0u64; 33];
    // One-second chunks of loudness for a coarse profile.
    let mut profile = Vec::new();
    let chunk = 1024u64;
    let mut t = 0u64;
    while t < total {
        while next < tx.len() && frame_to_sample(tx[next].0) <= t {
            board.midi_in(tx[next].1);
            next += 1;
        }
        let n = chunk.min(total - t);
        let start = out.len();
        board.run(n as usize, &mut out);
        t += n;
        if first_irq_on.is_none() && board.cpu.sr() & 0x0700 != 0x0700 {
            first_irq_on = Some((t, board.cpu.pc));
        }
        let active = board.bus.scsp.slots.iter().filter(|s| s.active).count();
        slot_peak[active] += 1;
        let sum: f64 = out[start..].iter().map(|&v| (v as f64) * (v as f64)).sum();
        profile.push(sum);
    }
    let secs = started.elapsed().as_secs_f64();
    let emulated = total as f64 / SAMPLE_RATE as f64;

    let b = &board.bus;
    let c = &b.scsp.stats;
    println!("emulated      {emulated:.2}s of audio in {secs:.2}s host ({:.1}x real time)", emulated / secs);
    println!("68k           {} instructions, PC {:#08x}, SR {:#06x}", board.instructions, board.cpu.pc, board.cpu.sr());
    match first_irq_on {
        Some((t, pc)) => {
            println!("interrupts    first unmasked at {:.3}s (PC {pc:#08x})", t as f64 / SAMPLE_RATE as f64)
        }
        None => println!("interrupts    never unmasked"),
    }
    let words = b.rom.len() / 2;
    println!(
        "coverage      {} of {words} program words started an instruction ({:.1}%)",
        board.covered_words(),
        100.0 * board.covered_words() as f64 / words as f64
    );
    let mut hot: Vec<(usize, u64)> = board.hot.iter().copied().enumerate().filter(|&(_, n)| n > 0).collect();
    hot.sort_by_key(|&(i, n)| (std::cmp::Reverse(n), i));
    let hot: Vec<String> = hot
        .iter()
        .take(6)
        .map(|&(i, n)| format!("{:#08x} {:.1}%", 0x60_0000 + i * 256, 100.0 * n as f64 / board.instructions as f64))
        .collect();
    println!("hot blocks    {}", hot.join(", "));
    println!(
        "midi in       {} bytes queued, {} delivered, {} lost to FIFO overflow, {} left on the link, {} in the FIFO",
        tx.len(),
        c.midi_in,
        c.midi_overflows,
        board.link_pending(),
        b.scsp.midi_pending()
    );
    println!("midi out      {} bytes written to MOBUF", c.midi_out);
    let used: Vec<String> =
        c.slot_key_ons.iter().enumerate().filter(|(_, &n)| n > 0).map(|(i, n)| format!("{i}:{n}")).collect();
    println!("key-ons       {} total, {} slots used ({})", c.key_ons, used.len(), used.join(" "));
    let busiest = slot_peak.iter().rposition(|&n| n > 0).unwrap_or(0);
    println!("polyphony     up to {busiest} slots sounding at a chunk boundary");
    println!(
        "scsp          DMA {} transfers, DSP program writes {}{}, MVOL {}, ring {:#06x}",
        c.dma,
        c.dsp_program_writes,
        if b.scsp.dsp.enabled() { format!(" (program {} steps)", b.scsp.dsp.program_len()) } else { String::new() },
        b.scsp.common_reg(0x400) & 0xf,
        b.scsp.common_reg(0x402)
    );
    println!(
        "bus           sample ROM reads {}, latch writes {} (last {:#04x}), ROM writes {}",
        b.stats.sample_rom_reads, b.stats.latch_writes, b.latch, b.stats.rom_writes
    );
    println!("unmapped      {} reads, {} writes", b.stats.unmapped_reads, b.stats.unmapped_writes);
    for &(a, w, pc) in &b.stats.unmapped {
        println!("  {} {a:#08x} at PC {pc:#08x}", if w { "write" } else { "read " });
    }

    let peak = out.iter().map(|&v| (v as i32).abs()).max().unwrap_or(0);
    let rms = (out.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / out.len().max(1) as f64).sqrt();
    let db = |x: f64| if x > 0.0 { 20.0 * (x / 32768.0).log10() } else { f64::NEG_INFINITY };
    let clipped = out.iter().filter(|&&v| v == i16::MAX || v == i16::MIN).count();
    println!(
        "level         peak {peak} ({:.1} dBFS), RMS {rms:.0} ({:.1} dBFS), {clipped} clipped samples",
        db(peak as f64),
        db(rms)
    );
    // RMS per 5-second window.
    let per = (5 * SAMPLE_RATE as u64 / chunk) as usize;
    let windows: Vec<String> = profile
        .chunks(per)
        .map(|w| {
            let n = (w.len() as u64 * chunk * 2) as f64;
            format!("{:.0}", db((w.iter().sum::<f64>() / n).sqrt()).max(-99.0))
        })
        .collect();
    println!("RMS per 5 s   {} dBFS", windows.join(" "));

    if let Some(log) = &b.scsp.key_log {
        let text: String = log
            .iter()
            .map(|(t, n, on, r)| {
                let regs: Vec<String> = r.iter().map(|w| format!("{w:04x}")).collect();
                format!(
                    "{:9.4} {n:2} {} {}\n",
                    *t as f64 / SAMPLE_RATE as f64,
                    if *on { "on " } else { "off" },
                    regs.join(" ")
                )
            })
            .collect();
        let dsp = &b.scsp.dsp;
        let prog = &dsp.mpro[..dsp.program_len()];
        let ewa: std::collections::BTreeSet<u64> =
            prog.iter().filter(|&&o| o >> 28 & 1 != 0).map(|o| o >> 24 & 0xf).collect();
        let ira: std::collections::BTreeSet<u64> = prog.iter().map(|o| o >> 38 & 0x3f).filter(|&i| i >= 0x20).collect();
        let mem = prog.iter().filter(|&&o| o >> 29 & 3 != 0).count();
        println!("dsp           writes EFREG {ewa:?}, reads MIXS/EXTS inputs {ira:x?}, {mem} memory steps");
        let slots: Vec<String> = b.scsp.slots[..16].iter().map(|s| format!("{:03x}", s.regs[11] & 0xff)).collect();
        println!("efsdl/efpan   slots 0-15 (low byte of +0x16): {}", slots.join(" "));
        let path = wav.with_extension("keys.txt");
        let _ = std::fs::write(&path, text);
        println!("key trace     {} events to {}", log.len(), path.display());
    }
    if let Some(d) = wav.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(wav, sound::wav::encode(SAMPLE_RATE, 2, &out)).map_err(|e| format!("{}: {e}", wav.display()))?;
    println!("wrote         {}", wav.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_tx_lines() {
        let t = "    88 ctl  00\n    88 data ff\n  4141 data a1\nbogus\n";
        assert_eq!(super::parse_tx(t), vec![(88, 0xff), (4141, 0xa1)]);
    }
}
