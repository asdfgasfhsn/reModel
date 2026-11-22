//! Sessions over loopback with synthetic records (no ROMs): the handshake and
//! its refusals, delivery under loss, jitter and duplication, a disconnect,
//! and a damaged record found by the state check.

use linknet::*;
use model2::link::NodeRec;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

fn cfg(name: &str, faults: Faults) -> Config {
    Config {
        game: "srallyc".into(),
        rom_hash: 0x1234,
        version: "t".into(),
        name: name.into(),
        block: 448,
        max_hosts: 4,
        faults,
        timeout: Duration::from_millis(700),
        check_every: 10,
    }
}

fn lo() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Host and joiners through the handshake, polled from this one thread.
fn connect(host_faults: Faults, join_faults: Vec<Faults>, delay: usize) -> Vec<Session> {
    let mut host = Host::bind(cfg("host", host_faults), lo()).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    let mut joins: Vec<Join> = join_faults
        .into_iter()
        .enumerate()
        .map(|(i, f)| Join::connect(cfg(&format!("p{i}"), f), addr).unwrap())
        .collect();
    let n = joins.len();
    let t = Instant::now();
    let mut sessions: Vec<Option<Session>> = (0..=n).map(|_| None).collect();
    let mut started = false;
    let mut accepted = vec![false; n];
    while sessions.iter().any(|s| s.is_none()) {
        assert!(t.elapsed() < Duration::from_secs(10), "the handshake took too long");
        if !started && host.players() == n + 1 {
            host.start(Some(delay), Some(vec![9, 9]), &|_, h| 100 + h as u64).unwrap();
            started = true;
        }
        match host.poll() {
            HostStatus::Ready(s, p) => {
                assert_eq!(p.delay as usize, delay);
                sessions[0] = Some(*s)
            }
            HostStatus::Failed(e) => panic!("host: {e}"),
            _ => {}
        }
        for (i, j) in joins.iter_mut().enumerate() {
            if sessions[i + 1].is_some() {
                continue;
            }
            match j.poll() {
                JoinStatus::Params(p) => {
                    assert_eq!(p.base.as_deref(), Some(&[9u8, 9][..]));
                    j.accept(101 + i as u64);
                    accepted[i] = true;
                }
                JoinStatus::Ready(s, _) => sessions[i + 1] = Some(*s),
                JoinStatus::Failed(e) => panic!("join {i}: {e}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_micros(300));
    }
    assert!(accepted.iter().all(|&a| a));
    sessions.into_iter().map(|s| s.unwrap()).collect()
}

/// What a host received (node, frame, record), or how its session ended.
type Outcome = Result<Vec<(usize, u64, NodeRec)>, End>;

fn rec(node: usize, frame: u64) -> NodeRec {
    // Mostly zero with a few moving words, like a real block.
    let mut tx = vec![0u8; 448];
    tx[0x18] = (frame / 40) as u8;
    tx[0x28..0x2c].copy_from_slice(&(frame as u32 * 31 + node as u32).to_le_bytes());
    tx[0x40 + node] = 0xaa;
    NodeRec {
        started: frame > 3,
        master: node == 0,
        start_frame: if frame > 3 { 3 } else { 0 },
        tx: if frame > 3 { tx } else { vec![] },
        change: None,
    }
}

/// Run `frames` boundaries of every session in lockstep with delay `d` from
/// one thread each; returns what each received, by (node, frame).
fn run(sessions: Vec<Session>, frames: u64, d: u64, stop: Option<(usize, u64)>) -> Vec<Outcome> {
    let handles: Vec<_> = sessions
        .into_iter()
        .enumerate()
        .map(|(h, mut s)| {
            std::thread::spawn(move || {
                let me = s.local_nodes()[0];
                let n = s.params.nodes();
                let mut got: Vec<(usize, u64, NodeRec)> = Vec::new();
                for f in 1..=frames {
                    if stop == Some((h, f)) {
                        return Ok(got);
                    }
                    s.publish(me, f, rec(me, f));
                    loop {
                        for ev in s.pump() {
                            if let Event::Rec { node, frame, rec } = ev {
                                got.push((node, frame, rec));
                            }
                        }
                        if let Some(e) = &s.end {
                            return Err(e.clone());
                        }
                        let need = (f + 1).saturating_sub(d);
                        let have = |node: usize| need == 0 || got.iter().any(|g| g.0 == node && g.1 == need);
                        if (0..n).filter(|&k| k != me).all(have) {
                            break;
                        }
                        std::thread::sleep(Duration::from_micros(200));
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                // Stay until the others hold everything of ours.
                let t = Instant::now();
                while s.acked_by_all() < frames && s.end.is_none() && t.elapsed() < Duration::from_secs(3) {
                    s.pump();
                    std::thread::sleep(Duration::from_micros(300));
                }
                assert!(s.stats.checks_ok > 0 || s.params.check_every == 0, "no checkpoint compared");
                Ok(got)
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

fn check_delivery(r: &[Outcome], frames: u64, d: u64) {
    for (h, res) in r.iter().enumerate() {
        let got = res.as_ref().unwrap_or_else(|e| panic!("host {h} ended: {e}"));
        for (node, f, rec_) in got {
            assert_eq!(*rec_, rec(*node, *f), "host {h}: node {node} frame {f}");
        }
        for node in (0..r.len()).filter(|&n| n != h) {
            for f in 1..=(frames + 1 - d) {
                assert!(got.iter().any(|g| g.0 == node && g.1 == f), "host {h} lacks node {node} frame {f}");
            }
        }
    }
}

#[test]
fn two_hosts_exchange_records_in_order_on_a_clean_loopback() {
    let s = connect(Faults::default(), vec![Faults::default()], 2);
    let r = run(s, 300, 2, None);
    check_delivery(&r, 300, 2);
}

#[test]
fn records_survive_loss_jitter_and_duplication() {
    let f = |seed| Faults { latency_ms: 5.0, jitter_ms: 50.0, loss_pct: 5.0, dup_pct: 5.0, seed, ..Faults::default() };
    let s = connect(f(1), vec![f(2)], 6);
    let r = run(s, 400, 6, None);
    check_delivery(&r, 400, 6);
}

#[test]
fn three_hosts_mesh() {
    let f = |seed| Faults { latency_ms: 2.0, jitter_ms: 10.0, loss_pct: 3.0, dup_pct: 2.0, seed, ..Faults::default() };
    let s = connect(f(1), vec![f(2), f(3)], 3);
    let r = run(s, 200, 3, None);
    check_delivery(&r, 200, 3);
}

/// Pump `s` until `done` is true of what it has raised (or a second passes); the events seen.
fn pump_until(s: &mut Session, secs: f32, done: impl Fn(&[Event]) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let t = Instant::now();
    while t.elapsed().as_secs_f32() < secs && !done(&seen) {
        seen.extend(s.pump());
        std::thread::sleep(Duration::from_micros(300));
    }
    seen
}

#[test]
fn a_host_that_goes_silent_has_left_and_the_session_goes_on() {
    let mut s = connect(Faults::default(), vec![Faults::default()], 2);
    let mut a = s.remove(0);
    let b = s.remove(0);
    drop(b);
    let ev = pump_until(&mut a, 3.0, |e| e.iter().any(|x| matches!(x, Event::Dropped { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::Left { host: 1, lost: true, name } if name == "p0")), "{ev:?}");
    assert!(ev.iter().any(|e| matches!(e, Event::Dropped { host: 1, frame: 0 })), "{ev:?}");
    assert_eq!(a.end, None, "a leaving host does not end the session");
    assert!(a.alone());
}

#[test]
fn a_goodbye_is_a_leave_at_once_with_the_session_going_on() {
    let mut s = connect(Faults::default(), vec![Faults::default()], 2);
    let mut a = s.remove(0);
    let mut b = s.remove(0);
    b.bye("quit");
    let t = Instant::now();
    let ev = pump_until(&mut a, 1.0, |e| e.iter().any(|x| matches!(x, Event::Dropped { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::Left { host: 1, lost: false, name } if name == "p0")), "{ev:?}");
    assert_eq!(a.end, None);
    assert_eq!(a.drop_frame(1), Some(0));
    assert!(t.elapsed() < Duration::from_millis(300));
}

#[test]
fn a_session_knows_the_names_and_uses_them_in_messages() {
    let mut s = connect(Faults::default(), vec![Faults::default(), Faults::default()], 2);
    assert_eq!(s[0].names(), ["host", "p0", "p1"]);
    let mut a = s.remove(0);
    let _b = s.remove(0);
    let mut c = s.remove(0);
    c.bye("quit");
    let ev = pump_until(&mut a, 1.0, |e| e.iter().any(|x| matches!(x, Event::Left { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::Left { host: 2, name, .. } if name == "p1")), "{ev:?}");
    assert!(!a.alone(), "p0 is still here");
}

#[test]
fn a_damaged_record_is_found_by_the_state_check() {
    let f = Faults { corrupt_frame: 40, ..Faults::default() };
    let s = connect(Faults::default(), vec![f], 2);
    let r = run(s, 200, 2, None);
    for (h, x) in r.iter().enumerate() {
        match x {
            Err(End::Desync { frame: 40, .. }) | Err(End::Left { .. }) => {}
            other => panic!("host {h}: {other:?}"),
        }
    }
    assert!(r.iter().any(|x| matches!(x, Err(End::Desync { frame: 40, .. }))));
}

#[test]
fn mismatched_games_roms_and_builds_are_refused_with_a_reason() {
    let host = Host::bind(cfg("host", Faults::default()), lo()).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    let mut host = host;
    for (what, tweak) in [
        ("game", (|c: &mut Config| c.game = "daytona".into()) as fn(&mut Config)),
        ("ROMs", |c| c.rom_hash = 5),
        ("version", |c| c.version = "other".into()),
    ] {
        let mut c = cfg("j", Faults::default());
        tweak(&mut c);
        let mut j = Join::connect(c, addr).unwrap();
        let t = Instant::now();
        let msg = loop {
            host.poll();
            match j.poll() {
                JoinStatus::Failed(m) => break m,
                _ => assert!(t.elapsed() < Duration::from_secs(3), "{what}: no answer"),
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(msg.contains("refused"), "{what}: {msg}");
        eprintln!("{what}: {msg}");
    }
    assert_eq!(host.players(), 1);
}

#[test]
fn settings_that_differ_stop_the_start_for_everyone() {
    let mut host = Host::bind(cfg("host", Faults::default()), lo()).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    let mut j = Join::connect(cfg("j", Faults::default()), addr).unwrap();
    let t = Instant::now();
    let mut started = false;
    let (mut host_msg, mut join_msg) = (None, None);
    while (host_msg.is_none() || join_msg.is_none()) && t.elapsed() < Duration::from_secs(5) {
        if !started && host.players() == 2 {
            host.start(Some(2), None, &|_, h| h as u64).unwrap();
            started = true;
        }
        if let HostStatus::Failed(m) = host.poll() {
            host_msg = Some(m);
        }
        match j.poll() {
            JoinStatus::Params(_) => j.accept(999),
            JoinStatus::Failed(m) => join_msg = Some(m),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let (h, j) = (host_msg.expect("host refuses"), join_msg.expect("joiner told"));
    assert!(h.contains("settings differ"), "{h}");
    assert!(j.contains("aborted"), "{j}");
}

#[test]
fn a_host_in_its_lobby_answers_discovery() {
    let mut host = Host::bind(cfg("the host", Faults::default()), lo()).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    let mut d = Discovery::to(addr).unwrap();
    let t = Instant::now();
    while d.poll().is_empty() && t.elapsed() < Duration::from_secs(2) {
        host.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
    let f = d.found.first().expect("found the host");
    assert_eq!((f.name.as_str(), f.game.as_str(), f.players, f.max, f.addr), ("the host", "srallyc", 1, 4, addr));
}
