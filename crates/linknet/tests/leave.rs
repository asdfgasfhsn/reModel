//! Players leaving a running session (no ROMs, synthetic records): the survivors agree the
//! leaver's drop frame, get any records of it they lack from one another, carry on to the
//! end on the records the hub would synthesise, and keep passing the state checks. The
//! argument is in docs/hw/link-board.md section 12.3.

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

fn connect(faults: Vec<Faults>, delay: usize) -> Vec<Session> {
    let lo: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut it = faults.into_iter();
    let mut host = Host::bind(cfg("h0", it.next().unwrap()), lo).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    let mut joins: Vec<Join> =
        it.enumerate().map(|(i, f)| Join::connect(cfg(&format!("h{}", i + 1), f), addr).unwrap()).collect();
    let n = joins.len();
    let t = Instant::now();
    let mut sessions: Vec<Option<Session>> = (0..=n).map(|_| None).collect();
    let mut started = false;
    while sessions.iter().any(|s| s.is_none()) {
        assert!(t.elapsed() < Duration::from_secs(10), "the handshake took too long");
        if !started && host.players() == n + 1 {
            host.start(Some(delay), Some(vec![9, 9]), &|_, h| 100 + h as u64).unwrap();
            started = true;
        }
        match host.poll() {
            HostStatus::Ready(s, _) => sessions[0] = Some(*s),
            HostStatus::Failed(e) => panic!("host: {e}"),
            _ => {}
        }
        for (i, j) in joins.iter_mut().enumerate() {
            if sessions[i + 1].is_some() {
                continue;
            }
            match j.poll() {
                JoinStatus::Params(_) => j.accept(101 + i as u64),
                JoinStatus::Ready(s, _) => sessions[i + 1] = Some(*s),
                JoinStatus::Failed(e) => panic!("join {i}: {e}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_micros(300));
    }
    sessions.into_iter().map(|s| s.unwrap()).collect()
}

fn rec(node: usize, frame: u64) -> NodeRec {
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

#[derive(Clone, Copy, PartialEq)]
enum Exit {
    /// Say goodbye, then go.
    Quit,
    /// Go with no word.
    Vanish,
}

#[derive(Debug, Default)]
struct Out {
    end: Option<End>,
    /// Frames this host ran.
    ran: u64,
    /// (host, drop frame) as agreed.
    dropped: Vec<(usize, u64)>,
    /// Records consumed: (node, frame the view needed, the record given).
    used: Vec<(usize, u64, NodeRec)>,
    checks_at_drop: u64,
    checks_end: u64,
    relayed_rx: u64,
    relayed_tx: u64,
    alone: bool,
    left: Vec<usize>,
}

/// Run every host's frames in lockstep with delay `d` on a thread each. `exits[h]` is the frame
/// host `h` leaves at (before publishing it) and how. A survivor consumes, for each other node at
/// frame `f`, the record of frame `f + 1 - d`, or of the drop frame if that is earlier (what
/// `LinkHub` does).
fn run(sessions: Vec<Session>, frames: u64, d: u64, exits: Vec<Option<(u64, Exit)>>) -> Vec<Out> {
    let handles: Vec<_> = sessions
        .into_iter()
        .enumerate()
        .map(|(h, mut s)| {
            let exit = exits[h];
            std::thread::spawn(move || {
                let me = s.local_nodes()[0];
                let n = s.params.nodes();
                let mut o = Out::default();
                let mut got: std::collections::BTreeMap<(usize, u64), NodeRec> = Default::default();
                for f in 1..=frames {
                    match exit {
                        Some((at, Exit::Quit)) if at == f => {
                            s.bye("left");
                            return o;
                        }
                        Some((at, Exit::Vanish)) if at == f => return o,
                        _ => {}
                    }
                    s.publish(me, f, rec(me, f));
                    let wait = Instant::now();
                    loop {
                        for ev in s.pump() {
                            match ev {
                                Event::Rec { node, frame, rec } => {
                                    got.insert((node, frame), rec);
                                }
                                Event::Left { host, .. } => o.left.push(host),
                                Event::Dropped { host, frame } => {
                                    o.dropped.push((host, frame));
                                    o.checks_at_drop = s.stats.checks_ok;
                                }
                            }
                        }
                        if let Some(e) = &s.end {
                            o.end = Some(e.clone());
                            return o;
                        }
                        let need = (f + 1).saturating_sub(d);
                        let want = |node: usize| match s.drop_frame(node) {
                            Some(df) => need.min(df),
                            None => need,
                        };
                        if (0..n).filter(|&k| k != me).all(|k| want(k) == 0 || got.contains_key(&(k, want(k)))) {
                            for k in (0..n).filter(|&k| k != me) {
                                if want(k) != 0 {
                                    o.used.push((k, need, got[&(k, want(k))].clone()));
                                }
                            }
                            break;
                        }
                        assert!(wait.elapsed() < Duration::from_secs(15), "host {h} stuck at frame {f}");
                        std::thread::sleep(Duration::from_micros(200));
                    }
                    o.ran = f;
                    std::thread::sleep(Duration::from_micros(500));
                }
                let t = Instant::now();
                while s.acked_by_all() < frames && s.end.is_none() && t.elapsed() < Duration::from_secs(3) {
                    s.pump();
                    std::thread::sleep(Duration::from_micros(300));
                }
                o.checks_end = s.stats.checks_ok;
                o.relayed_rx = s.stats.relayed_rx;
                o.relayed_tx = s.stats.relayed_tx;
                o.alone = s.alone();
                o
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

/// What every survivor must show: no end, every frame, the same drop frame of each leaver,
/// each consumed record the real one up to the drop frame and the repeat of it after, and state
/// checks that kept passing after the drop. Returns the agreed drop frames.
fn survivors_agree(outs: &[Out], survivors: &[usize], leavers: &[usize], frames: u64, d: u64) -> Vec<(usize, u64)> {
    let mut agreed: Vec<(usize, u64)> = Vec::new();
    for &s in survivors {
        let o = &outs[s];
        assert_eq!(o.end, None, "host {s} ended");
        assert_eq!(o.ran, frames, "host {s} stopped early");
        let mut got = o.dropped.clone();
        got.sort();
        if agreed.is_empty() {
            agreed = got.clone();
        }
        assert_eq!(got, agreed, "host {s} agrees another drop frame");
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), leavers, "host {s}: who dropped");
        assert!(
            o.alone || o.checks_end > o.checks_at_drop,
            "host {s}: no state check passed after the drop ({} at the drop, {} at the end)",
            o.checks_at_drop,
            o.checks_end
        );
    }
    for &s in survivors {
        for (node, need, r) in &outs[s].used {
            let want = match agreed.iter().find(|a| a.0 == *node) {
                Some((_, f)) => (*need).min(*f),
                None => *need,
            };
            assert_eq!(*r, rec(*node, want), "host {s}: node {node} needed {need}");
        }
    }
    let _ = d;
    agreed
}

fn lossy(seed: u64) -> Faults {
    Faults { latency_ms: 2.0, jitter_ms: 10.0, loss_pct: 5.0, dup_pct: 3.0, seed, ..Faults::default() }
}

#[test]
fn a_two_player_game_goes_on_alone_after_a_polite_leave() {
    let s = connect(vec![Faults::default(); 2], 3);
    let outs = run(s, 300, 3, vec![None, Some((100, Exit::Quit))]);
    let agreed = survivors_agree(&outs, &[0], &[1], 300, 3);
    assert!(agreed[0].1 <= 99 && agreed[0].1 >= 90, "{agreed:?}");
    assert!(outs[0].alone);
}

#[test]
fn a_two_player_game_goes_on_alone_after_a_vanish() {
    let s = connect(vec![Faults::default(); 2], 3);
    let outs = run(s, 400, 3, vec![Some((120, Exit::Vanish)), None]);
    let agreed = survivors_agree(&outs, &[1], &[0], 400, 3);
    assert!(agreed[0].1 <= 119, "{agreed:?}");
}

#[test]
fn the_host_leaving_is_no_different() {
    let s = connect(vec![Faults::default(); 3], 4);
    let outs = run(s, 300, 4, vec![Some((90, Exit::Quit)), None, None]);
    survivors_agree(&outs, &[1, 2], &[0], 300, 4);
}

#[test]
fn three_players_one_vanishes_under_loss_and_jitter() {
    let s = connect(vec![lossy(1), lossy(2), lossy(3)], 5);
    let outs = run(s, 400, 5, vec![None, Some((150, Exit::Vanish)), None]);
    survivors_agree(&outs, &[0, 2], &[1], 400, 5);
}

#[test]
fn survivors_that_hold_different_counts_of_the_leavers_records_get_the_rest_relayed() {
    // Host 2 stops sending its records to host 1 from frame 80, then quits at 90: host 0 has
    // them to 89, host 1 only to 79. Host 1 can only go on if host 0 hands over 80 to 89.
    let d = 8;
    let faults = vec![Faults::default(), Faults::default(), Faults { starve: Some((1, 80)), ..Faults::default() }];
    let s = connect(faults, d as usize);
    let outs = run(s, 300, d, vec![None, None, Some((90, Exit::Quit))]);
    let agreed = survivors_agree(&outs, &[0, 1], &[2], 300, d);
    assert!(agreed[0].1 >= 85, "the larger report wins: {agreed:?}");
    assert!(outs[1].relayed_rx >= 5, "host 1 had records relayed: {}", outs[1].relayed_rx);
    assert!(outs[0].relayed_tx >= 5);
}

#[test]
fn the_same_with_a_vanish_and_loss() {
    let d = 8;
    let faults = vec![lossy(4), lossy(5), Faults { starve: Some((0, 70)), ..lossy(6) }];
    let s = connect(faults, d as usize);
    let outs = run(s, 300, d, vec![None, None, Some((85, Exit::Vanish))]);
    survivors_agree(&outs, &[0, 1], &[2], 300, d);
    assert!(outs[0].relayed_rx >= 3, "host 0 had records relayed: {}", outs[0].relayed_rx);
}

#[test]
fn two_leave_in_turn_and_the_last_plays_on() {
    let s = connect(vec![Faults::default(); 3], 3);
    let outs = run(s, 400, 3, vec![None, Some((100, Exit::Quit)), Some((200, Exit::Vanish))]);
    survivors_agree(&outs, &[0], &[1, 2], 400, 3);
    assert!(outs[0].alone);
}

#[test]
fn a_real_mismatch_still_ends_the_session_after_a_leave() {
    // Host 1 sees a damaged record of host 0's at frame 200, well after host 2 left.
    let faults = vec![Faults::default(), Faults { corrupt_frame: 200, ..Faults::default() }, Faults::default()];
    let s = connect(faults, 3);
    let outs = run(s, 400, 3, vec![None, None, Some((60, Exit::Quit))]);
    assert!(
        outs.iter().take(2).any(|o| matches!(o.end, Some(End::Desync { frame: 200, .. }))),
        "{:?} {:?}",
        outs[0].end,
        outs[1].end
    );
}
