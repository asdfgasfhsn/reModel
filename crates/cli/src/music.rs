//! `music-render` and `mpeg-dump`: the DSB2 and DSB1 music boards and their decoder.

use romset::{Role, RomSet};
use std::path::Path;
use std::time::Instant;

/// Host frames per second: the rate `run` stamps on `sound-tx.txt` lines.
const HOST_FPS: f64 = model2::pacing::FPS;

/// Write 16-bit PCM as a WAV file.
pub fn write_wav(path: &Path, rate: u32, channels: u16, pcm: &[i16]) -> std::io::Result<()> {
    std::fs::write(path, sound::wav::encode(rate, channels, pcm))
}

/// Peak, RMS (both relative to full scale) and clipped-sample count.
fn levels(pcm: &[i16]) -> (f64, f64, usize) {
    let peak = pcm.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0) as f64 / 32768.0;
    let rms = (pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt() / 32768.0;
    let clipped = pcm.iter().filter(|&&s| s == i16::MAX || s == i16::MIN).count();
    (peak, rms, clipped)
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-10).log10()
}

/// Decode `seconds` of MPEG frames starting at `addr` straight from the ROM.
/// Writes `mpeg-ADDR.wav` (native rate), `.f32` (raw float, interleaved) and
/// `.mp2` (the frames' bytes, for comparison with another decoder).
pub fn mpeg_dump(roms: &RomSet, addr: u32, seconds: f64, dir: &Path) -> Result<(), String> {
    let rom = roms.mpeg_data();
    let mut at = mpeg::sync(&rom, addr as usize).ok_or("no frame header at or after the address")?;
    if at != addr as usize {
        println!("first frame header at {at:#x}");
    }
    let start = at;
    let mut dec = mpeg::Decoder::new();
    let mut out = [[0.0; mpeg::FRAME_SAMPLES]; 2];
    let (mut pcm, mut float) = (Vec::new(), Vec::new());
    let mut first: Option<mpeg::Header> = None;
    let (mut frames, mut odd) = (0, 0);
    let mut rate = 0;
    while (frames as f64) * (mpeg::FRAME_SAMPLES as f64) < seconds * rate.max(1) as f64 || frames == 0 {
        let h = match dec.decode_frame(&rom[at..], &mut out) {
            Ok(h) => h,
            Err(e) => {
                println!("stopped at {at:#x}: {e}");
                break;
            }
        };
        rate = h.sample_rate;
        let f = *first.get_or_insert(h);
        if (h.bitrate, h.sample_rate, h.mode) != (f.bitrate, f.sample_rate, f.mode) {
            odd += 1;
        }
        for i in 0..mpeg::FRAME_SAMPLES {
            for ch in &out {
                float.extend(ch[i].to_le_bytes());
                pcm.push((ch[i] * 32767.0).clamp(-32768.0, 32767.0) as i16);
            }
        }
        at += h.frame_len();
        frames += 1;
    }
    let h = first.ok_or("no frames decoded")?;
    println!(
        "{frames} frames {start:#x}-{at:#x}: {} kbit/s, {} Hz, {:?}{}; {odd} frames with other parameters",
        h.bitrate / 1000,
        h.sample_rate,
        h.mode,
        if h.crc { ", CRC" } else { "" }
    );
    let (peak, rms, clipped) = levels(&pcm);
    println!("peak {:.1} dBFS, rms {:.1} dBFS, {clipped} clipped samples", db(peak), db(rms));
    let stem = dir.join(format!("mpeg-{start:x}"));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_wav(&stem.with_extension("wav"), h.sample_rate, 2, &pcm).map_err(|e| e.to_string())?;
    std::fs::write(stem.with_extension("f32"), float).map_err(|e| e.to_string())?;
    std::fs::write(stem.with_extension("mp2"), &rom[start..at]).map_err(|e| e.to_string())?;
    println!("wrote {}.{{wav,f32,mp2}}", stem.display());
    Ok(())
}

