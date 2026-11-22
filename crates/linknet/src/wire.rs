//! The wire format: every packet is `RMLN`, the protocol version, a kind
//! byte and the kind's fields (little-endian). Everything received is
//! parsed with explicit bounds and rejected whole if anything is out of
//! range; nothing in a packet is trusted until it has been checked.

use model2::link::NodeRec;

pub const MAGIC: &[u8; 4] = b"RMLN";
/// Bumped whenever the packet layout or the session's rules change.
/// 2: player names in the lobby packets, the host's version and ROM hash in the
/// discovery answer.
/// 3: a host that leaves no longer ends the session: the data packet carries the
/// leave reports and a `Relay` packet carries a departed host's records.
/// 4: the ROM hash in the lobby packets and the discovery record covers every chip of
/// the set and its id, not the program alone (a host with other chips, or another
/// variant, is refused).
/// 5: the delay D can change mid-session: a record may announce `D becomes D' from frame F`
/// (flag bit 3 and `F`, `D'` after the start frame), and the data packet carries the sender's
/// held share of frames.
pub const PROTO: u8 = 5;
/// The largest datagram sent (under an Ethernet MTU) and read.
pub const MAX_PACKET: usize = 1400;
pub const MAX_HOSTS: usize = 8;
pub const MAX_NODES: usize = 8;
/// The most frames of records in one data packet.
pub const MAX_FRAMES: usize = 64;

pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn new(kind: u8) -> Self {
        let mut v = Vec::with_capacity(MAX_PACKET);
        v.extend_from_slice(MAGIC);
        v.push(PROTO);
        v.push(kind);
        Writer(v)
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn str(&mut self, s: &str) -> &mut Self {
        let b = &s.as_bytes()[..s.len().min(120)];
        // Cut on a character boundary.
        let b = std::str::from_utf8(b).map_or_else(|e| &b[..e.valid_up_to()], |_| b);
        self.0.push(b.len() as u8);
        self.0.extend_from_slice(b);
        self
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, at: 0 }
    }
    pub fn left(&self) -> usize {
        self.b.len() - self.at
    }
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }
    pub fn str(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        if n > 120 {
            return None;
        }
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }
}

/// FNV-1a, 64 bits.
pub fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

pub const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

/// What a session is agreed to run: sent by the host to every joiner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Params {
    pub session: u32,
    pub game: String,
    /// The link delay D (`LinkHub::delay`): frames.
    pub delay: u8,
    /// Frames between state checks (0: none).
    pub check_every: u16,
    /// Milliseconds of silence that end a session.
    pub timeout_ms: u32,
    /// Bytes in a link block (`LinkPlay::block`).
    pub block: u16,
    /// Per host, its address as the host saw it and the nodes it runs, in
    /// order; host 0 is the master's.
    pub hosts: Vec<HostEntry>,
    /// The settings record every node starts from (the master's), if the
    /// game has one to share.
    pub base: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostEntry {
    pub addr: String,
    pub name: String,
    pub nodes: Vec<u8>,
}

impl Params {
    pub fn nodes(&self) -> usize {
        self.hosts.iter().map(|h| h.nodes.len()).sum()
    }

    pub fn encode(&self, w: &mut Writer) {
        w.u32(self.session).str(&self.game).u8(self.delay).u16(self.check_every).u32(self.timeout_ms).u16(self.block);
        w.u8(self.hosts.len() as u8);
        for h in &self.hosts {
            w.str(&h.addr).str(&h.name).u8(h.nodes.len() as u8);
            for &n in &h.nodes {
                w.u8(n);
            }
        }
        match &self.base {
            Some(b) => {
                w.u8(1).u16(b.len() as u16);
                w.0.extend_from_slice(b);
            }
            None => {
                w.u8(0);
            }
        }
    }

