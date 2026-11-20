//! mDNS / DNS-SD discovery: a host in a lobby advertises `_remodel._udp.local.`
//! on its game port with what a joiner needs to know in the TXT record, and
//! a joiner browses for those. It sits beside the broadcast of
//! [`crate::Discovery`], which stays as the fallback (a network that drops
//! multicast, or no multicast route). Both ends use the shared port 5353 the
//! way other mDNS responders do (Avahi on the same machine is fine).
//!
//! If the multicast sockets cannot be had, [`Advert::start`] and
//! [`Browser::start`] return an error; callers log it once and go on with the
//! broadcast alone.

use crate::setup::Found;
use crate::wire::PROTO;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// The DNS-SD service type.
pub const SERVICE: &str = "_remodel._udp.local.";

/// What a lobby is doing, as advertised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lobby {
    /// Open: players can join.
    Open,
    /// Every seat is taken.
    Full,
    /// The start is under way.
    Starting,
}

impl Lobby {
    fn text(self) -> &'static str {
        match self {
            Lobby::Open => "open",
            Lobby::Full => "full",
            Lobby::Starting => "starting",
        }
    }
    fn parse(s: &str) -> Option<Lobby> {
        Some(match s {
            "open" => Lobby::Open,
            "full" => Lobby::Full,
            "starting" => Lobby::Starting,
            _ => return None,
        })
    }
}

/// The TXT record of a host's advert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Txt {
    pub game: String,
    pub version: String,
    /// The hash of the program ROMs the session agreement compares.
    pub rom_hash: u64,
    pub players: u8,
    pub max: u8,
    /// The delay D the host would start with now.
    pub delay: u8,
    pub lobby: Lobby,
    /// The host's user name.
    pub name: String,
}

impl Txt {
    /// Seats still free.
    pub fn free(&self) -> u8 {
        self.max.saturating_sub(self.players)
    }

    /// The key/value strings of the record.
    pub fn encode(&self) -> Vec<(String, String)> {
        let kv = |k: &str, v: String| (k.to_string(), v);
        vec![
            kv("proto", PROTO.to_string()),
            kv("game", self.game.clone()),
            kv("ver", self.version.clone()),
            kv("rom", format!("{:016x}", self.rom_hash)),
            kv("players", self.players.to_string()),
            kv("max", self.max.to_string()),
            kv("free", self.free().to_string()),
            kv("d", self.delay.to_string()),
            kv("state", self.lobby.text().to_string()),
            kv("name", self.name.clone()),
        ]
    }

    /// The record from a lookup of its keys; None if the protocol is not ours
    /// or anything is missing or out of range.
    pub fn decode(get: &dyn Fn(&str) -> Option<String>) -> Option<Txt> {
        if get("proto")?.parse::<u8>().ok()? != PROTO {
            return None;
        }
        let t = Txt {
            game: get("game")?,
            version: get("ver")?,
            rom_hash: u64::from_str_radix(&get("rom")?, 16).ok()?,
            players: get("players")?.parse().ok()?,
            max: get("max")?.parse().ok()?,
            delay: get("d")?.parse().ok()?,
            lobby: Lobby::parse(&get("state")?)?,
            name: crate::names::sanitize_name(&get("name")?),
        };
        (t.max >= 2 && t.players <= t.max && t.max as usize <= crate::wire::MAX_HOSTS && !t.game.is_empty())
            .then_some(t)
    }
}

/// A host advertising itself. Dropping it withdraws the advert (a goodbye on
/// the network), so a host that exits cleanly disappears from lists at once.
pub struct Advert {
    daemon: ServiceDaemon,
    port: u16,
    host: String,
    instance: String,
    registered: Option<(String, Txt)>,
}

impl Advert {
    /// Start the multicast responder for a host on `port`.
    pub fn start(port: u16) -> Result<Advert, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS: {e}"))?;
        let tag = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let host = format!("remodel-{:08x}.local.", tag ^ std::process::id().rotate_left(12));
        Ok(Advert { daemon, port, host, instance: String::new(), registered: None })
    }

    /// Advertise `txt`, or withdraw the advert for None. Only a change is sent.
    pub fn set(&mut self, txt: Option<&Txt>) -> Result<(), String> {
        match (txt, &self.registered) {
            (Some(t), Some((_, old))) if t == old => Ok(()),
            (Some(t), reg) => {
                let instance = match reg {
                    Some(_) => self.instance.clone(),
                    None => format!("{} {} {}", t.name, t.game, self.port),
                };
                let props: HashMap<String, String> = t.encode().into_iter().collect();
                let info = ServiceInfo::new(SERVICE, &instance, &self.host, (), self.port, props)
                    .map_err(|e| format!("mDNS: {e}"))?
                    .enable_addr_auto();
                let full = info.get_fullname().to_string();
                self.daemon.register(info).map_err(|e| format!("mDNS: {e}"))?;
                self.instance = instance;
                self.registered = Some((full, t.clone()));
                Ok(())
            }
            (None, _) => {
                self.withdraw(Duration::from_millis(150));
                Ok(())
            }
        }
    }

    /// Whether the advert is out now.
    pub fn is_advertised(&self) -> bool {
        self.registered.is_some()
    }

    fn withdraw(&mut self, wait: Duration) {
        if let Some((full, _)) = self.registered.take() {
            if let Ok(rx) = self.daemon.unregister(&full) {
                let _ = rx.recv_timeout(wait);
            }
        }
    }
}