/// Replay the host's UART stream (`sound-tx.txt` from `run`) into the DSB2
/// and render its music. With `soundtrack`, replaced tracks play from its files.
pub fn music_render(
    roms: &RomSet,
    tx: &Path,
    wav: &Path,
    seconds: Option<f64>,
    soundtrack: Option<&Path>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(tx).map_err(|e| format!("{}: {e} (make it with `run`)", tx.display()))?;
    let bytes: Vec<(f64, u8)> = text
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let frame: f64 = w.next()?.parse().ok()?;
            (w.next()? == "data").then_some(())?;
            Some((frame / HOST_FPS, u8::from_str_radix(w.next()?, 16).ok()?))
        })
        .collect();
    let last = bytes.last().map_or(0.0, |b| b.0);
    let seconds = seconds.unwrap_or(last + 5.0);
    println!("{} host bytes over {last:.1} s; rendering {seconds:.1} s", bytes.len());

    if !roms.game.files.iter().any(|f| f.role == Role::MpegCpu) {
        return dsb1_render(roms, &bytes, seconds, wav, soundtrack);
    }

    // Music commands as the host sent them (channel 14, `AE cc vv`).
    let mut status = 0;
    let mut msg = vec![];
    for &(t, b) in &bytes {
        if b & 0x80 != 0 {
            status = b;
            msg.clear();
        } else if status == 0xae {
            msg.push(b);
            if msg.len() == 2 {
                let what = match msg[0] {
                    0x10 if msg[1] == 0 => "stop".into(),
                    0x10 => format!("play track {}", msg[1]),
                    0x02 => format!("master volume {:#04x}", msg[1]),
                    0x03 => format!("fade out, rate {:#04x}", msg[1]),
                    _ => "?".into(),
                };
                println!("  host {t:7.2} s  AE {:02X} {:02X}  {what}", msg[0], msg[1]);
                msg.clear();
            }
        }
    }

    let program = roms.game.files.iter().position(|f| f.role == Role::MpegCpu).ok_or("game has no DSB2")?;
    let mut dsb = dsb::Dsb2::new(&roms.chips[program], roms.mpeg_data());
    let table = dsb.track_table();
    println!("firmware track table: {} tracks", table.len());
    let track_of = |start: u32, end: u32| table.iter().position(|&r| r == (start, end)).map(|i| i + 1);
    if let Some(dir) = soundtrack {
        let st = dsb::Soundtrack::load(dir, &table)?;
        println!(
            "soundtrack {}{}: {} of {} tracks replaced",
            dir.display(),
            st.title.as_ref().map_or(String::new(), |t| format!(" ({t})")),
            st.replacements.len(),
            table.len()
        );
        for r in &st.replacements {
            let s = &r.spec;
            let mut extra = vec![];
            if s.loop_mode != dsb::audio::LoopMode::Game {
                extra.push(format!("loop {:?}", s.loop_mode).to_lowercase());
            }
            if s.loop_start.is_some() || s.loop_end.is_some() {
                let p = |p: Option<dsb::soundtrack::Pos>, d: &str| p.map_or(d.to_string(), |p| p.to_string());
                extra.push(format!("loop {} to {}", p(s.loop_start, "start"), p(s.loop_end, "end")));
            }
            if s.gain != 1.0 {
                extra.push(format!("gain {:+.1} dB", db(s.gain as f64)));
            }
            println!(
                "  track {:2} {:#08x}-{:#08x} <- {} ({}{})",
                s.track,
                r.start,
                r.end,
                s.file.display(),
                r.info,
                extra.iter().map(|e| format!("; {e}")).collect::<String>()
            );
        }
        let kept: Vec<String> = (1..=table.len())
            .filter(|t| !st.replacements.iter().any(|r| r.spec.track as usize == *t))
            .map(|t| t.to_string())
            .collect();
        println!("  original: tracks {}", if kept.is_empty() { "none".into() } else { kept.join(", ") });
        dsb.set_soundtrack(st);
    }
    let total = (seconds * dsb::OUT_HZ as f64) as usize;
    let mut pcm = Vec::with_capacity(total * 2);
    let mut next = 0;
    let chunk = 441;
    let t0 = Instant::now();
    while pcm.len() / 2 < total {
        let now = (pcm.len() / 2) as f64 / dsb::OUT_HZ as f64;
        while next < bytes.len() && bytes[next].0 <= now {
            dsb.midi_in(bytes[next].1);
            next += 1;
        }
        dsb.run(chunk.min(total - pcm.len() / 2), &mut pcm);
    }
    let host = t0.elapsed().as_secs_f64();

    let s = dsb.stats();
    println!(
        "68k: {} instructions, {} ticks, {} UART bytes in ({} overruns), {} forwarded out, {} upload bytes, {} MPEG frames{}",
        s.instructions,
        s.ticks,
        s.rx_bytes,
        s.overruns,
        dsb.tx().len(),
        s.upload_bytes,
        s.frames,
        if dsb.halted() { ", HALTED" } else { "" }
    );
    let pcs = dsb.pc_histogram();
    let n: u64 = pcs.iter().map(|p| p.1).sum();
    let top: Vec<String> =
        pcs.iter().take(8).map(|(a, c)| format!("{a:06x} {:.0}%", 100.0 * *c as f64 / n as f64)).collect();
    println!("68k PCs: {}", top.join(", "));
    for (a, c) in dsb.unmapped() {
        println!("  unmapped {} {:06x} x{c}", if a & 0x8000_0000 != 0 { "write" } else { "read" }, a & 0xffffff);
    }
    let names = ["A0 L<-L", "A1 L<-R", "A2", "A3", "A4 R<-L", "A5 R<-R", "A6", "A7"];
    let mut unknown = std::collections::BTreeMap::new();
    for (t, e) in dsb.events() {
        let t = *t as f64 / dsb::OUT_HZ as f64;
        if let dsb::Event::Unknown(c) = e {
            *unknown.entry(*c).or_insert(0) += 1;
            continue;
        }
        let what = match e {
            dsb::Event::Play { start, end, looped } => format!(
                "play {}{start:#08x}-{end:#08x} ({:.1} s){}",
                track_of(*start, *end).map_or(String::new(), |t| format!("track {t} ")),
                (end - start) as f64 / 576.0 * 0.036,
                if *looped { ", looping" } else { "" }
            ),
            dsb::Event::Replaced { track } => format!("  -> replaced: soundtrack track {track}"),
            dsb::Event::ReplaceFailed { track } => format!("  -> soundtrack track {track} failed to load, ROM plays"),
            dsb::Event::ReplacementEnd { track, looped: true } => format!("soundtrack track {track} looped"),
            dsb::Event::ReplacementEnd { track, looped: false } => format!("soundtrack track {track} ended, stopped"),
            dsb::Event::LoopStart(a) => format!("loop point {a:#08x}"),
            dsb::Event::SegmentEnd { restart: Some(a) } => format!("end reached, loop to {a:#08x}"),
            dsb::Event::SegmentEnd { restart: None } => "end reached, stopped".into(),
            dsb::Event::Stop => "stop".into(),
            dsb::Event::Gain { reg, value } if reg & 0x10 == 0 => {
                format!("gain {} = {value:#04x}", names[(reg & 7) as usize])
            }
            dsb::Event::Gain { reg, value } => format!("gain {reg:02X} = {value:#04x}"),
            dsb::Event::BadFrame(a) => format!("no frame at {a:#08x}, stopped"),
            dsb::Event::Attenuation(v) => format!("attenuation {v:#04x}"),
            dsb::Event::Unknown(_) => unreachable!(),
        };
        println!("  dsb2 {t:7.2} s  {what}");
    }
    let unknown: Vec<String> = unknown.iter().map(|(c, n)| format!("{c:02X} x{n}")).collect();
    println!("decoder bytes with no known meaning (boot upload and setup): {}", unknown.join(", "));
    if let Some(st) = dsb.soundtrack() {
        for e in &st.errors {
            println!("soundtrack error: {e}");
        }
    }
    let (peak, rms, clipped) = levels(&pcm);
    println!("peak {:.1} dBFS, rms {:.1} dBFS, {clipped} clipped samples", db(peak), db(rms));
    println!("rendered {seconds:.1} s in {host:.2} s ({:.0}x real time)", seconds / host);
    if let Some(d) = wav.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    write_wav(wav, dsb::OUT_HZ as u32, 2, &pcm).map_err(|e| e.to_string())?;
    println!("wrote {}", wav.display());
    Ok(())
}

