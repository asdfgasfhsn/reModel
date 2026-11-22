//! A change of the link delay D mid-session (no ROMs, synthetic records): the session host
//! announces `D becomes D' from frame F` in one of its records, every host switches at F and
//! feeds its hub the same view of the others, whatever the loss, and when a player leaves with
//! a change in flight. The rule is in docs/hw/link-board.md section 12.7.

use linknet::*;
use model2::link::{LinkHub, NodeRec};
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
                JoinStatus::Params(_) => j.accept(100 + j.index() as u64),
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
    Quit,
    Vanish,
}

#[derive(Default)]
struct Out {
    end: Option<End>,
    ran: u64,
    /// What the hub fed this host's machine at each boundary: (boundary, node, view frame, record).
    used: Vec<(u64, usize, u64, NodeRec)>,
    changes: Vec<(u64, usize)>,
    dropped: Vec<(usize, u64)>,
}

/// Every host runs `frames` boundaries on a thread: it makes its record, the session host
/// (host 0) announces `plan` (`(frame, D)`), the records come through the session into a
/// hub, and a boundary passes when the hub has what its view asks for.
fn run(
    sessions: Vec<Session>,
    frames: u64,
    d0: usize,
    plan: Vec<(u64, usize)>,
    exits: Vec<Option<(u64, Exit)>>,
) -> Vec<Out> {
    let handles: Vec<_> = sessions
        .into_iter()
        .enumerate()
        .map(|(h, mut s)| {
            let (exit, mut plan) = (exits[h], plan.clone());
            std::thread::spawn(move || {
                let me = s.local_nodes()[0];
                let n = s.params.nodes();
                let mut hub = LinkHub::new(n, d0);
                let mut o = Out::default();
                for f in 1..=frames {
                    match exit {
                        Some((at, Exit::Quit)) if at == f => {
                            s.bye("left");
                            return o;
                        }
                        Some((at, Exit::Vanish)) if at == f => return o,
                        _ => {}
                    }
                    let mut r = rec(me, f);
                    if h == 0 && !hub.change_pending(f) && plan.first().is_some_and(|p| p.0 <= f) {
                        let (_, nd) = plan.remove(0);
                        let from = f + hub.delay_at(f) as u64 + 1;
                        r.change = Some((from, nd as u8));
                        assert!(hub.schedule_delay(from, nd));
                    }
                    hub.put(me, f, r.clone());
                    s.publish(me, f, r);
                    let wait = Instant::now();
                    loop {
                        for ev in s.pump() {
                            match ev {
                                Event::Rec { node, frame, rec } => {
                                    hub.put(node, frame, rec);
                                }
                                Event::Dropped { host, frame } => {
                                    for &nd in s.nodes_of_host(host) {
                                        hub.leave(nd, frame);
                                    }
                                    o.dropped.push((host, frame));
                                }
                                Event::Left { .. } => {}
                            }
                        }
                        if let Some(e) = &s.end {
                            o.end = Some(e.clone());
                            return o;
                        }
                        if hub.has_all(f, &[me]) {
                            let vf = hub.view_frame(me, f).unwrap_or(0);
                            for k in (0..n).filter(|&k| k != me) {
                                if vf >= 1 {
                                    let need = hub.left_at(k).map_or(vf, |l| vf.min(l));
                                    o.used.push((f, k, vf, hub.rec(k, need).unwrap().into_owned()));
                                }
                            }
                            break;
                        }
                        assert!(wait.elapsed() < Duration::from_secs(15), "host {h} stuck at frame {f}");
                        std::thread::sleep(Duration::from_micros(200));
                    }
                    o.ran = f;
                    std::thread::sleep(Duration::from_micros(400));
                }
                let t = Instant::now();
                while s.acked_by_all() < frames && s.end.is_none() && t.elapsed() < Duration::from_secs(3) {
                    s.pump();
                    std::thread::sleep(Duration::from_micros(300));
                }
                o.changes = hub.changes().iter().map(|c| (c.from, c.d)).collect();
                o
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

/// The view frame at boundary `b` by the plain definition: `b + 1 - D(b)`, and once D has
/// been changed never behind what the last boundary's was.
fn reference_view(d0: usize, changes: &[(u64, usize)], b: u64) -> u64 {
    let mut vf = 0u64;
    for x in 1..=b {
        let d = changes.iter().rev().find(|c| c.0 <= x).map_or(d0, |c| c.1) as u64;
        let v = (x + 1).saturating_sub(d);
        vf = if changes.iter().any(|c| c.0 <= x) { vf.max(v) } else { v };
    }
    vf
}

/// Every survivor ran every frame, saw the same changes and drops, and was fed the view the
/// definition gives with the real records (the repeat of the drop frame's for a leaver's).
fn check(outs: &[Out], survivors: &[usize], frames: u64, d0: usize) -> Vec<(u64, usize)> {
    let sched = outs[survivors[0]].changes.clone();
    let mut drops0 = outs[survivors[0]].dropped.clone();
    drops0.sort();
    for &s in survivors {
        let o = &outs[s];
        assert_eq!(o.end, None, "host {s} ended");
        assert_eq!(o.ran, frames, "host {s} stopped early");
        assert_eq!(o.changes, sched, "host {s} has other delay changes");
        let mut drops = o.dropped.clone();
        drops.sort();
        assert_eq!(drops, drops0, "host {s} agrees other drop frames");
        let mut last = 0;
        for (b, node, vf, r) in &o.used {
            assert_eq!(*vf, reference_view(d0, &sched, *b), "host {s} boundary {b}");
            assert!(*vf >= last || *node != o.used[0].1, "host {s}: the view went back at boundary {b}");
            if *node == o.used[0].1 {
                last = *vf;
            }
            let drop = drops.iter().find(|d| d.0 == *node).map(|d| d.1);
            let want = drop.map_or(*vf, |df| (*vf).min(df));
            let mut r = r.clone();
            r.change = None;
            assert!(r == rec(*node, want), "host {s} boundary {b} node {node}: not the record of frame {want}");
        }
    }
    sched
}

fn lossy(seed: u64) -> Faults {
    Faults { latency_ms: 2.0, jitter_ms: 10.0, loss_pct: 5.0, dup_pct: 3.0, seed, ..Faults::default() }
}

#[test]
fn every_host_switches_at_the_same_frame_clean() {
    let s = connect(vec![Faults::default(); 2], 3);
    let plan = vec![(100, 6), (200, 5), (260, 4), (330, 7)];
    let outs = run(s, 450, 3, plan, vec![None, None]);
    let sched = check(&outs, &[0, 1], 450, 3);
    assert_eq!(sched.iter().map(|c| c.1).collect::<Vec<_>>(), [6, 5, 4, 7], "{sched:?}");
    // From = the announcing frame + the D then in force + 1.
    assert_eq!(sched[0].0, 104);
}

#[test]
fn and_under_loss_jitter_and_duplication_with_three_hosts() {
    let s = connect(vec![lossy(1), lossy(2), lossy(3)], 3);
    let plan = vec![(60, 5), (150, 8), (260, 7), (330, 2)];
    let outs = run(s, 420, 3, plan, vec![None, None, None]);
    let sched = check(&outs, &[0, 1, 2], 420, 3);
    assert_eq!(sched.len(), 4, "{sched:?}");
}

#[test]
fn a_player_leaving_with_a_change_in_flight() {
    // D 3 -> 6 announced with frame 100 takes effect at 104; host 2 quits at 102, in between.
    let s = connect(vec![Faults::default(); 3], 3);
    let outs = run(s, 300, 3, vec![(100, 6), (200, 4)], vec![None, None, Some((102, Exit::Quit))]);
    let sched = check(&outs, &[0, 1], 300, 3);
    assert_eq!(sched.iter().map(|c| c.1).collect::<Vec<_>>(), [6, 4], "{sched:?}");
    // And one that vanishes at the boundary the change takes effect, under loss.
    let s = connect(vec![lossy(4), lossy(5), lossy(6)], 3);
    let outs = run(s, 300, 3, vec![(100, 6), (200, 4)], vec![None, None, Some((104, Exit::Vanish))]);
    check(&outs, &[0, 1], 300, 3);
}

#[test]
fn the_announcer_leaving_right_after_its_record_still_changes_every_survivor_alike() {
    // Host 0 announces with frame 100 and quits at 102; host 1 is starved of host 0's records from
    // 99, so it has the record carrying the change only if host 2 relays it.
    let faults = vec![Faults { starve: Some((1, 99)), ..Faults::default() }, Faults::default(), Faults::default()];
    let s = connect(faults, 4);
    let outs = run(s, 300, 4, vec![(100, 7)], vec![Some((102, Exit::Quit)), None, None]);
    let sched = check(&outs, &[1, 2], 300, 4);
    assert_eq!(sched, vec![(105, 7)], "both survivors take the change the record 100 carried");
}

#[test]
fn two_changes_and_a_leave_under_loss() {
    let s = connect(vec![lossy(7), lossy(8), lossy(9)], 2);
    let outs = run(s, 360, 2, vec![(50, 5), (120, 6), (200, 3)], vec![None, Some((125, Exit::Quit)), None]);
    check(&outs, &[0, 2], 360, 2);
}