impl Drop for Advert {
    fn drop(&mut self) {
        self.withdraw(Duration::from_millis(500));
        if let Ok(rx) = self.daemon.shutdown() {
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
    }
}

/// Looking for adverts.
pub struct Browser {
    daemon: ServiceDaemon,
    rx: mdns_sd::Receiver<ServiceEvent>,
    hosts: HashMap<String, Found>,
}

impl Browser {
    pub fn start() -> Result<Browser, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS: {e}"))?;
        let rx = daemon.browse(SERVICE).map_err(|e| format!("mDNS: {e}"))?;
        Ok(Browser { daemon, rx, hosts: HashMap::new() })
    }

    /// Take in what has been heard; the hosts known now.
    pub fn poll(&mut self) -> Vec<Found> {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                ServiceEvent::ServiceResolved(r) => {
                    let get = |k: &str| r.get_property_val_str(k).map(str::to_string);
                    let Some(txt) = Txt::decode(&get) else { continue };
                    let addrs: Vec<IpAddr> = r.get_addresses().iter().map(|a| a.to_ip_addr()).collect();
                    if let Some(f) = found_from(&txt, r.get_port(), addrs) {
                        self.hosts.insert(r.get_fullname().to_string(), f);
                    }
                }
                ServiceEvent::ServiceRemoved(_, full) => {
                    self.hosts.remove(&full);
                }
                _ => {}
            }
        }
        let mut v: Vec<Found> = self.hosts.values().cloned().collect();
        v.sort_by_key(|a| a.addr);
        v
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE);
        if let Ok(rx) = self.daemon.shutdown() {
            let _ = rx.recv_timeout(Duration::from_millis(300));
        }
    }
}

/// A [`Found`] from an advert's record and addresses: the address to use is an IPv4 one
/// (the lobby listens on IPv4), the one on our own subnet first.
pub fn found_from(txt: &Txt, port: u16, addrs: Vec<IpAddr>) -> Option<Found> {
    let ours = crate::lan_address();
    let mut v4: Vec<Ipv4Addr> = addrs
        .iter()
        .filter_map(|a| match a {
            IpAddr::V4(a) if !a.is_unspecified() && !a.is_link_local() && !a.is_multicast() => Some(*a),
            _ => None,
        })
        .collect();
    v4.sort_by_key(|a| {
        let same_net = matches!(ours, Some(IpAddr::V4(o)) if o.octets()[..3] == a.octets()[..3]);
        (!same_net, !a.is_private(), a.octets())
    });
    let ip = *v4.first()?;
    Some(Found {
        addr: SocketAddr::new(IpAddr::V4(ip), port),
        game: txt.game.clone(),
        name: txt.name.clone(),
        players: txt.players as usize,
        max: txt.max as usize,
        version: txt.version.clone(),
        rom_hash: Some(txt.rom_hash),
        delay: txt.delay,
        lobby: txt.lobby,
        addrs: addrs.into_iter().collect(),
        mdns: true,
        broadcast: false,
    })
}

/// Why a joiner with `mine` cannot join `f`, if it cannot: a different version,
/// ROM hash or game, or no free seat. `have(game)` says whether the player has the
/// ROMs of another game than the chosen one.
pub fn blocker(f: &Found, mine: &Mine, have: &dyn Fn(&str) -> bool) -> Option<String> {
    if f.game != mine.game {
        return Some(if have(&f.game) {
            format!("a different game ({})", f.game)
        } else {
            format!("a game you don't have ({})", f.game)
        });
    }
    if f.version != mine.version {
        return Some(format!("reModel {} (you have {})", f.version, mine.version));
    }
    if f.rom_hash.is_some_and(|h| h != mine.rom_hash) {
        return Some("different ROMs".to_string());
    }
    if f.lobby == Lobby::Starting {
        return Some("already starting".to_string());
    }
    if f.lobby == Lobby::Full || f.players >= f.max {
        return Some("full".to_string());
    }
    None
}

/// What a joiner brings to compare against an advert.
pub struct Mine<'a> {
    pub game: &'a str,
    pub version: &'a str,
    pub rom_hash: u64,
}

