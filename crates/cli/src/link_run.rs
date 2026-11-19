//! `link-run`: several real machines of one game in one process, joined by
//! a link hub (`model2::link::LinkHub`), run in lockstep a frame at a time.
//! Experimental and headless (docs/hw/link-board.md, section 8a): it writes
//! side-by-side pictures of every machine's screen and a log of the
//! handshake, the nodes' states and what they put in their blocks.

use super::*;
use model2::link::{LinkHub, BLOCK, RX, TX};

const USAGE_LINK: &str = "usage: remodel --game G link-run [--nodes N] [--frames F] [--input0 S --input1 S ...]
        [--delay D] [--link-types 1,2,..] [--eeprom FILE] [--shot-every K] [--shot-at F,F,..]
        [--log-every K] [--trace NODE] [--drop-at F] [--freeze NODE:F] [--save-at F FILE] [--load FILE]
  --input S    inputs for every node without its own --inputN (as `run`'s --input)
  --link-types the LINK TYPE setting patched into each node's EEPROM (1 CAR 1 (master), 2 CAR 2, ..; default 1,2,3,4)
  --eeprom     the settings record to start from (default: the game's defaults); the user's files are never written
  --set-byte N:OFF:V   set byte OFF of node N's EEPROM image (before the CRC is fixed)
  --delay-to N:D[,..]  the other nodes' blocks reach node N D frames late (a test)
  --skew A,B,..        node i starts running A frames late (a test)
  --ids A,B,..         hand out these node IDs in place of master first (a test)
  --layout L           received-block order, 3 (default: own block last, 1-based IDs) or older guesses 0-2
  --dump-at F,..       write every node's block and received blocks at those frames
  --ram-dump N:ADDR:FILE, --watch N:ADDR   dump a node's memory at the end / log the writes to a word
  --fresh-hub          with --load: lose the board's state (only the machines were saved)
";

/// The settings record of a `srallyc` EEPROM at its defaults (docs/hw/link-board.md section 2): the
/// 4-byte header (CRC-16, length) and 32 bytes of settings, LINK TYPE 0 (NOTLINK); the bytes are those
/// a game's own first boot writes.
const DEFAULT_RECORD: [u8; 38] = [
    0x00, 0x00, 0x24, 0x00, 0x02, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0x01, 0x04, 0, 0, 0x80, 0x3f, 0xf2, 0xff, 0xff, 0xff,
    0x01, 0, 0x02, 0, 0, 0, 0x0b, 0, 0x01, 0, 0x01, 0, 0xff, 0xff,
];

/// The CRC-16 of a settings record: CCITT polynomial, start value `0xFFFF`,
/// complemented (docs/hw/link-board.md section 2).
fn record_crc(b: &[u8]) -> u16 {
    let mut c = 0xffffu16;
    for &x in b {
        c ^= (x as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    !c
}

/// A 128-byte EEPROM image from `base` with LINK TYPE set to `link` and the header's CRC
/// fixed. The game's work RAM from `0x202010` is the record from its first byte, header
/// included, so LINK TYPE (RAM `0x20201B`) is byte `0x0B` of the image.
fn patched_eeprom(base: Option<&[u8]>, link: u8, set: &[(usize, u8)]) -> Vec<u8> {
    let mut e = vec![0xffu8; 128];
    match base {
        Some(b) => e[..b.len().min(128)].copy_from_slice(&b[..b.len().min(128)]),
        None => e[..38].copy_from_slice(&DEFAULT_RECORD),
    }
    e[0x0b] = link;
    for &(o, v) in set {
        if o < 36 {
            e[o] = v;
        }
    }
    let c = record_crc(&e[2..36]);
    e[0] = c as u8;
    e[1] = (c >> 8) as u8;
    e
}

fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn f32at(b: &[u8], o: usize) -> f32 {
    f32::from_bits(u32at(b, o))
}

/// What a block holds, as far as srallyc's layout is known (docs/hw/link-board.md section 4).
fn block_summary(b: &[u8]) -> String {
    format!(
        "id {} car {} st {} seed {:#x} pos ({:.1},{:.1},{:.1}) clk {}",
        u16::from_le_bytes([b[0], b[1]]),
        u16::from_le_bytes([b[2], b[3]]),
        u32at(b, 0x18),
        u32at(b, 0x1c),
        f32at(b, 0x28),
        f32at(b, 0x2c),
        f32at(b, 0x30),
        u32at(b, 0x19c)
    )
}

struct Args {
    nodes: usize,
    frames: u64,
    delay: usize,
    types: Vec<u8>,
    eeprom: Option<PathBuf>,
    shot_every: u64,
    shot_at: Vec<u64>,
    log_every: u64,
    trace: Option<usize>,
    drop_at: u64,
    freeze: Option<(usize, u64)>,
    save: Option<(u64, PathBuf)>,
    dump_at: Vec<u64>,
    set_bytes: Vec<(usize, usize, u8)>,
    fresh_hub: bool,
    layout: u8,
    delay_to: Vec<(usize, usize)>,
    skew: Vec<u64>,
    ids: Option<Vec<u8>>,
    ram_dump: Option<(usize, u32, PathBuf)>,
    watch: Option<(usize, u32)>,
    load: Option<PathBuf>,
    inputs: Vec<Option<String>>,
}

fn parse(o: &Opts) -> Result<Args, String> {
    let mut a = Args {
        nodes: 2,
        frames: 3000,
        delay: 1,
        types: vec![1, 2, 3, 4],
        eeprom: None,
        shot_every: 0,
        shot_at: vec![],
        log_every: 0,
        trace: None,
        drop_at: 0,
        freeze: None,
        save: None,
        dump_at: vec![],
        set_bytes: vec![],
        fresh_hub: false,
        layout: 3,
        delay_to: vec![],
        skew: vec![0; 4],
        ids: None,
        ram_dump: None,
        watch: None,
        load: None,
        inputs: vec![None; 4],
    };
    let mut it = o.args.iter().skip(1);
    let num = |s: Option<&String>, what: &str| -> Result<u64, String> {
        s.and_then(|s| parse_num(s)).map(u64::from).ok_or_else(|| format!("{what} wants a number"))
    };
    while let Some(f) = it.next() {
        match f.as_str() {
            "--nodes" => a.nodes = num(it.next(), f)? as usize,
            "--frames" => a.frames = num(it.next(), f)?,
            "--delay" => a.delay = num(it.next(), f)? as usize,
            "--shot-every" => a.shot_every = num(it.next(), f)?,
            "--log-every" => a.log_every = num(it.next(), f)?,
            "--trace" => a.trace = Some(num(it.next(), f)? as usize),
            "--drop-at" => a.drop_at = num(it.next(), f)?,
            "--shot-at" => {
                let v = it.next().ok_or("--shot-at wants frames")?;
                a.shot_at = v.split(',').filter_map(|x| parse_num(x).map(u64::from)).collect();
            }
            "--dump-at" => {
                let v = it.next().ok_or("--dump-at wants frames")?;
                a.dump_at = v.split(',').filter_map(|x| parse_num(x).map(u64::from)).collect();
            }
            "--ram-dump" => {
                let v = it.next().ok_or("--ram-dump wants NODE:ADDR:FILE")?;
                let mut p = v.splitn(3, ':');
                let (n, ad, f) = (p.next(), p.next(), p.next());
                let (Some(n), Some(ad), Some(f)) = (n.and_then(|x| x.parse().ok()), ad.and_then(parse_num), f) else {
                    return Err("--ram-dump wants NODE:ADDR:FILE".into());
                };
                a.ram_dump = Some((n, ad, PathBuf::from(f)));
            }
            "--watch" => {
                let v = it.next().ok_or("--watch wants NODE:ADDR")?;
                let (n, ad) = v.split_once(':').ok_or("--watch wants NODE:ADDR")?;
                a.watch =
                    Some((n.parse().map_err(|_| "--watch: bad node")?, parse_num(ad).ok_or("--watch: bad address")?));
            }
            "--ids" => {
                let v = it.next().ok_or("--ids wants a list")?;
                a.ids = Some(v.split(',').filter_map(|x| x.parse().ok()).collect());
            }
            "--skew" => {
                let v = it.next().ok_or("--skew wants a list")?;
                a.skew = v.split(',').filter_map(|x| x.parse().ok()).collect();
                a.skew.resize(4, 0);
            }
            "--delay-to" => {
                let v = it.next().ok_or("--delay-to wants NODE:D[,NODE:D]")?;
                for part in v.split(',') {
                    let (n, d) = part.split_once(':').ok_or("--delay-to wants NODE:D")?;
                    a.delay_to.push((
                        n.parse().map_err(|_| "--delay-to: bad node")?,
                        d.parse().map_err(|_| "--delay-to: bad delay")?,
                    ));
                }
            }
            "--layout" => a.layout = num(it.next(), f)? as u8,
            "--set-byte" => {
                let v = it.next().ok_or("--set-byte wants NODE:OFFSET:VALUE")?;
                let p: Vec<u32> = v.split(':').filter_map(parse_num).collect();
                let [n, off, val] = p[..] else { return Err("--set-byte wants NODE:OFFSET:VALUE".into()) };
                a.set_bytes.push((n as usize, off as usize, val as u8));
            }
            "--fresh-hub" => a.fresh_hub = true,
            "--link-types" => {
                let v = it.next().ok_or("--link-types wants a list")?;
                a.types = v.split(',').filter_map(|x| x.parse().ok()).collect();
            }
            "--eeprom" => a.eeprom = Some(PathBuf::from(it.next().ok_or("--eeprom wants a file")?)),
            "--freeze" => {
                let v = it.next().ok_or("--freeze wants NODE:FRAME")?;
                let (n, fr) = v.split_once(':').ok_or("--freeze wants NODE:FRAME")?;
                a.freeze = Some((
                    n.parse().map_err(|_| "--freeze: bad node")?,
                    fr.parse().map_err(|_| "--freeze: bad frame")?,
                ));
            }
            "--save-at" => {
                let fr = num(it.next(), f)?;
                a.save = Some((fr, PathBuf::from(it.next().ok_or("--save-at wants FRAME FILE")?)));
            }
            "--load" => a.load = Some(PathBuf::from(it.next().ok_or("--load wants a file")?)),
            s if s.starts_with("--input") && s.len() == 8 => {
                let n = (s.as_bytes()[7] as char).to_digit(10).ok_or("bad --inputN")? as usize;
                if n >= 4 {
                    return Err("--inputN: N is 0 to 3".into());
                }
                a.inputs[n] = Some(it.next().ok_or("--inputN wants a script")?.clone());
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if !(1..=4).contains(&a.nodes) {
        return Err("--nodes: 1 to 4".into());
    }
    if a.types.len() < a.nodes {
        return Err("--link-types: one per node".into());
    }
    Ok(a)
}

/// Fold bytes into an FNV-1a style 64-bit hash.
fn fold(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

pub fn run(o: &Opts, roms: &RomSet) -> ExitCode {
    let a = match parse(o) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE_LINK}");
            return ExitCode::FAILURE;
        }
    };
    match go(o, roms, &a) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn go(o: &Opts, roms: &RomSet, a: &Args) -> Result<(), String> {
    let dir = OUT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("out").join("link-run"));
    let frames_dir = dir.join("frames");
    let _ = std::fs::remove_dir_all(&frames_dir);
    std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    let base = match &a.eeprom {
        Some(p) => Some(std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?),
        None => None,
    };
    let scripts: Vec<model2::input::InputScript> = (0..a.nodes)
        .map(|i| {
            let spec = a.inputs[i].as_deref().or(o.input.as_deref()).unwrap_or("");
            model2::input::InputScript::parse(spec, &roms.game.controls).map_err(|e| format!("input {i}: {e}"))
        })
        .collect::<Result<_, _>>()?;
    let mut ms: Vec<model2::Machine> = (0..a.nodes)
        .map(|i| {
            let mut m = machine(roms);
            m.bus.geo.set_threaded(!o.single_thread);
            m.bus.eeprom.load(&patched_eeprom(
                base.as_deref(),
                a.types[i],
                &a.set_bytes.iter().filter(|b| b.0 == i).map(|b| (b.1, b.2)).collect::<Vec<_>>(),
            ));
            m.bus.link = Some(model2::link::Link::new(romset::LinkBoard::Idle).joined());
            if let Some((_, ad)) = a.watch.filter(|w| w.0 == i) {
                m.bus.watch = Some((ad, Vec::new()));
            }
            if a.trace == Some(i) {
                m.bus.link_trace = Some(Vec::new());
            }
            m
        })
        .collect();
    let mut hub = LinkHub::new(a.nodes, a.delay);
    hub.drop_at = a.drop_at;
    hub.freeze = a.freeze;
    hub.layout = a.layout;
    for &(n, d) in &a.delay_to {
        if n < a.nodes {
            hub.delays[n] = d.max(1);
        }
    }
    if let Some(ids) = &a.ids {
        if ids.len() != a.nodes {
            return Err("--ids: one per node".into());
        }
        hub.fixed_ids = Some(ids.clone());
    }
    let mut log = String::new();
    let mut blocks = String::new();
    let say = |log: &mut String, s: String| {
        println!("{s}");
        log.push_str(&s);
        log.push('\n');
    };
    if let Some(p) = &a.load {
        let data = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let raw = png::zlib::decompress(data.strip_prefix(b"LINKRUN-SAVE-1\n").ok_or("not a link-run save")?)
            .map_err(|e| format!("save damaged: {e}"))?;
        let mut at = 0usize;
        let take = |at: &mut usize| -> Result<&[u8], String> {
            let n = u64::from_le_bytes(raw.get(*at..*at + 8).ok_or("save truncated")?.try_into().unwrap()) as usize;
            let s = raw.get(*at + 8..*at + 8 + n).ok_or("save truncated")?;
            *at += 8 + n;
            Ok(s)
        };
        for m in ms.iter_mut() {
            let s = take(&mut at)?;
            m.load_state(s)?;
        }
        let h = take(&mut at)?.to_vec();
        let mut buses: Vec<&mut model2::bus::Model2Bus> = ms.iter_mut().map(|m| &mut m.bus).collect();
        hub = LinkHub::load(&h, &mut buses)?;
        if a.fresh_hub {
            // Only the machines were saved: the board's own state is lost.
            hub = LinkHub::new(a.nodes, a.delay);
            for m in ms.iter_mut() {
                if let Some(l) = m.bus.link.as_mut() {
                    (l.started, l.master) = (false, false);
                }
            }
        }
        hub.drop_at = a.drop_at;
        hub.freeze = a.freeze;
        hub.layout = a.layout;
        for (m, s) in ms.iter_mut().zip(&scripts) {
            s.apply(m.frames, &mut m.bus);
        }
        say(&mut log, format!("state         loaded {} (frame {})", p.display(), ms[0].frames));
    }
    say(
        &mut log,
        format!(
            "link-run      {} {} nodes, delay {}, link types {:?}",
            roms.game.id,
            a.nodes,
            a.delay,
            &a.types[..a.nodes]
        ),
    );
    let mut last_state = vec![u32::MAX; a.nodes];
    let mut last_rx = vec![u32::MAX; a.nodes];
    // The received block that is the node's own echo.
    let own_k = if a.layout == 3 { a.nodes - 1 } else { 0 };
    let mut last_tx: Vec<Vec<u8>> = vec![vec![0; BLOCK]; a.nodes];
    let started = std::time::Instant::now();
    let mut frame = ms[0].frames;
    let mut step = 0u64;
    while frame < a.frames {
        step += 1;
        for (i, m) in ms.iter_mut().enumerate() {
            // A node held in reset for `skew` steps boots that much later than the first.
            if step < a.skew[i] {
                continue;
            }
            let f0 = m.frames;
            while m.frames == f0 {
                m.run_slice(u64::MAX).map_err(|s| format!("node {i} stopped: {s:?}"))?;
            }
        }
        frame = ms[0].frames;
        let _ = step;
        let mut buses: Vec<&mut model2::bus::Model2Bus> = ms.iter_mut().map(|m| &mut m.bus).collect();
        for l in hub.exchange(frame, &mut buses) {
            say(&mut log, format!("frame {frame:>6}: {l}"));
        }
        let mut line = String::new();
        for (i, m) in ms.iter().enumerate() {
            let Some(ram) = m.bus.link_ram() else { continue };
            let (ready, id, total) = (ram[0], ram[2], ram[3]);
            let tx = &ram[TX..TX + BLOCK];
            let st = u32at(tx, 0x18);
            if st != last_state[i] {
                say(
                    &mut log,
                    format!(
                        "frame {frame:>6}: node {i} state {} -> {st} (ready {ready} id {id} total {total}) own: {}",
                        last_state[i] as i32,
                        block_summary(tx)
                    ),
                );
                last_state[i] = st;
            }
            // Every change of a word of the node's block outside the car's moving
            // numbers (the position and wheel floats, the race clock).
            for w in (0..BLOCK).step_by(4) {
                if (0x24..0x80).contains(&w) || w == 0x19c || w == 0x18 {
                    continue;
                }
                if tx[w..w + 4] != last_tx[i][w..w + 4] {
                    say(
                        &mut log,
                        format!(
                            "frame {frame:>6}: node {i} block word {w:#05x}: {:#x} -> {:#x}",
                            u32at(&last_tx[i], w),
                            u32at(tx, w)
                        ),
                    );
                }
            }
            last_tx[i].copy_from_slice(tx);
            if a.nodes > 1 {
                let other = if own_k == 0 { 1 } else { 0 };
                let rx1 = &ram[RX + other * BLOCK..RX + (other + 1) * BLOCK];
                let s1 = u32at(rx1, 0x18);
                if s1 != last_rx[i] {
                    say(
                        &mut log,
                        format!("frame {frame:>6}: node {i} sees the other at state {s1}: {}", block_summary(rx1)),
                    );
                    last_rx[i] = s1;
                }
            }
            if hub.is_ready() && (a.log_every == 0 || frame.is_multiple_of(a.log_every)) {
                let mut s = format!(" | n{i} {}", block_summary(tx));
                for k in (0..a.nodes).filter(|&k| k != own_k) {
                    s.push_str(&format!(" ; rx{k} {}", block_summary(&ram[RX + k * BLOCK..RX + (k + 1) * BLOCK])));
                }
                line.push_str(&s);
            }
        }
        if !line.is_empty() {
            blocks.push_str(&format!("f{frame}{line}\n"));
        }
        if a.dump_at.contains(&frame) {
            for (i, m) in ms.iter().enumerate() {
                if let Some(ram) = m.bus.link_ram() {
                    let mut t = String::new();
                    for (k, blk) in ram[TX..RX + 4 * BLOCK].chunks(BLOCK).enumerate() {
                        t.push_str(&format!(
                            "block {}\n",
                            if k == 0 { "tx".to_string() } else { format!("rx{}", k - 1) }
                        ));
                        for (n, w) in blk.chunks(16).enumerate() {
                            t.push_str(&format!(
                                "{:03x}: {}\n",
                                n * 16,
                                w.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
                            ));
                        }
                    }
                    let _ = std::fs::write(dir.join(format!("blocks-{frame}-n{i}.txt")), t);
                }
            }
        }
        if let Some((at, path)) = a.save.as_ref().filter(|s| s.0 == frame) {
            let mut raw = Vec::new();
            for m in ms.iter_mut() {
                let s = m.save_state();
                raw.extend((s.len() as u64).to_le_bytes());
                raw.extend(s);
            }
            let buses: Vec<&mut model2::bus::Model2Bus> = ms.iter_mut().map(|m| &mut m.bus).collect();
            let h = hub.save(&buses);
            raw.extend((h.len() as u64).to_le_bytes());
            raw.extend(h);
            let mut out = b"LINKRUN-SAVE-1\n".to_vec();
            out.extend(png::zlib::compress(&raw));
            std::fs::write(path, &out).map_err(|e| format!("{}: {e}", path.display()))?;
            say(&mut log, format!("state         saved {} at frame {at} ({} KB)", path.display(), out.len() / 1024));
        }
        for (m, s) in ms.iter_mut().zip(&scripts) {
            s.apply(m.frames, &mut m.bus);
        }
        if (a.shot_every > 0 && frame.is_multiple_of(a.shot_every)) || a.shot_at.contains(&frame) {
            shot(&mut ms, &frames_dir.join(format!("{frame:06}.png")));
        }
    }
    shot(&mut ms, &frames_dir.join("final.png"));
    if let Some((n, ad, f)) = &a.ram_dump {
        if let Some(mem) = ms.get(*n).and_then(|m| m.bus.memory_at(*ad)) {
            let _ = std::fs::write(f, mem);
            say(&mut log, format!("ram dump      node {n} {ad:#x}: {} bytes to {}", mem.len(), f.display()));
        }
    }
    if let Some((n, ad)) = a.watch {
        if let Some((_, hits)) = ms.get(n).and_then(|m| m.bus.watch.as_ref()) {
            for &(t, ip, addr, v, sz) in hits.iter().take(400) {
                say(
                    &mut log,
                    format!(
                        "watch node {n} {ad:#x}: frame {:>6} ip {ip:#x} {addr:#010x} <- {v:#x} ({sz} bytes)",
                        t / model2::bus::FRAME_CYCLES
                    ),
                );
            }
        }
    }
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (i, m) in ms.iter_mut().enumerate() {
        m.bus.geo.sync();
        let sh = m.bus.state_hash();
        say(&mut log, format!("node {i}        state hash {sh:016x} ({} frames, ip {:#x})", m.frames, m.cpu.ip));
        h = fold(h, &sh.to_le_bytes());
        let _ = std::fs::write(dir.join(format!("eeprom-{i}.bin")), m.bus.eeprom.save());
        if let Some(t) = &m.bus.link_trace {
            let text: String = t
                .iter()
                .map(|x| {
                    format!(
                        "{:>6} ip {:08x} {} {:08x} {} {:0w$x}\n",
                        x.cycle / model2::bus::FRAME_CYCLES,
                        x.ip,
                        if x.write { "W" } else { "R" },
                        x.addr,
                        x.size,
                        x.value,
                        w = x.size as usize * 2
                    )
                })
                .collect();
            let _ = std::fs::write(dir.join(format!("link-trace-{i}.txt")), text);
        }
    }
    let buses: Vec<&mut model2::bus::Model2Bus> = ms.iter_mut().map(|m| &mut m.bus).collect();
    h = fold(h, &hub.save(&buses));
    say(&mut log, format!("combined hash {h:016x}"));
    say(
        &mut log,
        format!("host time     {:.1}s for {frame} frames of {} nodes", started.elapsed().as_secs_f64(), a.nodes),
    );
    let _ = std::fs::write(dir.join("link-run.log"), log);
    let _ = std::fs::write(dir.join("blocks.txt"), blocks);
    Ok(())
}

/// Every node's screen, side by side, to a PNG.
fn shot(ms: &mut [model2::Machine], path: &Path) {
    let mut views = Vec::new();
    for m in ms.iter_mut() {
        m.bus.geo.sync();
        views.push(render(m, None, path));
    }
    let (w, h) = (views[0].0.width, views[0].0.height);
    let gap = 4usize;
    let tw = w * views.len() + gap * (views.len() - 1);
    let mut rgb = vec![40u8; tw * h * 3];
    for (n, (_, px)) in views.iter().enumerate() {
        for y in 0..h {
            let dst = (y * tw + n * (w + gap)) * 3;
            rgb[dst..dst + w * 3].copy_from_slice(&px[y * w * 3..(y + 1) * w * 3]);
        }
    }
    let _ = std::fs::write(path, png::encode_rgb(tw, h, &rgb));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_record_checks_out() {
        // The CRC of a record the game itself wrote (link type 0), docs/hw/link-board.md section 2.
        let mut e = patched_eeprom(None, 0, &[]);
        assert_eq!((e[0], e[1]), (0x71, 0xad));
        e = patched_eeprom(Some(&e), 1, &[]);
        assert_eq!(e[0x0b], 1);
        assert_eq!(record_crc(&e[2..36]), u16::from_le_bytes([e[0], e[1]]));
    }
}