/// Load a soundtrack against `table` and print what it replaces.
fn load_soundtrack(dir: &Path, table: &[(u32, u32)]) -> Result<dsb::Soundtrack, String> {
    let st = dsb::Soundtrack::load(dir, table)?;
    println!(
        "soundtrack {}{}: {} of {} tracks replaced",
        dir.display(),
        st.title.as_ref().map_or(String::new(), |t| format!(" ({t})")),
        st.replacements.len(),
        table.len()
    );
    for r in &st.replacements {
        println!(
            "  track {:2} {:#08x}-{:#08x} <- {} ({})",
            r.spec.track,
            r.start,
            r.end,
            r.spec.file.display(),
            r.info
        );
    }
    Ok(st)
}

/// `music_render` for the DSB1 (Z80) board: the host's channel-15 (`AF`)
/// commands, the decoder's events, and per-second levels.
fn dsb1_render(
    roms: &RomSet,
    bytes: &[(f64, u8)],
    seconds: f64,
    wav: &Path,
    soundtrack: Option<&Path>,
) -> Result<(), String> {
    let mut status = 0;
    let mut msg = vec![];
    for &(t, b) in bytes {
        if b & 0x80 != 0 {
            status = b;
            msg.clear();
        } else if status == 0xaf {
            msg.push(b);
            if msg.len() == 2 {
                let what = match msg[0] {
                    0x6f if msg[1] == 0 => "stop".into(),
                    0x6f => format!("play track {}", msg[1]),
                    0x01 => format!("volume scale {:#04x}", msg[1]),
                    0x02 => format!("volume {:#04x}", msg[1]),
                    0x03 => format!("fade out, rate {:#04x}", msg[1]),
                    _ => "?".into(),
                };
                println!("  host {t:7.2} s  AF {:02X} {:02X}  {what}", msg[0], msg[1]);
                msg.clear();
            }
        }
    }

    let program = roms.game.files.iter().position(|f| f.role == Role::Dsb1Cpu).ok_or("game has no music board")?;
    let mut dsb = dsb::Dsb1::new(&roms.chips[program], roms.mpeg_data());
    let table = dsb.track_table();
    println!("firmware track list: {} tracks", table.len());
    if let Some(dir) = soundtrack {
        dsb.set_soundtrack(load_soundtrack(dir, &table)?);
    }
    let track_of = |start: u32| table.iter().position(|&r| r.0 == start).map(|i| i + 1);
    let total = (seconds * dsb::OUT_HZ as f64) as usize;
    let mut pcm = Vec::with_capacity(total * 2);
    let mut next = 0;
    let t0 = Instant::now();
    while pcm.len() / 2 < total {
        let now = (pcm.len() / 2) as f64 / dsb::OUT_HZ as f64;
        while next < bytes.len() && bytes[next].0 <= now {
            dsb.midi_in(bytes[next].1);
            next += 1;
        }
        dsb.run(441.min(total - pcm.len() / 2), &mut pcm);
    }
    let host = t0.elapsed().as_secs_f64();

    let s = dsb.stats();
    println!(
        "z80: {} instructions, {} UART bytes in ({} overruns), {} echoed, {} MPEG frames{}",
        s.instructions,
        s.rx_bytes,
        s.overruns,
        s.tx_bytes,
        s.frames,
        if dsb.halted() { ", HALTED" } else { "" }
    );
    let pcs = dsb.pc_histogram();
    let n: u64 = pcs.iter().map(|p| p.1).sum();
    let top: Vec<String> =
        pcs.iter().take(8).map(|(a, c)| format!("{a:04x} {:.0}%", 100.0 * *c as f64 / n as f64)).collect();
    println!("z80 PCs: {}", top.join(", "));
    for (p, v, c) in dsb.other_ports() {
        match v {
            Some(v) => println!("  port {p:02X} <- {v:02X} x{c}"),
            None => println!("  port {p:02X} read x{c}"),
        }
    }
    let events = dsb.events();
    for (i, (t, e)) in events.iter().enumerate() {
        // A fade steps the attenuation every few ms: show where each run ends.
        let fading = |j: usize| matches!(events.get(j), Some((u, dsb::Event::Attenuation(_))) if u - t < 4410);
        if matches!(e, dsb::Event::Attenuation(_)) && fading(i + 1) {
            continue;
        }
        let t = *t as f64 / dsb::OUT_HZ as f64;
        let what = match e {
            dsb::Event::Attenuation(v) => format!("attenuation {v:#04x}"),
            dsb::Event::Play { start, end, looped } => format!(
                "play {}{start:#08x}-{end:#08x} ({:.1} s){}",
                track_of(*start).map_or(String::new(), |t| format!("track {t} ")),
                (end - start) as f64 / 576.0 * 0.036,
                if *looped { ", continuing" } else { "" }
            ),
            dsb::Event::LoopStart(a) => format!("next segment from {a:#08x}"),
            dsb::Event::SegmentEnd { restart: Some(a) } => format!("segment end, on to {a:#08x}"),
            dsb::Event::SegmentEnd { restart: None } => "segment end, stopped".into(),
            e => format!("{e:?}"),
        };
        println!("  dsb1 {t:7.2} s  {what}");
    }
    if let Some(st) = dsb.soundtrack() {
        for e in &st.errors {
            println!("soundtrack error: {e}");
        }
    }
    let per_second: Vec<String> = pcm
        .chunks(2 * dsb::OUT_HZ as usize)
        .map(|c| {
            let (_, rms, _) = levels(c);
            if rms > 0.0 {
                format!("{:.0}", db(rms))
            } else {
                "-".into()
            }
        })
        .collect();
    println!("rms dBFS per second: {}", per_second.join(" "));
    let (peak, rms, clipped) = levels(&pcm);
    println!("peak {:.1} dBFS, rms {:.1} dBFS, {clipped} clipped samples", db(peak), db(rms));
    println!("rendered {seconds:.1} s in {host:.2} s ({:.0}x real time)", seconds / host);
    if let Some(d) = wav.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    write_wav(wav, dsb::OUT_HZ as u32, 2, &pcm).map_err(|e| e.to_string())?;
    println!("wrote {}", wav.display());
    Ok(())
}
