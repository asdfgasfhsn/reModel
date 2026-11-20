//! The UDP socket with optional fault injection (added latency, jitter,
//! loss and duplication, for tests), shared by the lobby and the session.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Faults applied to every datagram sent (not to what is received).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Faults {
    /// Added one-way delay and up to this much more, random, in milliseconds.
    pub latency_ms: f32,
    pub jitter_ms: f32,
    /// Percent of datagrams dropped / sent twice.
    pub loss_pct: f32,
    pub dup_pct: f32,
    pub seed: u64,
    /// A test of desync detection: flip a bit of the record received for this frame (0: none).
    pub corrupt_frame: u64,
}

impl Faults {
    pub fn any(&self) -> bool {
        self.latency_ms > 0.0 || self.jitter_ms > 0.0 || self.loss_pct > 0.0 || self.dup_pct > 0.0
    }
}

pub struct Net {
    pub sock: UdpSocket,
    pub faults: Faults,
    rng: u64,
    /// Datagrams waiting out their added delay: (due, to, bytes).
    delayed: Vec<(Instant, SocketAddr, Vec<u8>)>,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub pkts_tx: u64,
    pub pkts_rx: u64,
    pub lost_by_fault: u64,
}

impl Net {
    pub fn new(sock: UdpSocket, faults: Faults) -> std::io::Result<Net> {
        sock.set_nonblocking(true)?;
        Ok(Net {
            sock,
            rng: faults.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            faults,
            delayed: Vec::new(),
            bytes_tx: 0,
            bytes_rx: 0,
            pkts_tx: 0,
            pkts_rx: 0,
            lost_by_fault: 0,
        })
    }

    /// A random number in [0, 1).
    fn rand(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    }

    fn raw(&mut self, to: SocketAddr, bytes: &[u8]) {
        if self.sock.send_to(bytes, to).is_ok() {
            self.bytes_tx += bytes.len() as u64;
            self.pkts_tx += 1;
        }
    }

    /// Send a datagram, subject to the faults.
    pub fn send(&mut self, to: SocketAddr, bytes: &[u8]) {
        if !self.faults.any() {
            return self.raw(to, bytes);
        }
        if self.rand() * 100.0 < self.faults.loss_pct {
            self.lost_by_fault += 1;
            return;
        }
        let copies = 1 + (self.rand() * 100.0 < self.faults.dup_pct) as usize;
        for _ in 0..copies {
            let ms = self.faults.latency_ms + self.rand() * self.faults.jitter_ms;
            self.delayed.push((Instant::now() + Duration::from_secs_f32(ms / 1000.0), to, bytes.to_vec()));
        }
        self.flush();
    }

    /// Send what has waited long enough.
    pub fn flush(&mut self) {
        if self.delayed.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut due: Vec<(Instant, SocketAddr, Vec<u8>)> = Vec::new();
        self.delayed.retain(|d| {
            if d.0 <= now {
                due.push(d.clone());
                false
            } else {
                true
            }
        });
        due.sort_by_key(|d| d.0);
        for (_, to, b) in due {
            self.raw(to, &b);
        }
    }

    /// A datagram waiting, if any.
    pub fn recv(&mut self) -> Option<(Vec<u8>, SocketAddr)> {
        let mut buf = [0u8; 2048];
        match self.sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                self.bytes_rx += n as u64;
                self.pkts_rx += 1;
                Some((buf[..n].to_vec(), from))
            }
            Err(_) => None,
        }
    }
}

/// This machine's address on the local network (the one it would use to
/// reach the Internet; nothing is sent), if it has one.
pub fn lan_address() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}