    pub fn decode(r: &mut Reader) -> Option<Params> {
        let (session, game, delay) = (r.u32()?, r.str()?, r.u8()?);
        let (check_every, timeout_ms, block) = (r.u16()?, r.u32()?, r.u16()?);
        let nh = r.u8()? as usize;
        if !(2..=MAX_HOSTS).contains(&nh) {
            return None;
        }
        let mut hosts = Vec::new();
        for _ in 0..nh {
            let (addr, name, k) = (r.str()?, r.str()?, r.u8()? as usize);
            if k == 0 || k > MAX_NODES {
                return None;
            }
            let nodes = r.bytes(k)?.to_vec();
            hosts.push(HostEntry { addr, name: crate::names::sanitize_name(&name), nodes });
        }
        let base = match r.u8()? {
            0 => None,
            1 => {
                let n = r.u16()? as usize;
                if n > 512 {
                    return None;
                }
                Some(r.bytes(n)?.to_vec())
            }
            _ => return None,
        };
        let p = Params { session, game, delay, check_every, timeout_ms, block, hosts, base };
        // The nodes are 0..N once each, in host order.
        let all: Vec<u8> = p.hosts.iter().flat_map(|h| h.nodes.iter().copied()).collect();
        if all.len() > MAX_NODES || !all.iter().enumerate().all(|(i, &n)| n as usize == i) {
            return None;
        }
        if p.delay == 0 || p.delay > 60 || p.block == 0 || p.block > 1024 || r.left() != 0 {
            return None;
        }
        Some(p)
    }
}

pub mod kind {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const REFUSE: u8 = 3;
    pub const LOBBY: u8 = 4;
    pub const PING: u8 = 5;
    pub const PONG: u8 = 6;
    pub const START: u8 = 7;
    pub const START_ACK: u8 = 8;
    pub const GO: u8 = 9;
    pub const DATA: u8 = 10;
    pub const BYE: u8 = 11;
    pub const ABORT: u8 = 12;
    pub const DISCOVER: u8 = 13;
    pub const INFO: u8 = 14;
    pub const RELAY: u8 = 15;
}

/// A datagram split into its kind and fields, or None if it isn't ours. The
/// version is checked by the caller (a `Hello` of another version is
/// answered, not dropped).
pub fn header(p: &[u8]) -> Option<(u8, u8, Reader<'_>)> {
    if p.len() < 6 || p.len() > MAX_PACKET || &p[..4] != MAGIC {
        return None;
    }
    Some((p[4], p[5], Reader::new(&p[6..])))
}

/// Encode a record against `base` (the same node's previous frame's block):
/// flags, the start frame while started, then the block as runs of
/// unchanged bytes and changed bytes.
pub fn encode_rec(w: &mut Writer, rec: &NodeRec, base: &[u8], block: usize) {
    let has_tx = rec.started && rec.tx.len() == block;
    w.u8(rec.started as u8 | (rec.master as u8) << 1 | (has_tx as u8) << 2 | (rec.change.is_some() as u8) << 3);
    if rec.started {
        w.u64(rec.start_frame);
    }
    if let Some((from, d)) = rec.change {
        w.u64(from).u8(d);
    }
    if !has_tx {
        return;
    }
    let delta: Vec<u8> = rec.tx.iter().zip(base).map(|(a, b)| a ^ b).collect();
    let mut at = 0;
    while at < block {
        let z = delta[at..].iter().take(255).take_while(|&&b| b == 0).count();
        at += z;
        let mut l = 0;
        while at + l < block && l < 255 {
            // A run of three zeros costs more as literals than as a new token.
            if delta[at + l..].iter().take(3).filter(|&&b| b == 0).count() == 3 {
                break;
            }
            l += 1;
        }
        w.u8(z as u8).u8(l as u8);
        w.0.extend_from_slice(&delta[at..at + l]);
        at += l;
    }
}

/// Decode what [`encode_rec`] wrote; None if it is malformed in any way.
pub fn decode_rec(r: &mut Reader, base: &[u8], block: usize) -> Option<NodeRec> {
    let flags = r.u8()?;
    if flags & !15 != 0 || (flags & 4 != 0 && flags & 1 == 0) {
        return None;
    }
    let started = flags & 1 != 0;
    let start_frame = if started { r.u64()? } else { 0 };
    let change = if flags & 8 != 0 {
        let (from, d) = (r.u64()?, r.u8()?);
        // A delay of 1 to 60 frames, from a boundary that is not the first.
        if !(1..=60).contains(&d) || from < 2 {
            return None;
        }
        Some((from, d))
    } else {
        None
    };
    let mut tx = Vec::new();
    if flags & 4 != 0 {
        tx = base.to_vec();
        let mut at = 0usize;
        while at < block {
            let (z, l) = (r.u8()? as usize, r.u8()? as usize);
            if z + l == 0 || at + z + l > block {
                return None;
            }
            at += z;
            for (d, s) in tx[at..at + l].iter_mut().zip(r.bytes(l)?) {
                *d ^= s;
            }
            at += l;
        }
    }
    Some(NodeRec { started, master: flags & 2 != 0, start_frame, tx, change })
}