/// One list from several: a host heard by both ways, or at more than one of its
/// addresses, is one entry (the richer one, mDNS before broadcast), in a stable order.
pub fn merge(lists: &[&[Found]]) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    // mDNS entries first: they carry the most.
    let mut all: Vec<&Found> = lists.iter().flat_map(|l| l.iter()).collect();
    all.sort_by_key(|f| (!f.mdns, f.addr));
    for f in all {
        match out.iter_mut().find(|o| o.same_host(f)) {
            Some(o) => {
                o.mdns |= f.mdns;
                o.broadcast |= f.broadcast;
                for a in &f.addrs {
                    if !o.addrs.contains(a) {
                        o.addrs.push(*a);
                    }
                }
                if o.rom_hash.is_none() {
                    o.rom_hash = f.rom_hash;
                }
            }
            None => out.push(f.clone()),
        }
    }
    out.sort_by_key(|a| (a.name.to_lowercase(), a.addr));
    out
}

/// Whether a multicast responder can be started here at all (a test's way to
/// skip itself where multicast is unavailable).
pub fn multicast_available() -> Result<(), String> {
    let d = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let _ = d.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt() -> Txt {
        Txt {
            game: "srallyc".into(),
            version: "0.11.1".into(),
            rom_hash: 0x0123_4567_89ab_cdef,
            players: 1,
            max: 2,
            delay: 3,
            lobby: Lobby::Open,
            name: "Sam".into(),
        }
    }

    #[test]
    fn the_record_round_trips() {
        let t = txt();
        let kv: HashMap<String, String> = t.encode().into_iter().collect();
        assert_eq!(kv["free"], "1");
        assert_eq!(kv["rom"], "0123456789abcdef");
        let back = Txt::decode(&|k| kv.get(k).cloned()).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn a_bad_record_is_not_a_host() {
        let mut kv: HashMap<String, String> = txt().encode().into_iter().collect();
        let get = |kv: &HashMap<String, String>| Txt::decode(&|k| kv.get(k).cloned());
        assert!(get(&kv).is_some());
        kv.insert("proto".into(), "99".into());
        assert!(get(&kv).is_none());
        kv.insert("proto".into(), PROTO.to_string());
        kv.insert("players".into(), "5".into());
        assert!(get(&kv).is_none());
        kv.insert("players".into(), "1".into());
        kv.insert("state".into(), "weird".into());
        assert!(get(&kv).is_none());
        kv.insert("state".into(), "open".into());
        kv.remove("name");
        assert!(get(&kv).is_none());
    }

    fn found(ip: [u8; 4], port: u16, name: &str, mdns: bool) -> Found {
        let t = Txt { name: name.into(), ..txt() };
        let mut f = found_from(&t, port, vec![IpAddr::from(ip)]).unwrap();
        if !mdns {
            f.mdns = false;
            f.broadcast = true;
            f.rom_hash = None;
        }
        f
    }

    #[test]
    fn the_lists_merge_without_doubles() {
        let a = [found([192, 168, 1, 20], 47890, "Sam", true)];
        let b = [found([192, 168, 1, 20], 47890, "Sam", false), found([192, 168, 1, 30], 47890, "Kim", false)];
        let m = merge(&[&b, &a]);
        assert_eq!(m.len(), 2);
        let sam = m.iter().find(|f| f.name == "Sam").unwrap();
        assert!(sam.mdns && sam.broadcast);
        assert!(sam.rom_hash.is_some());
        let kim = m.iter().find(|f| f.name == "Kim").unwrap();
        assert!(!kim.mdns && kim.broadcast);
    }

    #[test]
    fn a_host_at_two_addresses_is_one() {
        let t = txt();
        let two = found_from(&t, 47890, vec![[192, 168, 1, 20].into(), [10, 0, 0, 5].into()]).unwrap();
        let bc = found([10, 0, 0, 5], 47890, "Sam", false);
        let m = merge(&[&[bc], &[two]]);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn reasons_a_host_cannot_be_joined() {
        let f = found([192, 168, 1, 20], 47890, "Sam", true);
        let mine = Mine { game: "srallyc", version: "0.11.1", rom_hash: 0x0123_4567_89ab_cdef };
        let have = |_: &str| true;
        assert_eq!(blocker(&f, &mine, &have), None);
        assert!(blocker(&f, &Mine { version: "0.12.0", ..mine }, &have).unwrap().contains("0.11.1"));
        assert!(blocker(&f, &Mine { rom_hash: 1, ..mine }, &have).unwrap().contains("ROMs"));
        assert!(blocker(&f, &Mine { game: "daytona", ..mine }, &have).unwrap().contains("different game"));
        assert!(blocker(&f, &Mine { game: "daytona", ..mine }, &|_| false).unwrap().contains("don't have"));
        let full = Found { players: 2, ..f.clone() };
        assert_eq!(blocker(&full, &mine, &have).as_deref(), Some("full"));
        let starting = Found { lobby: Lobby::Starting, ..f };
        assert!(blocker(&starting, &mine, &have).unwrap().contains("starting"));
    }
}
