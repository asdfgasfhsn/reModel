//! `link-net`: one machine of a linked game on this host, the others on
//! other hosts, the link board's blocks carried between them over UDP
//! (`crates/linknet`, docs/hw/link-board.md section 12). Headless, scripted
//! inputs, for testing two processes on one machine over loopback (with
//! fault injection) and for trying a real network. The machine is the same
//! one `link-run` builds for the node, so the same inputs and delay give the
//! same state hash.

use super::link_run::{node_machine, shot};
use super::*;
use linknet::{Config, Faults, Host, HostStatus, Join, JoinStatus, NetLink, Step};
use model2::link::LinkHub;
use romset::SettingsStore;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

const USAGE_NET: &str = "usage: remodel --game G link-net (--host [PORT] | --join HOST:PORT) [--nodes N] [--frames F] [--input S]
        [--delay D] [--name NAME] [--bind IP] [--timeout S] [--check-every F] [--lobby-wait S] [--eeprom FILE]
        [--shot-every K] [--shot-at F,F,..] [--leave-at F] [--hash-every K] [--realtime]
        [--latency MS] [--jitter MS] [--loss PCT] [--dup PCT] [--seed N] [--corrupt-rx-at F] [--tamper-byte OFF:V]
  --host [PORT]   host the session (default port 47890) and wait for --nodes N - 1 players (default 2); this host runs node 1 (CAR 1, the master)
  --join H:P      join the host at H:P; the host assigns the node
  --delay D       host: the link delay D in frames (default: from the measured round trip). D = 1 means no slack
  --frames F      end after this many frames (every host should give the same)
  --input S       scripted inputs as `run`'s --input, for this host's machine
  --timeout S     silence that ends the session (default 5)
  --check-every F frames between state checks (default 60, 0 off)
  --eeprom FILE   host: the settings image all nodes start from (default: the game's defaults)
  --realtime      hold the machine to 60 frames a second (the stalls then are what a player would see)
  --leave-at F    say goodbye and stop at frame F (a test)
  fault injection on what this host sends (tests): --latency/--jitter MS added one way, --loss/--dup PCT, --seed
  --corrupt-rx-at F   damage the record received for frame F (a test of the state check)
  --tamper-byte OFF:V set byte OFF of this machine's settings image after agreeing them (a test: the host refuses)
";

struct Args {
    host: Option<u16>,
    join: Option<String>,
    nodes: usize,
    frames: u64,
    delay: Option<usize>,
    name: String,
    bind: String,
    timeout: f32,
    check_every: u16,
    lobby_wait: f32,
    eeprom: Option<PathBuf>,
    shot_every: u64,
    shot_at: Vec<u64>,
    leave_at: u64,
    hash_every: u64,
    ram_trace: bool,
    realtime: bool,
    faults: Faults,
    tamper: Option<(usize, u8)>,
}

fn parse(o: &Opts) -> Result<Args, String> {
    let mut a = Args {
        host: None,
        join: None,
        nodes: 2,
        frames: 3000,
        delay: None,
        name: String::new(),
        bind: "0.0.0.0".into(),
        timeout: 5.0,
        check_every: 60,
        lobby_wait: 120.0,
        eeprom: None,
        shot_every: 0,
        shot_at: vec![],
        leave_at: 0,
        hash_every: 0,
        ram_trace: false,
        realtime: false,
        faults: Faults::default(),
        tamper: None,
    };
    let args: Vec<&String> = o.args.iter().skip(1).collect();
    let mut i = 0;
    let next = |i: &mut usize, f: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).map(|s| s.to_string()).ok_or_else(|| format!("{f} wants a value"))
    };
    while i < args.len() {
        let f = args[i].as_str();
        let num = |s: String| s.parse::<u64>().map_err(|_| format!("{f} wants a number"));
        let flt = |s: String| s.parse::<f32>().map_err(|_| format!("{f} wants a number"));
        match f {
            "--host" => {
                let port = match args.get(i + 1).and_then(|s| s.trim_start_matches(':').parse::<u16>().ok()) {
                    Some(p) => {
                        i += 1;
                        p
                    }
                    None => linknet::DEFAULT_PORT,
                };
                a.host = Some(port);
            }
            "--join" => a.join = Some(next(&mut i, f)?),
            "--nodes" => a.nodes = num(next(&mut i, f)?)? as usize,
            "--frames" => a.frames = num(next(&mut i, f)?)?,
            "--delay" => a.delay = Some(num(next(&mut i, f)?)? as usize),
            "--name" => a.name = next(&mut i, f)?,
            "--bind" => a.bind = next(&mut i, f)?,
            "--timeout" => a.timeout = flt(next(&mut i, f)?)?,
            "--check-every" => a.check_every = num(next(&mut i, f)?)? as u16,
            "--lobby-wait" => a.lobby_wait = flt(next(&mut i, f)?)?,
            "--eeprom" => a.eeprom = Some(PathBuf::from(next(&mut i, f)?)),
            "--shot-every" => a.shot_every = num(next(&mut i, f)?)?,
            "--shot-at" => {
                a.shot_at = next(&mut i, f)?.split(',').filter_map(|x| x.parse().ok()).collect();
            }
            "--leave-at" => a.leave_at = num(next(&mut i, f)?)?,
            "--hash-every" => a.hash_every = num(next(&mut i, f)?)?,
            "--ram-trace" => a.ram_trace = true,
            "--realtime" => a.realtime = true,
            "--latency" => a.faults.latency_ms = flt(next(&mut i, f)?)?,
            "--jitter" => a.faults.jitter_ms = flt(next(&mut i, f)?)?,
            "--loss" => a.faults.loss_pct = flt(next(&mut i, f)?)?,
            "--dup" => a.faults.dup_pct = flt(next(&mut i, f)?)?,
            "--seed" => a.faults.seed = num(next(&mut i, f)?)?,
            "--corrupt-rx-at" => a.faults.corrupt_frame = num(next(&mut i, f)?)?,
            "--tamper-byte" => {
                let v = next(&mut i, f)?;
                let p: Vec<u32> = v.split(':').filter_map(parse_num).collect();
                let [off, val] = p[..] else { return Err("--tamper-byte wants OFFSET:VALUE".into()) };
                a.tamper = Some((off as usize, val as u8));
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    if a.host.is_some() == a.join.is_some() {
        return Err("give --host or --join".into());
    }
    if !(2..=8).contains(&a.nodes) {
        return Err("--nodes: 2 to 8".into());
    }
    Ok(a)
}

pub fn run(o: &Opts, roms: &RomSet) -> ExitCode {
    let a = match parse(o) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE_NET}");
            return ExitCode::FAILURE;
        }
    };
    match go(o, roms, &a) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn go(o: &Opts, roms: &RomSet, a: &Args) -> Result<ExitCode, String> {
    let play = romset::games::link_play(roms.game.id)
        .ok_or_else(|| format!("{}: no link play data (crates/romset/src/games/link.rs)", roms.game.id))?;
    if a.nodes > play.max_nodes {
        return Err(format!("{} links at most {} nodes", roms.game.id, play.max_nodes));
    }
    let dir = OUT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("out").join("link-net"));
    let frames_dir = dir.join("frames");
    std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    let cfg = Config {
        game: roms.game.id.to_string(),
        rom_hash: linknet::rom_hash(&roms.program),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: if a.name.is_empty() { format!("remodel-{}", std::process::id()) } else { a.name.clone() },
        block: play.block as u16,
        max_hosts: play.max_nodes,
        faults: a.faults.clone(),
        timeout: Duration::from_secs_f32(a.timeout),
        check_every: a.check_every,
    };
    let base_file = match (&a.eeprom, play.settings.store) {
        (Some(p), SettingsStore::Eeprom) => Some(std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?),
        _ => None,
    };
    let hash_for = |p: &linknet::Params, h: usize| {
        linknet::settings_hash(play, p.base.as_deref(), p.hosts[h].nodes[0] as usize, &[])
    };

    // Setup: the lobby, the agreement, the start.
    let (sess, params) = if let Some(port) = a.host {
        let ip: std::net::IpAddr = a.bind.parse().map_err(|_| format!("--bind {}: not an address", a.bind))?;
        let mut host =
            Host::bind(cfg, SocketAddr::new(ip, port)).map_err(|e| format!("cannot listen on {ip}:{port}: {e}"))?;
        let lan = linknet::lan_address().map(|i| i.to_string()).unwrap_or_else(|| "this machine's address".into());
        println!(
            "link-net      hosting {} on {} (join with --join {lan}:{})",
            roms.game.id,
            host.local_addr(),
            host.local_addr().port()
        );
        let t = Instant::now();
        let mut full_at: Option<Instant> = None;
        let mut started = false;
        loop {
            if !started && host.players() >= a.nodes {
                // Let the round trips settle for a moment.
                let at = *full_at.get_or_insert_with(Instant::now);
                if at.elapsed() > Duration::from_millis(1200) || a.delay.is_some() {
                    for (n, r) in host.joiners() {
                        println!(
                            "link-net      player {n}: round trip {} ms",
                            r.map_or("?".into(), |r| format!("{r:.2}"))
                        );
                    }
                    host.start(a.delay, base_file.clone(), &hash_for)?;
                    started = true;
                }
            }
            match host.poll() {
                HostStatus::Ready(s, p) => break (*s, p),
                HostStatus::Failed(e) => return Err(e),
                _ => {}
            }
            if !started && t.elapsed().as_secs_f32() > a.lobby_wait {
                return Err(format!("only {} of {} players joined in {} s", host.players(), a.nodes, a.lobby_wait));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    } else {
        let target = a.join.as_deref().unwrap();
        let addr = target
            .to_socket_addrs()
            .map_err(|e| format!("--join {target}: {e}"))?
            .next()
            .ok_or_else(|| format!("--join {target}: no such address"))?;
        let mut join = Join::connect(cfg, addr).map_err(|e| e.to_string())?;
        loop {
            match join.poll() {
                JoinStatus::Params(p) => {
                    // This host's settings from the master's, as they will be run (with any test tampering), and their hash.
                    let node = p.hosts[join.index()].nodes[0] as usize;
                    let extra: Vec<(usize, u8)> = a.tamper.into_iter().collect();
                    join.accept(linknet::settings_hash(play, p.base.as_deref(), node, &extra));
                }
                JoinStatus::Ready(s, p) => break (*s, p),
                JoinStatus::Failed(e) => return Err(e),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    run_session(o, roms, a, play, sess, params, &frames_dir)
}

fn run_session(
    o: &Opts,
    roms: &RomSet,
    a: &Args,
    play: &'static romset::LinkPlay,
    sess: linknet::Session,
    params: linknet::Params,
    frames_dir: &Path,
) -> Result<ExitCode, String> {
    let me = sess.me;
    let node = params.hosts[me].nodes[0] as usize;
    let nodes = params.nodes();
    let delay = params.delay as usize;
    let role = *play.roles.get(node).or(play.roles.last()).unwrap();
    let extra: Vec<(usize, u8)> = a.tamper.into_iter().collect();
    let mut m = node_machine(roms, o, play, node, role, params.base.as_deref(), &extra);
    let script = model2::input::InputScript::parse(o.input.as_deref().unwrap_or(""), &roms.game.controls)
        .map_err(|e| format!("input: {e}"))?;
    let hub = LinkHub::for_game(nodes, delay, play);
    let mut net = NetLink::new(hub, sess);
    let names: Vec<String> = params.hosts.iter().map(|h| h.name.clone()).collect();
    println!(
        "link-net      {} node {node} of {nodes} (player {}), delay D = {delay}, players {names:?}",
        roms.game.id,
        me + 1
    );
    let started = Instant::now();
    let mut next_frame = Instant::now();
    let mut trace =
        a.ram_trace.then(|| std::io::BufWriter::new(std::fs::File::create(frames_dir.join("ram-trace.txt")).unwrap()));
    let mut end: Option<linknet::End> = None;
    let mut left = false;
    'frames: while m.frames < a.frames {
        let f0 = m.frames;
        while m.frames == f0 {
            m.run_slice(u64::MAX).map_err(|s| format!("machine stopped: {s:?}"))?;
        }
        let frame = m.frames;
        loop {
            match net.boundary(frame, &mut [&mut m.bus]) {
                Step::Done(log) => {
                    for l in log {
                        println!("frame {frame:>6}: {l}");
                    }
                    break;
                }
                Step::Stalled => std::thread::sleep(Duration::from_micros(100)),
                Step::End(e) => {
                    end = Some(e);
                    break 'frames;
                }
            }
        }
        if let Some(t) = trace.as_mut() {
            use std::io::Write;
            let h = m.bus.link_ram().map_or(0, |r| linknet::fnv(linknet::FNV_START, r));
            let _ = writeln!(t, "{frame} {h:016x} {:#x}", m.cpu.ip);
        }
        script.apply(m.frames, &mut m.bus);
        if a.realtime {
            next_frame += Duration::from_nanos(16_666_667);
            match next_frame.checked_duration_since(Instant::now()) {
                Some(d) => std::thread::sleep(d),
                // Behind (a hold): the frames lost are not made up in a sprint.
                None => next_frame = Instant::now(),
            }
        }
        if a.hash_every > 0 && frame.is_multiple_of(a.hash_every) {
            m.bus.geo.sync();
            println!("frame {frame:>6}: state hash {:016x}", m.bus.state_hash());
        }
        if (a.shot_every > 0 && frame.is_multiple_of(a.shot_every)) || a.shot_at.contains(&frame) {
            shot(std::slice::from_mut(&mut m), &frames_dir.join(format!("{frame:06}.png")));
        }
        if a.leave_at != 0 && frame >= a.leave_at {
            net.sess.bye("left");
            left = true;
            break;
        }
    }
    if end.is_none() && !left {
        net.linger(a.frames, Duration::from_secs(3));
    }
    shot(std::slice::from_mut(&mut m), &frames_dir.join("final.png"));
    m.bus.geo.sync();
    println!("node {node}        state hash {:016x} ({} frames, ip {:#x})", m.bus.state_hash(), m.frames, m.cpu.ip);
    let (st, ss) = (&net.stalls, &net.sess.stats);
    let secs = started.elapsed().as_secs_f64();
    let rtt = net.sess.worst_rtt().map_or("?".to_string(), |r| format!("{r:.2} ms"));
    println!(
        "link-net      stalls {} of {} boundaries ({:.1} ms in all, longest {:.1} ms), round trip {rtt}",
        st.stalls,
        st.boundaries,
        st.stall_time.as_secs_f64() * 1000.0,
        st.longest.as_secs_f64() * 1000.0
    );
    println!(
        "link-net      sent {} packets {:.1} KB ({:.0} bytes a frame, {:.1} KB/s at 60 frames/s), received {} packets {:.1} KB ({:.0} bytes a frame, {:.1} KB/s at 60 frames/s), duplicate records {}, malformed packets {}, dropped by fault injection {}, state checks passed {}",
        ss.pkts_tx,
        ss.bytes_tx as f64 / 1024.0,
        ss.bytes_tx as f64 / m.frames.max(1) as f64,
        ss.bytes_tx as f64 / m.frames.max(1) as f64 * 60.0 / 1024.0,
        ss.pkts_rx,
        ss.bytes_rx as f64 / 1024.0,
        ss.bytes_rx as f64 / m.frames.max(1) as f64,
        ss.bytes_rx as f64 / m.frames.max(1) as f64 * 60.0 / 1024.0,
        ss.dup_recs,
        ss.bad,
        ss.lost_by_fault,
        ss.checks_ok
    );
    println!("host time     {secs:.1}s for {} frames", m.frames);
    match end {
        Some(linknet::End::Left { host, reason }) if reason == "finished" => {
            println!("link-net      player {} finished (at frame {})", host + 1, m.frames);
            Ok(ExitCode::SUCCESS)
        }
        Some(e) => {
            eprintln!("link-net: {e} (at frame {})", m.frames);
            Ok(ExitCode::from(2))
        }
        None => Ok(ExitCode::SUCCESS),
    }
}