/// The hash two hosts compare at a checkpoint: every node's record of one frame.
pub fn hash_recs(recs: &[&NodeRec]) -> u64 {
    let mut h = FNV_START;
    for r in recs {
        h = fnv(h, &[r.started as u8, r.master as u8]);
        h = fnv(h, &r.start_frame.to_le_bytes());
        h = fnv(h, &(r.tx.len() as u32).to_le_bytes());
        h = fnv(h, &r.tx);
        if let Some((from, d)) = r.change {
            h = fnv(h, &[0xde, d]);
            h = fnv(h, &from.to_le_bytes());
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(seed: u8, started: bool) -> NodeRec {
        let tx = (0..448u32).map(|i| if i % 7 == 0 { (i as u8).wrapping_mul(seed) } else { 0 }).collect();
        NodeRec {
            started,
            master: seed == 1,
            start_frame: if started { 926 } else { 0 },
            tx: if started { tx } else { vec![] },
            change: None,
        }
    }

    #[test]
    fn a_record_round_trips_against_any_base() {
        for (seed, base_seed) in [(1u8, 0u8), (3, 5), (9, 9)] {
            for started in [true, false] {
                let (r, base) = (rec(seed, started), rec(base_seed, true).tx);
                let mut w = Writer::new(kind::DATA);
                encode_rec(&mut w, &r, &base, 448);
                let mut rd = Reader::new(&w.0[6..]);
                assert_eq!(decode_rec(&mut rd, &base, 448).as_ref(), Some(&r));
                assert_eq!(rd.left(), 0);
            }
        }
    }

    #[test]
    fn a_delay_change_rides_in_the_record_and_is_checked() {
        let mut r = rec(3, true);
        r.change = Some((1200, 4));
        let base = rec(1, true).tx;
        let mut w = Writer::new(kind::DATA);
        encode_rec(&mut w, &r, &base, 448);
        let mut rd = Reader::new(&w.0[6..]);
        assert_eq!(decode_rec(&mut rd, &base, 448).as_ref(), Some(&r));
        assert_ne!(hash_recs(&[&r]), hash_recs(&[&rec(3, true)]), "the state check sees it");
        // Out of range delays and a change from the first boundary are refused.
        for (from, d) in [(1200u64, 0u8), (1200, 61), (1, 3)] {
            let mut bad = r.clone();
            bad.change = Some((from, d));
            let mut w = Writer::new(kind::DATA);
            encode_rec(&mut w, &bad, &base, 448);
            assert!(decode_rec(&mut Reader::new(&w.0[6..]), &base, 448).is_none());
        }
    }

    #[test]
    fn an_unchanged_block_is_a_few_bytes_and_garbage_is_refused() {
        let r = rec(3, true);
        let mut w = Writer::new(kind::DATA);
        encode_rec(&mut w, &r, &r.tx, 448);
        assert!(w.0.len() < 32, "{} bytes", w.0.len());
        // Every truncation and a block overrun are refused.
        let mut w = Writer::new(kind::DATA);
        encode_rec(&mut w, &r, &vec![0; 448], 448);
        for cut in 0..w.0.len() - 6 {
            assert!(decode_rec(&mut Reader::new(&w.0[6..6 + cut]), &vec![0; 448], 448).is_none(), "cut {cut}");
        }
        assert!(decode_rec(&mut Reader::new(&[7, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]), &vec![0; 448], 448).is_none());
        assert!(decode_rec(&mut Reader::new(&[0x80]), &[], 448).is_none(), "unknown flag bits");
    }

    #[test]
    fn params_round_trip_and_bad_node_lists_are_refused() {
        let p = Params {
            session: 7,
            game: "srallyc".into(),
            delay: 3,
            check_every: 60,
            timeout_ms: 5000,
            block: 448,
            hosts: vec![
                HostEntry { addr: "10.0.0.1:47890".into(), name: "a".into(), nodes: vec![0] },
                HostEntry { addr: "10.0.0.2:47891".into(), name: "b".into(), nodes: vec![1, 2] },
            ],
            base: Some(vec![1, 2, 3]),
        };
        let mut w = Writer::new(kind::START);
        p.encode(&mut w);
        assert_eq!(Params::decode(&mut Reader::new(&w.0[6..])), Some(p.clone()));
        let mut bad = p.clone();
        bad.hosts[1].nodes = vec![2, 1];
        let mut w = Writer::new(kind::START);
        bad.encode(&mut w);
        assert_eq!(Params::decode(&mut Reader::new(&w.0[6..])), None);
        for cut in 0..w.0.len() - 6 {
            assert!(Params::decode(&mut Reader::new(&w.0[6..6 + cut])).is_none());
        }
    }
}
