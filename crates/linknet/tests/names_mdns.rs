//! Player names and discovery over loopback: names reaching the lobby, the
//! protocol refusals, the broadcast answer, and the mDNS advert following
//! its lobby (skipped with a message where multicast is not available).

use linknet::*;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

fn cfg(name: &str) -> Config {
    Config {
        game: "srallyc".into(),
        rom_hash: 0x1234,
        version: "t".into(),
        name: name.into(),
        block: 448,
        max_hosts: 4,
        faults: Faults::default(),
        timeout: Duration::from_millis(700),
        check_every: 10,
    }
}

fn lo() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Poll until `f` gives Some, or panic after a few seconds.
fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let t = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(t.elapsed() < Duration::from_secs(5), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn names_reach_the_lobby_and_the_joiners() {
    let mut host = Host::bind(cfg("  Alex\u{7} "), lo()).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    let mut a = Join::connect(cfg("Sam"), addr).unwrap();
    let mut b = Join::connect(cfg("Sam"), addr).unwrap();
    let names = until("both joined and told the names", || {
        host.poll();
        match (a.poll(), b.poll()) {
            (JoinStatus::Lobby { names: na, .. }, JoinStatus::Lobby { names: nb, .. })
                if na.len() == 3 && nb.len() == 3 =>
            {
                Some((na, nb))
            }
            _ => None,
        }
    });
    // The host's name cleaned, the two Sams told apart, on the host and on every joiner.
    let want = ["Alex", "Sam", "Sam (2)"];
    assert_eq!(names.0, want);
    assert_eq!(names.1, want);
    assert_eq!(host.names(), want);
    let joined: Vec<String> = host.joiners().into_iter().map(|j| j.0).collect();
    assert_eq!(joined, ["Sam", "Sam (2)"]);
}

#[test]
fn an_old_protocol_is_refused_with_a_message() {
    let mut host = Host::bind(cfg("h"), lo()).unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{}", host.local_addr().port()).parse().unwrap();
    // A hello as protocol 1 sent it.
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    let mut hello = b"RMLN".to_vec();
    hello.extend([1, wire::kind::HELLO]);
    sock.send_to(&hello, addr).unwrap();
    let reply = until("a refusal", || {
        host.poll();
        let mut buf = [0u8; 1400];
        sock.recv_from(&mut buf).ok().map(|(n, _)| buf[..n].to_vec())
    });
    assert_eq!(reply[5], wire::kind::REFUSE);
    let mut r = wire::Reader::new(&reply[6..]);
    let why = r.str().unwrap();
    assert!(why.contains("protocol 1") && why.contains("same reModel version"), "{why}");
    assert_eq!(host.players(), 1);
}

#[test]
fn a_joiner_hears_a_host_of_another_protocol() {
    // A fake old host: answers any packet with a refusal under protocol 1.
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_nonblocking(true).unwrap();
    let addr = sock.local_addr().unwrap();
    let mut j = Join::connect(cfg("Sam"), addr).unwrap();
    let why = until("a failure", || {
        let mut buf = [0u8; 1400];
        if let Ok((_, from)) = sock.recv_from(&mut buf) {
            let mut p = b"RMLN".to_vec();
            p.extend([1, wire::kind::REFUSE, 0]);
            let _ = sock.send_to(&p, from);
        }
        match j.poll() {
            JoinStatus::Failed(e) => Some(e),
            _ => None,
        }
    });
    assert!(why.contains("another version of reModel"), "{why}");
}

#[test]
fn the_broadcast_answer_carries_what_the_join_list_needs() {
    let mut host = Host::bind(cfg("Alex"), lo()).unwrap();
    let port = host.local_addr().port();
    let mut d = Discovery::to(format!("127.0.0.1:{port}").parse().unwrap()).unwrap();
    let f = until("an answer", || {
        host.poll();
        d.poll().first().cloned()
    });
    assert_eq!((f.name.as_str(), f.game.as_str(), f.players, f.max), ("Alex", "srallyc", 1, 4));
    assert_eq!((f.version.as_str(), f.rom_hash), ("t", Some(0x1234)));
    assert!(f.broadcast && !f.mdns);
}

/// The mDNS advert follows the lobby (listed while there is a free seat, updated as players
/// join, withdrawn when the lobby is full) and a browser sees it. Skips where multicast is
/// not available (a sandbox, CI without a multicast route).
#[test]
fn mdns_advert_is_found_updated_and_withdrawn() {
    if let Err(e) = mdns::multicast_available() {
        eprintln!("skipping the mDNS test: multicast is not available here ({e})");
        return;
    }
    let mut c = cfg("Alex");
    c.max_hosts = 3;
    // A game name of its own, so other runs on this network are not mistaken for this one.
    let game = format!("t{:x}", std::process::id());
    c.game = game.clone();
    let mut host = Host::bind(c.clone(), "0.0.0.0:0".parse().unwrap()).unwrap();
    let port = host.local_addr().port();
    if let Err(e) = host.advertise() {
        eprintln!("skipping the mDNS test: {e}");
        return;
    }
    let mut browser = match Browser::start() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping the mDNS test: {e}");
            return;
        }
    };
    let mine = |b: &mut Browser, host: &mut Host| -> Vec<Found> {
        host.poll();
        b.poll().into_iter().filter(|f| f.game == game && f.addr.port() == port).collect()
    };
    // Multicast may be on the machine but not delivered (no route, a filtering switch): skip if nothing shows.
    let t = Instant::now();
    let first = loop {
        if let Some(f) = mine(&mut browser, &mut host).first() {
            break f.clone();
        }
        if t.elapsed() > Duration::from_secs(8) {
            eprintln!("skipping the mDNS test: the advert was not heard on this network in 8 s");
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!((first.name.as_str(), first.players, first.max), ("Alex", 1, 3));
    assert_eq!((first.version.as_str(), first.rom_hash), ("t", Some(0x1234)));
    assert!(first.mdns);

    // A player joins: the record is updated.
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut jc = cfg("Sam");
    jc.game = game.clone();
    let mut j = Join::connect(jc, addr).unwrap();
    until("the update", || {
        j.poll();
        mine(&mut browser, &mut host).first().filter(|f| f.players == 2).cloned()
    });
    // The lobby fills: the advert is withdrawn.
    let mut jc2 = cfg("Kim");
    jc2.game = game.clone();
    let mut j2 = Join::connect(jc2, addr).unwrap();
    until("the withdrawal", || {
        j.poll();
        j2.poll();
        mine(&mut browser, &mut host).is_empty().then_some(())
    });
    assert!(!host.advertised());
    // Someone leaves: it is back.
    j2.leave();
    until("the return", || {
        j.poll();
        mine(&mut browser, &mut host).first().filter(|f| f.players == 2).cloned()
    });
    // Dropping the host says goodbye.
    drop(host);
    until("the goodbye", || {
        browser.poll().into_iter().all(|f| !(f.game == game && f.addr.port() == port)).then_some(())
    });
}
