//! ROM set manifests and loading for Model 2 games.
//!
//! Game data is always user-supplied. A manifest only records the chip names,
//! sizes and SHA-1 hashes needed to recognise a good dump and assemble it into
//! the address spaces the hardware sees.
//!
//! A ROM directory holds sets as MAME does, by set name: `ROOT/<set>.zip`
//! or unpacked in `ROOT/<set>/`. Loose chips directly in `ROOT` also work.

pub mod bindings;
pub mod controls;
pub mod games;
pub mod locate;
mod sha1;
pub mod variants;
pub mod zip;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use locate::{Candidate, HashCache, Locator};

pub use bindings::Bindings;
pub use controls::{
    AnalogInput, ButtonInput, Controls, Gun, GunSource, KeyRamp, PadAxis, PadButton, Reload, Shifter, StickDirs,
};
pub use sha1::sha1_hex;
pub use variants::{ChipSwap, Variant, VariantKind};

/// Cabinet boards beyond the Model 2 board itself that the game talks to
/// through the I/O chip, and which protocol they speak. The board model
/// (`model2::drive`) emulates each kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveBoard {
    /// Sega Ski Super G's platform drive board: one command byte a frame
    /// on I/O `0x1C00008`, one reply byte on `0x1C00006`; boots with a
    /// `0x00` -> `0xCE` handshake (docs/games/skisuprg.md).
    SkiPlatform,
    /// Sega Touring Car Championship's force feedback wheel: one command
    /// byte a frame on `0x1C00008`; the game stores the reply from
    /// `0x1C00006` but never checks it (docs/games/stcc.md).
    Wheel,
    /// Manx TT Superbike's deluxe "Active Motion Slider" (the bike's
    /// platform): one command byte a frame on `0x1C00008`, one reply on
    /// `0x1C00006`; boots with a `0xCE` then `0xCD` handshake
    /// (docs/games/manxtt.md).
    MotionSlider,
    /// Rail Chase 2's ride drive board, linked "by network": one command
    /// byte a frame on `0x1C00008`, the reply nibble on `0x1C00006`; boots
    /// with a wire test of eight walking-bit commands (docs/games/rchase2.md).
    RideLink,
}

/// A communication (link) board in the cabinet's expansion slot at
/// `0x1A10000`, which the game talks to for linking cabinets. With none
/// fitted the slot reads as all ones ("no board"); the model
/// (`model2::link`) answers each kind as far as the game's code checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkBoard {
    /// A board with no other cabinet on the link: it reports itself fitted
    /// and idle (`0x1A14000` bit 0 clear, `0x1A14002` bit 7 clear), which
    /// is all a game set to stand-alone checks for (`von`,
    /// docs/games/von.md).
    Idle,
    /// A board whose `0x1A14000` register holds what the game last wrote to it (bit 0 at least),
    /// which `skisuprg`'s board check reads back (write 1: bit 0 reads 1, write 0: it reads 0; an
    /// empty slot reads all ones, so it fails the second half; docs/hw/link-board.md section 17).
    Latch,
}

/// How one game sets up and lays out link play (`docs/hw/link-board.md`): what
/// `model2::link::LinkHub` and `remodel link-run` need to run several machines
/// of it. Per-game data in `games/link.rs`, found with [`games::link_play`].
#[derive(Clone, Copy, Debug)]
pub struct LinkPlay {
    /// Cabinets the game can link (the receive blocks that fit the link RAM
    /// bound it too: 16 KB).
    pub max_nodes: usize,
    /// Where the operator settings live and the record of them.
    pub settings: LinkSettings,
    /// Byte of the record that selects the cabinet's role, and its value for
    /// node 0 (the master), 1, 2, ... (the last repeats for more nodes).
    pub role_off: usize,
    pub roles: &'static [u8],
    /// A byte of the record that numbers the cabinet (`daytona`'s CAR NUMBER:
    /// the node's index, 0-based), if the game has one apart from the role.
    pub car_off: Option<usize>,
    /// Offsets in the link RAM (`0x1A10000`): the game's own block, the
    /// first received block, and the size of a block.
    pub tx: usize,
    pub rx: usize,
    pub block: usize,
    /// The node ID register (`0x1A10002`) holds the node's 0-based number
    /// plus this.
    pub id_base: u8,
    /// Where a block holds the node's state word, if known (logged by
    /// `link-run`), and three floats of position.
    pub state_off: Option<usize>,
    pub pos_off: Option<usize>,
    /// A player who leaves a running session is shown by the hub as a copy of the lowest node still
    /// playing, but for these byte ranges of the block, which stay the leaver's last (its identity
    /// and its car's pose), and, if the block has one, the offset of the word-sum checksum
    /// (the sum of the first 16 words but itself) the copy must have recomputed
    /// (`docs/hw/link-board.md` section 12.3).
    pub ghost_keep: &'static [(usize, usize)],
    pub ghost_sum: Option<usize>,
    /// What the game's own FREE PLAY coin setting changes in the record, if
    /// found (`docs/games/<set>.md`); a game without it has no free play.
    pub free_play: Option<&'static [FreePlayByte]>,
    /// Bytes of a linked node's record that depend on the node and the number of nodes in the
    /// session, beyond the role and `car_off` (`sgt24h`'s LINK MAX must equal the number of
    /// machines): `(node, nodes)` gives (offset, value) pairs.
    pub node_bytes: Option<NodeBytes>,
}

/// A function of a node and the session's node count that gives bytes (offset, value) of the node's
/// settings record ([`LinkPlay::node_bytes`]).
pub type NodeBytes = fn(usize, usize) -> Vec<(usize, u8)>;

/// One byte of the settings record the COIN ASSIGNMENT page's FREE PLAY
/// changes: its offset, the value with free play on and the value at the
/// game's defaults.
#[derive(Clone, Copy, Debug)]
pub struct FreePlayByte {
    pub at: usize,
    pub on: u8,
    pub off: u8,
}

impl LinkPlay {
    /// The bytes besides the role that make node `node` of `nodes` what it is: its ID byte
    /// (`car_off`, the node's index) and what [`LinkPlay::node_bytes`] says; `extra` last (a test's).
    pub fn node_set(&self, node: usize, nodes: usize, extra: &[(usize, u8)]) -> Vec<(usize, u8)> {
        let mut set: Vec<(usize, u8)> = self.car_off.map(|o| (o, node as u8)).into_iter().collect();
        if let Some(f) = self.node_bytes {
            set.extend(f(node, nodes));
        }
        set.extend_from_slice(extra);
        set
    }

    /// The bytes that put the game on free play (`on`), as `set` bytes for
    /// [`LinkSettings::image`]; none if the game has no free play data. With
    /// `on` false and a base that already holds every free play byte (a
    /// settings file written by an earlier free play session), the bytes
    /// that undo it, so a session's choice never outlives it.
    pub fn free_play_set(&self, base: Option<&[u8]>, on: bool) -> Vec<(usize, u8)> {
        let Some(fp) = self.free_play else { return vec![] };
        if on {
            return fp.iter().map(|b| (b.at, b.on)).collect();
        }
        match base {
            Some(img) if fp.iter().all(|b| img.get(b.at) == Some(&b.on)) => fp.iter().map(|b| (b.at, b.off)).collect(),
            _ => vec![],
        }
    }

    /// The settings record a linked session starts from (what `LinkSettings::image`
    /// takes as its `base`): the player's own settings file `own` (only a game that keeps
    /// its settings in the EEPROM has one) with free play set as `free` says. None if
    /// that is just the game's defaults.
    pub fn base_with_free_play(&self, own: Option<&[u8]>, free: bool) -> Option<Vec<u8>> {
        self.with_free_play(own.filter(|_| self.settings.store == SettingsStore::Eeprom), free)
    }

    /// `base` with free play set as `free` says, whatever the store (a test's settings file).
    pub fn with_free_play(&self, own: Option<&[u8]>, free: bool) -> Option<Vec<u8>> {
        let set = self.free_play_set(own, free);
        if set.is_empty() {
            return own.map(<[u8]>::to_vec);
        }
        let role = own.and_then(|b| b.get(self.role_off).copied()).unwrap_or(self.settings.record[self.role_off]);
        Some(self.settings.image(own, self.role_off, role, &set))
    }

    /// Take free play out of a settings image a session saved, if it is exactly
    /// what [`Self::free_play_set`] put in (checksum fixed): a session's free play
    /// is not kept in the player's own settings.
    pub fn unset_free_play(&self, image: &mut [u8]) {
        let set = self.free_play_set(Some(image), false);
        for &(o, v) in &set {
            image[o] = v;
        }
        if !set.is_empty() {
            self.settings.crc.fix(image);
        }
    }
}

/// The EEPROM's size in bytes. A settings image longer than this carries, from here on, the first
/// bytes of the backup SRAM (`sgt24h`, whose coin settings are there).
pub const EEPROM_BYTES: usize = 128;

/// The record of operator settings a game keeps, and where.
#[derive(Clone, Copy, Debug)]
pub struct LinkSettings {
    pub store: SettingsStore,
    /// The record at its defaults, with the role set to stand-alone; the
    /// rest of the store is filled with `fill`.
    pub record: &'static [u8],
    pub fill: u8,
    /// The size of the store image.
    pub size: usize,
    pub crc: RecordCrc,
}

/// Where a game keeps its operator settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsStore {
    /// The 93C46 EEPROM, 128 bytes (`srallyc`: the 2A boards).
    Eeprom,
    /// The original Model 2's I/O board buffer at `0x1C00200`, 128 bytes,
    /// byte N at word N (`daytona`).
    IoBoard,
    /// The battery-backed SRAM at `0x1D00000` (`von`): the image is the start of it, the
    /// rest keeps its power-on ones.
    BackupRam,
}

/// How a record's checksum is computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SumKind {
    /// CRC-16, CCITT polynomial `0x1021`, MSB first, start value `init`
    /// (low 16 bits), complemented if `complement` (`srallyc`, `manxtt`,
    /// `daytona`).
    Ccitt,
    /// The shifting checksum of the "S32A" records (`docs/hw/eeprom-records.md`
    /// section 2): a 32-bit register started at `init`, whose low byte takes
    /// each data byte, shifted left eight times a byte with `0x10210000`
    /// added whenever a 1 leaves the top (an addition, not an exclusive or),
    /// the result the top 16 bits.
    S32a,
    /// The low 16 bits of the sum of the bytes (`sgt24h`'s record).
    ByteSum,
    /// `von`'s settings in the backup SRAM (`docs/hw/link-board.md` section 15.2): a table-driven
    /// CRC-16 (the CCITT table) over a 32-bit register started at `init`; each byte is added to
    /// the register, which is shifted left eight bits and has the table entry of its old top
    /// byte exclusive-ored into bits 16-31; the result is the table entry of the top byte
    /// exclusive-ored with the register's bits 8-23.
    Table,
}

/// A record's checksum over bytes `from..to`, stored at `at` low byte first
/// ([`SumKind`]). `copy` is the size of the record that starts at `at`
/// when the store keeps a second, identical copy right after it (the "S32A"
/// records): [`RecordCrc::fix`] then rewrites the copy as well.
#[derive(Clone, Copy, Debug)]
pub struct RecordCrc {
    pub kind: SumKind,
    pub at: usize,
    pub from: usize,
    pub to: usize,
    pub init: u32,
    pub complement: bool,
    pub copy: Option<usize>,
    /// Offset in the record of a 2-byte marker the second copy has zeroed (`sgt24h`: the
    /// first copy carries `0xAD85`, the copy zeros, each with its own checksum).
    pub copy_clear: Option<usize>,
}

impl RecordCrc {
    /// A CCITT checksum (the `srallyc` kind), no second copy.
    pub const fn ccitt(at: usize, from: usize, to: usize, init: u16, complement: bool) -> Self {
        RecordCrc { kind: SumKind::Ccitt, at, from, to, init: init as u32, complement, copy: None, copy_clear: None }
    }

    /// An "S32A" record's checksum (`docs/hw/eeprom-records.md`): the record is `size` bytes
    /// from `at` (checksum word first) and repeated right after; `len` bytes after the
    /// checksum word's two are fed to the shifting sum, started at `init`.
    pub const fn s32a(at: usize, size: usize, len: usize, init: u32) -> Self {
        RecordCrc {
            kind: SumKind::S32a,
            at,
            from: at + 2,
            to: at + 2 + len,
            init,
            complement: false,
            copy: Some(size),
            copy_clear: None,
        }
    }

    /// `sgt24h`'s record: the byte sum of the `len` bytes after the checksum word, the record
    /// `size` bytes from `at`, repeated right after with the 2 bytes `clear` into the record
    /// zeroed (`docs/hw/eeprom-records.md` section 3).
    pub const fn byte_sum(at: usize, size: usize, len: usize, clear: usize) -> Self {
        RecordCrc {
            kind: SumKind::ByteSum,
            at,
            from: at + 2,
            to: at + 2 + len,
            init: 0,
            complement: false,
            copy: Some(size),
            copy_clear: Some(clear),
        }
    }

    /// `von`'s settings checksum ([`SumKind::Table`]): the word at `at` covers `from..to`; the
    /// record (`copy` bytes, header included) is repeated right after.
    pub const fn table(at: usize, from: usize, to: usize, init: u32, copy: usize) -> Self {
        RecordCrc { kind: SumKind::Table, at, from, to, init, complement: false, copy: Some(copy), copy_clear: None }
    }

    pub fn compute(&self, image: &[u8]) -> u16 {
        let data = &image[self.from..self.to];
        match self.kind {
            SumKind::Ccitt => {
                let mut c = self.init as u16;
                for &x in data {
                    c ^= (x as u16) << 8;
                    for _ in 0..8 {
                        c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
                    }
                }
                if self.complement {
                    !c
                } else {
                    c
                }
            }
            SumKind::S32a => {
                let mut r = self.init;
                for &x in data {
                    r = (r & 0xffff_ff00).wrapping_add(x as u32);
                    for _ in 0..8 {
                        r = if r & 0x8000_0000 == 0 { r << 1 } else { (r << 1).wrapping_add(0x1021_0000) };
                    }
                }
                (r >> 16) as u16
            }
            SumKind::Table => {
                let table = |i: u32| -> u32 {
                    let mut c = (i & 0xff) << 8;
                    for _ in 0..8 {
                        c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
                    }
                    c & 0xffff
                };
                let mut r = self.init;
                for &x in data {
                    r = (r.wrapping_add(x as u32) << 8) ^ (table(r >> 24) << 16);
                }
                (table(r >> 24) ^ (r << 8 >> 16)) as u16
            }
            SumKind::ByteSum => data.iter().fold(0u16, |a, &x| a.wrapping_add(x as u16)),
        }
    }

    /// Whether the checksum stored in the image (and in the second copy, if the store has
    /// one) is the one the data gives: `Err` names the copy and both values.
    pub fn check(&self, image: &[u8]) -> Result<(), String> {
        let copies = 1 + self.copy.is_some() as usize;
        for c in 0..copies {
            let n = c * self.copy.unwrap_or(0);
            let me = RecordCrc { at: self.at + n, from: self.from + n, to: self.to + n, ..*self };
            let stored = u16::from_le_bytes([image[me.at], image[me.at + 1]]);
            let got = me.compute(image);
            if stored != got {
                return Err(format!("copy {c} (at {:#x}): stored {stored:#06x}, computed {got:#06x}", me.at));
            }
        }
        Ok(())
    }

    /// Store the checksum in the image (and rewrite the second copy from the first).
    pub fn fix(&self, image: &mut [u8]) {
        let c = self.compute(image);
        image[self.at] = c as u8;
        image[self.at + 1] = (c >> 8) as u8;
        if let Some(n) = self.copy {
            // The copy ends where the image does: a record whose second copy is its last
            // (`von`'s) is copied up to the image's end.
            let len = n.min(image.len().saturating_sub(self.at + n));
            image.copy_within(self.at..self.at + len, self.at + n);
            if let Some(m) = self.copy_clear {
                image[self.at + n + m] = 0;
                image[self.at + n + m + 1] = 0;
                let me = RecordCrc { at: self.at + n, from: self.from + n, to: self.to + n, ..*self };
                let c = me.compute(image);
                image[me.at] = c as u8;
                image[me.at + 1] = (c >> 8) as u8;
            }
        }
    }
}

impl LinkSettings {
    /// The store's image for a node: `base` (a settings file) or the
    /// defaults, the role byte set, `set` bytes (offset, value) applied, the
    /// checksum fixed.
    pub fn image(&self, base: Option<&[u8]>, role_off: usize, role: u8, set: &[(usize, u8)]) -> Vec<u8> {
        let mut e = vec![self.fill; self.size];
        // The record's own bytes under whatever the settings file has (its EEPROM only: a file
        // shorter than the image leaves the rest, the backup SRAM of `sgt24h`, as the record has it).
        e[..self.record.len()].copy_from_slice(self.record);
        if let Some(b) = base {
            e[..b.len().min(self.size)].copy_from_slice(&b[..b.len().min(self.size)]);
        }
        e[role_off] = role;
        for &(o, v) in set {
            if o < self.size {
                e[o] = v;
            }
        }
        self.crc.fix(&mut e);
        e
    }
}

/// A device on the board's expansion bus at `0x1D80000` that the game
/// checks at boot (`model2::protection`). The game sends it a block of
/// data (and, on [`ProtectionInterface::Tables`], two tables) and compares
/// the stream it reads back with bytes in its own program ROM. How the
/// device turns one into the other isn't known, so the board model answers
/// each request with those bytes, read from the ROM at run time. This
/// answers the check; it doesn't model what the device computes for
/// anything else.
#[derive(Clone, Copy, Debug)]
pub struct Protection {
    pub interface: ProtectionInterface,
    pub replies: &'static [ProtectionReply],
}

/// The registers the game drives its [`Protection`] device through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectionInterface {
    /// `0x1D80000-0x1D87FFF`: a command at `0x1D84000`, the block, two
    /// tables through `0x1D87FF4/6`, started at `0x1D87FF2`, the answer at
    /// `0x1D87FF8`, status `0xFFF0` at `0x1D8400C` (`doa`,
    /// docs/hw/model2b.md).
    Tables,
    /// The block in RAM at `0x1D80000`, then from `0x1D90000`: its
    /// halfword offset at `+8`, a key at `+0xC` that starts the device,
    /// the answer at `+0xE`, busy in bit 0 of `+0` (`dynamcop`,
    /// docs/hw/model2a.md; `zerogun`, docs/hw/model2b.md).
    Keyed,
}

/// One request the game makes of its [`Protection`] device, and the answer.
#[derive(Clone, Copy, Debug)]
pub struct ProtectionReply {
    /// The value the game writes to the command register (`0x1D84000`)
    /// before the request, or the key that starts it (`0x1D9000C`).
    pub command: u16,
    /// The halfword offset into the device's RAM the request names (the
    /// value written to `0x1D87FF2` or `0x1D90008`), where it tells
    /// requests apart: `zerogun` sends the same key with two pieces of its
    /// block. `None` matches any offset.
    pub offset: Option<u16>,
    /// Where the answer is in the program ROM (i960 address) and how many
    /// bytes long: the bytes the game compares the device's stream with.
    pub rom: u32,
    pub len: u32,
}

/// A data line of a polygon ROM chip that reads some of its 1 bits as 0
/// in the only dump there is (a bad dump; the hash is still the one
/// [`RomFile::sha1`] checks). Loading restores what can be restored: the
/// low half's link bits 8 and 9 of a polygon's attribute word, from their
/// copy in bits 23 and 24 (the high chip of the pair), which every intact
/// polygon ROM keeps equal ([`repair_link_bits`]). A float word that lost
/// the bit stays off by that mantissa bit (at most 2^-14 relative for bit
/// 9 of the low half).
#[derive(Clone, Copy, Debug)]
pub struct Dropout {
    /// The chip: the low half of a [`Role::Polygons`] pair.
    pub chip: &'static str,
    /// The data line, 8 or 9.
    pub bit: u8,
}

/// Model 2 board revisions. They share the i960 main CPU and sound system;
/// the geometry processor differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Model2,
    Model2A,
    Model2B,
    Model2C,
    /// Not yet confirmed from the program code.
    Unconfirmed,
}

impl Board {
    /// The board's name as Sega wrote it ("Model 2C"); "Model 2" for an
    /// unconfirmed revision.
    pub fn name(self) -> &'static str {
        match self {
            Board::Model2 | Board::Unconfirmed => "Model 2",
            Board::Model2A => "Model 2A",
            Board::Model2B => "Model 2B",
            Board::Model2C => "Model 2C",
        }
    }

    /// The boards whose geometry processor is an MB86234 TGP with an
    /// uploaded program, Model 2A's video RAM at `0x12000000` and a
    /// fixed-function geometrizer: 2A and, as far as `daytona` shows, the
    /// original Model 2 (docs/hw/model2.md).
    pub fn has_tgp_2a(self) -> bool {
        matches!(self, Board::Model2 | Board::Model2A)
    }
}

/// The broad sort of a game, for the launcher's filters (the finer one is
/// [`Game::genre`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Racing,
    Fighting,
    Shooting,
    Sports,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Racing, Kind::Fighting, Kind::Shooting, Kind::Sports];

    /// The filter's name.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Racing => "Racing",
            Kind::Fighting => "Fighting",
            Kind::Shooting => "Shooting",
            Kind::Sports => "Sports",
        }
    }
}

/// One chip in a set.
#[derive(Clone, Copy, Debug)]
pub struct RomFile {
    pub name: &'static str,
    pub size: usize,
    pub sha1: &'static str,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// i960 program ROM. Chips appear in pairs, 16-bit interleaved
    /// (first = low half-word).
    Program,
    /// i960-visible data ROM, 16-bit interleaved pairs, in 8 MB slots
    /// from `0x2000000`.
    MainData,
    /// The second bank of i960-visible data ROM, 16-bit interleaved pairs
    /// in 8 MB slots from `0x6000000` (a smaller pair repeats in its slot).
    /// segawski and waverunr (2C) and motoraid (2A) fill it; their program
    /// pointers land on the pairs' data there (docs/games/<set>.md for each).
    /// How the board decodes the rest of `0x4000000-0x7ffffff` is
    /// unconfirmed.
    MainDataHigh,
    /// Geometry coprocessor data ROM, 16-bit interleaved pairs.
    CoproData,
    /// Polygon model data read by the geometry engine, interleaved pairs.
    Polygons,
    /// Data ROM on the external bus of Model 2A's TGP (MB86234), 16-bit
    /// pairs (low half first) packed from address `0x800000` of that bus:
    /// the course data its collision functions read.
    TgpData,
    /// Texture ROM, interleaved pairs.
    Textures,
    /// 68000 sound program (byte-swapped 16-bit).
    AudioCpu,
    /// SCSP sample ROMs (byte-swapped 16-bit, concatenated).
    Samples,
    /// MPEG sound board CPU program (byte-swapped 16-bit).
    MpegCpu,
    /// MPEG sound board stream data, concatenated.
    MpegData,
    /// DSB1 music board Z80 program (plain bytes). The MPEG ROMs beside
    /// it are `MpegData`.
    Dsb1Cpu,
    /// The original Model 2 sound board's first MultiPCM sample ROMs
    /// (plain bytes, in order, the one with the header table first, each in
    /// a 2 MB socket: [`RomSet::multipcm`]). The 68000 program beside it is
    /// `AudioCpu`.
    MultiPcmA,
    /// The second MultiPCM's sample ROMs, likewise.
    MultiPcmB,
    /// Not yet identified.
    Unknown,
}

/// Model 2C's ROM banks give each chip pair an 8 MB slot.
pub const ROM_SLOT: usize = 0x80_0000;

/// One named moment of a game: a recipe, never game data. The same kind of
/// fact as the manifest's `coin-start` script and `attract_frame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scene {
    /// Lower-case slug: `[a-z0-9-]`.
    pub name: &'static str,
    pub title: &'static str,
    /// An input script (`romset::Controls`' grammar; `coin-start` allowed).
    pub input: &'static str,
    /// The frame to stop and keep, counted from power-on with a fresh EEPROM.
    pub frame: u64,
    /// From the fixed list: attract, menu, select, play, result, hud.
    pub tags: &'static [&'static str],
}

#[derive(Clone)]
pub struct Game {
    /// Short set name, matching the usual dump naming (e.g. `topskatr`); a
    /// variant's own id (`topskatrj`, resolved from its manifest: [`variants`]).
    pub id: &'static str,
    pub title: &'static str,
    /// Year of release, as on the title screen's copyright line.
    pub year: u16,
    /// What sort of game it is, in a few words ("Rally", "Light gun"), as
    /// the launcher shows it.
    pub genre: &'static str,
    /// The launcher's filter it falls under.
    pub kind: Kind,
    pub board: Board,
    /// A frame of the attract mode (counted from boot, no inputs) that shows
    /// the game well: the browser build's game card screenshot
    /// (`remodel shot`).
    pub attract_frame: u64,
    /// The game's named scenes: recipes (an input script and a frame) that
    /// `remodel scene make` runs once to keep a frame's captured inputs
    /// (docs/uplift-studio.md, "Scenes"). Small and optional; a project's
    /// `scenes.txt` adds to them and wins on a name.
    pub scenes: &'static [Scene],
    /// The game's own input lag in play, measured: frames after the first
    /// that reads a control before the picture shows it (`make input-lag`,
    /// docs/games/<set>.md; the least seen, for the control
    /// measured). Run-ahead of more frames than this only judders, so the
    /// player suggests up to this much (`docs/play.md`, "Run-ahead").
    pub input_lag: u8,
    /// The most frames of run-ahead worth offering: [`Game::input_lag`]
    /// at most 3, or more where one control has lag and the main one has
    /// none (`schamp`, `lastbrnx`: punches 1, the stick 0, which judders).
    /// 0: nothing to hide, so the player offers no run-ahead
    /// (`docs/play.md`, "Run-ahead").
    pub run_ahead_max: u8,
    pub files: &'static [RomFile],
    /// Banks whose chip pairs follow each other with no gap, rather than
    /// each in its own [`ROM_SLOT`]: bel's main data and polygon ROMs, of
    /// 16 Mbit chips, sit every 4 MB (its textures still take 8 MB slots).
    pub packed: &'static [Role],
    /// Output level in dB on top of the player's mix, standing in for the
    /// cabinet amplifier's volume pot so games play at similar loudness.
    /// A listening choice: the ROMs don't set the analog gain.
    pub volume_db: f32,
    /// 3D viewports (window numbers) that hold HUD elements rather than the
    /// scene. Only enhancements use this: with the HUD at the edges of a
    /// wide view, each polygon in one moves by the third of the screen it
    /// is in, like the 2D HUD elements.
    pub hud_windows: &'static [usize],
    /// 3D viewports that hold HUD parts among other 3D (a rev counter's
    /// needle in a window that elsewhere holds cars in panels). Only
    /// enhancements use this: with the HUD at the edges of a wide view, a
    /// polygon in one moves with the 2D HUD element whose box contains the
    /// polygon's centre (a needle with its dial), and stays put if none
    /// does.
    pub hud_element_windows: &'static [usize],
    /// The cabinet's controls, where the board reads them, and default
    /// bindings.
    pub controls: Controls,
    /// A drive (motion or force feedback) board the game needs, if any.
    pub drive_board: Option<DriveBoard>,
    /// A communication (link) board the game needs fitted, if any; none
    /// means the slot is empty and reads as no board.
    pub link_board: Option<LinkBoard>,
    /// The set this one is a clone of (another revision of the same game):
    /// its chips are also looked for in the parent's folder or zip, as a
    /// merged MAME set keeps them, and it shares the parent's poster and
    /// bezel.
    pub parent: Option<&'static str>,
    /// Settings the game keeps in its battery-backed SRAM (`0x1D00000`)
    /// and never initialises itself, as (offset, bytes): what an operator
    /// would have chosen in the test menu. Written over the SRAM's power-on
    /// contents (all ones) at boot. sgt24h reads its I/O board type from
    /// there and indexes a table with it unchecked.
    pub backup_ram: &'static [(usize, &'static [u8])],
    /// The protection device the game checks for at `0x1D80000`, if any.
    pub protection: Option<Protection>,
    /// Known bad data lines in the set's dump, repaired at load.
    pub dropouts: &'static [Dropout],
    /// Texture RAM banks of 1 MB, each repeating through its 2 MB window
    /// at `0x11000000` and `0x11200000`: fvipers, schamp and vstriker
    /// upload their texels to the upper half of each window, which the
    /// rasteriser reads as the lower. Other boards keep 2 MB there (overrevb and sgt24h fill
    /// both halves with different data), of which the rasteriser reads the
    /// lower (docs/games/fvipers.md).
    pub texture_ram_mirrored: bool,
    /// Rules for smooth shading (an enhancement, off by default:
    /// `model2::light`); [`Shading::DEFAULT`] unless the game's pictures
    /// asked for something else.
    pub shading: Shading,
    /// What the player's "Enhanced" picture preset turns on for this game
    /// ([`Recommended::DEFAULT`] unless the game's pictures said
    /// otherwise; the evidence in `docs/games/<set>.md`).
    pub recommended: Recommended,
    /// Poll loops the main program waits in, which the board model may
    /// fast-forward through exactly (`model2::idle`, docs/idle-skip.md),
    /// each found and checked in the game's code (docs/games/<set>.md).
    /// Empty: nothing is skipped.
    pub idle_loops: &'static [IdleLoop],
}

/// A loop the i960 polls in until something it reads changes: its
/// instructions from `start` (where the loop branches back to) up to
/// `end`, just past the backward branch. The board model checks every
/// pass before it skips more (no writes, reads that can't change before
/// the next event, the CPU back in the same state but for counters), so
/// a wrong entry costs speed, not correctness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleLoop {
    pub start: u32,
    pub end: u32,
}

impl Game {
    /// Frames of run-ahead the "Enhanced" preset sets: the game's own
    /// input lag, within [`Game::run_ahead_max`]. A game whose main
    /// control has no lag gets none, however much a button has.
    pub fn run_ahead_recommended(&self) -> u8 {
        self.input_lag.min(self.run_ahead_max)
    }
}

/// Per-game choices of the player's "Enhanced" picture preset (smooth
/// motion and run-ahead aside: those follow [`Game::input_lag`]). Each is
/// on only where the game's own documentation says it is right for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recommended {
    /// The depth buffer: seen fixing pictures in the game's frames and
    /// breaking none (`docs/enhancements.md`, "Depth buffer").
    pub depth: bool,
    /// Smooth shading (vertex) with the game's crease: curved bodies and
    /// fighters look better (`docs/enhancements.md`, "Smooth shading").
    pub shading: bool,
    /// Temporal anti-aliasing at full size; false: 4x multisampling
    /// instead, where temporal misbehaves (`vstriker`).
    pub temporal: bool,
}

impl Recommended {
    /// What a game without evidence gets: temporal anti-aliasing, nothing
    /// else that changes what the game draws.
    pub const DEFAULT: Recommended = Recommended { depth: false, shading: false, temporal: true };
}

/// Per-game rules for smooth shading (`model2::light`, an enhancement):
/// which polygons it may smooth and how sharp an edge stays sharp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shading {
    /// Faces meeting at more than this many degrees keep a hard edge.
    pub crease: u8,
    /// 3D windows never smooth-shaded (besides [`Game::hud_windows`]).
    pub skip_windows: &'static [usize],
    /// Models never smooth-shaded: address ranges (first, last). Needs the
    /// lighting side channel (not built yet): ignored without it.
    pub skip_models: &'static [(u32, u32)],
    /// Models with a crease angle of their own: (first, last, degrees).
    /// Needs the side channel, as `skip_models`.
    pub model_crease: &'static [(u32, u32, u8)],
    /// Checkerboard (screen door) polygons stay flat.
    pub skip_checker: bool,
    /// Welding tolerance, relative to the position (0: exact bits).
    /// Reserved: only exact welding is implemented.
    pub weld_eps: f32,
    /// Reserved: models whose consecutive object blocks are welded
    /// together. Not implemented.
    pub weld_models: &'static [(u32, u32)],
}

impl Shading {
    /// What every game uses until its pictures say otherwise: a 40 degree
    /// crease (boxes, 8-sided wheels and limbs stay faceted; bodies and
    /// 10+-sided cylinders smooth), nothing else excluded. Checked on all
    /// 18 working games' frames (docs/enhancements.md, "Smooth shading");
    /// a new game starts with it and needs nothing else: the generic code
    /// reads only these fields, never a game's identity. To give a game
    /// its own, write `shading: crate::Shading { crease: N,
    /// ..crate::Shading::DEFAULT }` in its manifest with a one-line reason,
    /// and the pictures behind it in `docs/games/<set>.md`.
    pub const DEFAULT: Shading = Shading {
        crease: 40,
        skip_windows: &[],
        skip_models: &[],
        model_crease: &[],
        skip_checker: false,
        weld_eps: 0.0,
        weld_models: &[],
    };
}

#[derive(Debug)]
pub enum Error {
    UnknownGame(String),
    Io(PathBuf, std::io::Error),
    Size {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    /// A chip whose SHA-1 is not the known-good dump's, and no file in the
    /// ROM folder has the right one. `file` is where it was read from (empty for
    /// chips handed over in memory).
    Hash {
        name: &'static str,
        file: String,
        expected: &'static str,
        actual: String,
    },
    /// A chip found in none of the places a set can be.
    Missing {
        name: &'static str,
        root: PathBuf,
        set: &'static str,
    },
    Zip(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownGame(id) => {
                let known: Vec<_> = games::ALL.iter().map(|g| g.id).collect();
                write!(f, "unknown game '{id}' (known: {})", known.join(", "))
            }
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Size { name, expected, actual } => {
                write!(f, "{name}: expected {expected:#x} bytes, found {actual:#x}")
            }
            Error::Hash { name, file, expected, actual } if file.is_empty() => {
                write!(f, "{name}: sha1 {actual} does not match known-good {expected}")
            }
            Error::Hash { name, file, expected, actual } => {
                write!(f, "{name}: {file} has sha1 {actual}, not the known-good {expected}, and no file in the ROM folder has it")
            }
            Error::Missing { name, root, set } => {
                let r = root.display();
                write!(f, "{name}: not in {r}/{set}/, {r}/{set}.zip or {r}/")
            }
            Error::Zip(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A loaded, assembled ROM set.
pub struct RomSet {
    pub game: &'static Game,
    /// i960 program space starting at address 0.
    pub program: Vec<u8>,
    /// Raw chip images, in manifest order.
    pub chips: Vec<Vec<u8>>,
}

/// Where a set's chips can be in a ROM directory: its own folder and zip,
/// then its parent's, then the directory itself. This is where a chip is
/// looked for by its name; [`Locator`] looks by content.
struct Places {
    root: PathBuf,
    list: Vec<(PathBuf, Option<Arc<zip::Zip>>)>,
}

impl Places {
    fn open(root: &Path, game: &Game) -> Result<Self, Error> {
        let mut list = Vec::new();
        for set in std::iter::once(game.id).chain(game.parent) {
            let zip_path = root.join(format!("{set}.zip"));
            let archive =
                if zip_path.is_file() { Some(Arc::new(zip::Zip::open(&zip_path).map_err(Error::Zip)?)) } else { None };
            list.push((root.join(set), archive));
        }
        Ok(Places { root: root.to_path_buf(), list })
    }

    /// The chip `name` of set `id`: `<id>/<name>` in each place first, then
    /// the bare name in each place, then loose in the ROM directory. Nothing
    /// is read.
    fn find(&self, id: &str, name: &str) -> Option<Candidate> {
        let prefixed = format!("{id}/{name}");
        for n in [prefixed.as_str(), name] {
            for (dir, archive) in &self.list {
                if let Some(c) = Candidate::file(dir.join(n)) {
                    return Some(c);
                }
                if let Some(c) = archive.as_ref().and_then(|z| Candidate::member(z, n)) {
                    return Some(c);
                }
            }
        }
        Candidate::file(self.root.join(name))
    }

    /// Whether [`Places::find`] finds the chip.
    fn has(&self, id: &str, name: &str) -> bool {
        self.find(id, name).is_some()
    }
}

/// Print `warning` to stderr the first time this process has it (a player
/// loads a set several times).
fn warn_once(warning: &str) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.iter().any(|w| w == warning) {
        eprintln!("warning: {warning}");
        seen.push(warning.to_string());
    }
}

impl RomSet {
    /// Load `game`'s chips from ROM directory `root`. Each chip is taken by
    /// its SHA-1 first: from where its name says (`root/<set>/`, else
    /// `root/<set>.zip`, then for a variant or clone the same for its parent
    /// set, else `root/` itself; a chip is looked for as `<set id>/<name>`,
    /// a merged set keeps same-named chips of several variants in folders,
    /// `topskatrj/epr-19753.15`, before its bare name), and if that is
    /// missing or has another SHA-1, from any file or zip member under
    /// `root` that has the right one, whatever it is called (MAME's naming
    /// differs by set and version). A chip no file has the right SHA-1 for
    /// is taken from where its name says if it has the size (a warning on
    /// stderr, once, names the chip, the file and both hashes); with `verify`
    /// that is an error. A chip with no hash in its manifest is taken by name
    /// only. Hashes are remembered in the user's cache ([`HashCache::user`]),
    /// so a set is not hashed again until a file changes.
    pub fn load(game: &'static Game, root: &Path, verify: bool) -> Result<Self, Error> {
        let mut cache = HashCache::user();
        let r = Self::load_with(game, root, verify, &mut cache);
        cache.save();
        let (set, warnings) = r?;
        for w in &warnings {
            warn_once(w);
        }
        Ok(set)
    }

    /// [`RomSet::load`] with the hash cache given, and the warnings returned rather than printed.
    pub fn load_with(
        game: &'static Game,
        root: &Path,
        verify: bool,
        cache: &mut HashCache,
    ) -> Result<(Self, Vec<String>), Error> {
        let places = Places::open(root, game)?;
        let mut locator = Locator::new(root);
        let mut chips = Vec::with_capacity(game.files.len());
        let mut warnings = Vec::new();
        for rom in game.files {
            let named = places.find(game.id, rom.name);
            let hashed = !rom.sha1.is_empty();
            // The named file's SHA-1 if it could be had; it is right if it is the manifest's.
            let named_sha1 = named.as_ref().filter(|c| hashed && c.size == rom.size as u64).map(|c| c.sha1(cache));
            let named_good = match (&named, &named_sha1) {
                (Some(_), _) if !hashed => true,
                (Some(_), Some(Ok(h))) => *h == rom.sha1,
                _ => false,
            };
            if named_good {
                chips.push(named.as_ref().unwrap().read().map_err(Error::Zip)?);
                continue;
            }
            if hashed {
                if let Some((_, data)) = locator.find(rom, cache, true) {
                    chips.push(data);
                    continue;
                }
            }
            let Some(c) = named else {
                return Err(Error::Missing { name: rom.name, root: root.to_path_buf(), set: game.id });
            };
            let data = c.read().map_err(Error::Zip)?;
            if let Some(Ok(actual)) = named_sha1 {
                if verify {
                    return Err(Error::Hash { name: rom.name, file: c.label(), expected: rom.sha1, actual });
                }
                warnings.push(format!(
                    "{}: {} has sha1 {actual}, not the known-good {}, and no file in {} has it; using it anyway",
                    rom.name,
                    c.label(),
                    rom.sha1,
                    root.display()
                ));
            }
            chips.push(data);
        }
        Ok((Self::from_chips(game, chips, verify)?, warnings))
    }

    /// Assemble a set from chip images already in memory, in manifest
    /// order (e.g. fetched by a browser build).
    pub fn from_chips(game: &'static Game, chips: Vec<Vec<u8>>, verify: bool) -> Result<Self, Error> {
        for (rom, data) in game.files.iter().zip(&chips) {
            if data.len() != rom.size {
                return Err(Error::Size { name: rom.name, expected: rom.size, actual: data.len() });
            }
            if verify {
                let actual = sha1_hex(data);
                if actual != rom.sha1 {
                    return Err(Error::Hash { name: rom.name, file: String::new(), expected: rom.sha1, actual });
                }
            }
        }
        let mut chips = chips;
        for d in game.dropouts {
            let (lo, hi) =
                polygon_pair(game, d.chip).unwrap_or_else(|| panic!("{}: {} is no polygon low half", game.id, d.chip));
            let [lo_chip, hi_chip] = chips.get_disjoint_mut([lo, hi]).expect("distinct chips");
            repair_link_bits(lo_chip, hi_chip, d.bit);
        }
        let program = assemble(game, &chips, Role::Program);
        Ok(RomSet { game, program, chips })
    }

    /// Assemble a 32-bit region from its interleaved chip pairs.
    pub fn region32(&self, role: Role) -> Vec<u8> {
        assemble(self.game, &self.chips, role)
    }

    /// `region32`, with each pair in its own `slot` bytes: a pair smaller
    /// than its slot repeats to fill it, as a board decoding fewer address
    /// lines than the slot has. Model 2C's ROM banks are 8 MB slots
    /// (stcc's 1 MB main data pair is read at `0x2C4B210`, 4 MB into the
    /// second slot).
    pub fn region32_slots(&self, role: Role, slot: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let idx: Vec<usize> =
            self.game.files.iter().enumerate().filter(|(_, f)| f.role == role).map(|(i, _)| i).collect();
        for &[lo, hi] in idx.as_chunks::<2>().0 {
            let pair = interleave16(&self.chips[lo], &self.chips[hi]);
            if pair.is_empty() {
                continue;
            }
            let start = out.len();
            while out.len() - start < slot {
                let n = pair.len().min(slot - (out.len() - start));
                out.extend_from_slice(&pair[..n]);
            }
        }
        out
    }

    /// The bank of chips with `role` as the board maps it: pairs in
    /// [`ROM_SLOT`]s, or back to back for a bank the manifest lists as
    /// packed.
    pub fn bank(&self, role: Role) -> Vec<u8> {
        if self.game.packed.contains(&role) {
            self.region32(role)
        } else {
            self.region32_slots(role, ROM_SLOT)
        }
    }

    /// Concatenate the chips with `role` (16-bit, stored little-endian per
    /// word) and swap each byte pair, giving a 68000's big-endian view.
    pub fn region16be(&self, role: Role) -> Vec<u8> {
        let mut out = Vec::new();
        for (f, c) in self.game.files.iter().zip(&self.chips) {
            if f.role == role {
                out.extend(c.as_chunks::<2>().0.iter().flat_map(|&[a, b]| [b, a]));
            }
        }
        out
    }

    /// A MultiPCM's sample ROMs on the original Model 2 sound board
    /// ([`Role::MultiPcmA`], [`Role::MultiPcmB`]), in manifest order, each
    /// in its own 2 MB socket: a 1 MB chip repeats through its socket.
    /// `vcop`'s first chip has two 1 MB ROMs and selects its second with
    /// bank 2, the MB where a second 2 MB chip starts (`daytona`'s 2 MB
    /// chips fill their sockets; docs/hw/multipcm.md).
    pub fn multipcm(&self, role: Role) -> Vec<u8> {
        const SOCKET: usize = 0x20_0000;
        let mut out = Vec::new();
        for (_, c) in self.game.files.iter().zip(&self.chips).filter(|(f, c)| f.role == role && !c.is_empty()) {
            let start = out.len();
            while out.len() - start < SOCKET {
                let n = c.len().min(SOCKET - (out.len() - start));
                out.extend_from_slice(&c[..n]);
            }
        }
        out
    }

    pub fn chip(&self, name: &str) -> Option<&[u8]> {
        self.game.files.iter().position(|r| r.name == name).map(|i| self.chips[i].as_slice())
    }
}

/// The set folder in ROM directory `root` that holds every chip of `game`
/// unpacked, at its size: `root/<set>/`, else (for a clone) its parent's,
/// as a merged MAME set keeps them. The browser build fetches chips from
/// there.
pub fn rom_folder(game: &Game, root: &Path) -> Option<&'static str> {
    std::iter::once(game.id).chain(game.parent).find(|set| {
        game.files.iter().all(|f| fs::metadata(root.join(set).join(f.name)).is_ok_and(|m| m.len() == f.size as u64))
    })
}

/// The sets whose texture pack serves `game`, in the order to look for one
/// (docs/texture-packs.md): its own, then for a clone its parent's (not for a
/// variant with textures of its own, `dayton93`). Pack
/// keys are the textures' content, so a parent's images serve every
/// version. The native player looks for `<set>/texpack/` directories in
/// this order, the browser for `/texpack/<set>/index.txt`.
pub fn pack_sets(game: &Game) -> impl Iterator<Item = &'static str> {
    let own = variants::get(game.id).is_some_and(|v| v.own_textures);
    std::iter::once(game.id).chain(game.parent.filter(|_| !own))
}

/// Set names in ROM directory `root`: `<set>.zip` files and directories.
pub fn sets_in(root: &Path) -> Vec<String> {
    let mut sets: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let kind = e.file_type().ok()?;
            match name.strip_suffix(".zip") {
                Some(set) if kind.is_file() => Some(set.to_string()),
                None if kind.is_dir() => Some(name),
                _ => None,
            }
        })
        .collect();
    sets.sort();
    sets.dedup();
    sets
}

/// [`sets_in`], with each of a set's variants after it when the variant has
/// no zip or folder of its own and every chip it swaps is there (a merged
/// MAME set keeps a variant's chips in its parent's): a variant whose ROMs
/// are missing is not listed. A variant that has its own zip or folder is
/// listed as the set it is, wherever it sits. A game with no zip or folder
/// of its name is listed too when files under `root` have the SHA-1 of every
/// one of its chips, however the set is laid out and the chips are named
/// ([`RomSet::load`]).
pub fn sets_with_clones(root: &Path) -> Vec<String> {
    let mut cache = HashCache::user();
    let out = sets_with_clones_in(root, &mut cache);
    cache.save();
    out
}

/// [`sets_with_clones`] with the hash cache given.
pub fn sets_with_clones_in(root: &Path, cache: &mut HashCache) -> Vec<String> {
    let mut out = Vec::new();
    let sets = sets_in(root);
    let mut locator = Locator::new(root);
    for set in &sets {
        out.push(set.clone());
        if let Ok(parent) = games::find(set) {
            push_variants(root, parent, &sets, &mut locator, cache, &mut out);
        }
    }
    for game in games::ALL {
        if !sets.iter().any(|s| s == game.id) && has_every_chip(game, &mut locator, cache) {
            out.push(game.id.to_string());
            push_variants(root, game, &sets, &mut locator, cache, &mut out);
        }
    }
    out
}

/// The variants of `parent` that play, have no set of their own in `sets` and whose own chips
/// are there (by name, else by content), onto `out`.
fn push_variants(
    root: &Path,
    parent: &'static Game,
    sets: &[String],
    locator: &mut Locator,
    cache: &mut HashCache,
    out: &mut Vec<String>,
) {
    let own: Vec<&Variant> = variants::of(parent.id).filter(|v| v.plays && !sets.iter().any(|s| s == v.id)).collect();
    if own.is_empty() {
        return;
    }
    let places = Places::open(root, parent);
    for v in own {
        let Some(game) = variants::resolve(v) else { continue };
        let present = v.own_chips(game).all(|f| {
            places.as_ref().is_ok_and(|p| p.has(v.id, f.name))
                || (!f.sha1.is_empty() && locator.find(f, cache, false).is_some())
        });
        if present {
            out.push(v.id.to_string());
        }
    }
}

/// Whether files under the locator's folder have the SHA-1 of each of `game`'s chips.
fn has_every_chip(game: &Game, locator: &mut Locator, cache: &mut HashCache) -> bool {
    if game.files.iter().any(|f| f.sha1.is_empty()) {
        return false;
    }
    // The chips whose size is rarest first: a game that is not there is found out soonest.
    let mut chips: Vec<(usize, &RomFile)> = game.files.iter().map(|f| (locator.count(f.size), f)).collect();
    chips.sort_by_key(|c| c.0);
    chips.iter().all(|&(n, f)| n > 0 && locator.find(f, cache, false).is_some())
}

/// Whether every chip `variant` swaps in is in ROM directory `root`
/// (its own folder or zip, else the parent's, else by content).
pub fn variant_available(variant: &'static Variant, root: &Path) -> bool {
    let Some(game) = variants::resolve(variant) else { return false };
    let places = Places::open(root, game);
    let mut cache = HashCache::user();
    let mut locator = Locator::new(root);
    let found = variant.own_chips(game).all(|f| {
        places.as_ref().is_ok_and(|p| p.has(variant.id, f.name))
            || (!f.sha1.is_empty() && locator.find(f, &mut cache, false).is_some())
    });
    cache.save();
    found
}

/// Concatenate the interleaved pairs of chips with `role`, in manifest order.
fn assemble(game: &Game, chips: &[Vec<u8>], role: Role) -> Vec<u8> {
    let idx: Vec<usize> = game.files.iter().enumerate().filter(|(_, f)| f.role == role).map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for &[lo, hi] in idx.as_chunks::<2>().0 {
        out.extend(interleave16(&chips[lo], &chips[hi]));
    }
    out
}

/// Combine two 16-bit-wide chips into one 32-bit little-endian bus:
/// `lo` supplies bytes 0-1 of each word, `hi` bytes 2-3.
/// The manifest indices of `chip` and its partner, if `chip` is the low
/// half of a [`Role::Polygons`] pair.
fn polygon_pair(game: &Game, chip: &str) -> Option<(usize, usize)> {
    let idx: Vec<usize> =
        game.files.iter().enumerate().filter(|(_, f)| f.role == Role::Polygons).map(|(i, _)| i).collect();
    idx.as_chunks::<2>().0.iter().find(|&&[lo, _]| game.files[lo].name == chip).map(|&[lo, hi]| (lo, hi))
}

/// Restore link bit `bit` (8 or 9) of the polygon attribute words in a
/// pair's low chip `lo` from its copy 15 bits up, in the high chip `hi`.
/// An attribute word has a polygon type in bits 0-1 and bits 2-7 and 25-26
/// clear; its bits 23-24 repeat the link bits 8-9 (in every intact Model 2
/// polygon ROM here; `docs/hw/geometrizer.md` §3.3). The bit is set where
/// it reads 0, its copy 1, and the other link bit agrees with its own
/// copy. A float word that happens to look like that gets one mantissa bit
/// of its low half set, a change of at most 2^-14 relative, the size of
/// the error the dropout itself leaves in floats. Returns how many words
/// changed.
pub fn repair_link_bits(lo: &mut [u8], hi: &[u8], bit: u8) -> usize {
    assert!(bit == 8 || bit == 9, "only link bits 8 and 9 have a copy");
    let other = 17 - bit as u32;
    let mut n = 0;
    for (l, h) in lo.as_chunks_mut::<2>().0.iter_mut().zip(hi.as_chunks::<2>().0) {
        let w = u16::from_le_bytes(*l) as u32 | (u16::from_le_bytes(*h) as u32) << 16;
        let attr = w & 3 != 0 && w >> 2 & 0x3F == 0 && w >> 25 & 3 == 0;
        let lost = w >> bit & 1 == 0 && w >> (bit + 15) & 1 == 1;
        if attr && lost && w >> other & 1 == w >> (other + 15) & 1 {
            *l = (w as u16 | 1 << bit).to_le_bytes();
            n += 1;
        }
    }
    n
}

pub fn interleave16(lo: &[u8], hi: &[u8]) -> Vec<u8> {
    assert_eq!(lo.len(), hi.len());
    let mut out = Vec::with_capacity(lo.len() * 2);
    for (l, h) in lo.as_chunks::<2>().0.iter().zip(hi.as_chunks::<2>().0) {
        out.extend_from_slice(l);
        out.extend_from_slice(h);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ahead_is_offered_up_to_three_frames() {
        for g in games::ALL {
            assert!(g.run_ahead_max <= 3, "{}", g.id);
            // Never past what the game's lag gives, except where a button
            // has some and the main control none.
            assert!(g.run_ahead_max <= g.input_lag.max(1) || g.run_ahead_max == 0, "{}", g.id);
            assert!(g.run_ahead_recommended() <= g.run_ahead_max);
        }
        let ra = |id| games::find(id).ok().map(|g| (g.run_ahead_max, g.run_ahead_recommended()));
        assert_eq!(ra("sgt24h"), Some((3, 3)));
        assert_eq!(ra("schamp"), Some((1, 0)));
        assert_eq!(ra("lastbrnx"), Some((1, 0)));
        assert_eq!(ra("topskatr"), Some((0, 0)));
        assert_eq!(ra("stcc"), Some((1, 1)));
    }

    #[test]
    fn clones_fall_back_to_the_parents_pack() {
        let sets = |id| pack_sets(games::find(id).unwrap()).collect::<Vec<_>>();
        assert_eq!(sets("overrevb"), ["overrevb", "overrev"]);
        assert_eq!(sets("topskatr"), ["topskatr"]);
    }

    #[test]
    fn interleave_order() {
        assert_eq!(interleave16(&[1, 2, 5, 6], &[3, 4, 7, 8]), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    /// Halves of a 32-bit word as two chips' little-endian 16-bit words.
    fn halves(words: &[u32]) -> (Vec<u8>, Vec<u8>) {
        let lo = words.iter().flat_map(|w| (*w as u16).to_le_bytes()).collect();
        let hi = words.iter().flat_map(|w| ((w >> 16) as u16).to_le_bytes()).collect();
        (lo, hi)
    }

    #[test]
    fn link_bit_repair() {
        let words = [
            0x7884_0902, // link 1, copy 1: intact
            0x7984_0902, // link 1, copy 3: bit 9 lost -> 0x7984_0b02
            0x7904_0801, // link 0, copy 2: bit 9 lost -> 0x7904_0a01
            0x7880_0902, // link 1, copy 1 with bit 23 clear: copies disagree on bit 8, left alone
            0x3f80_0001, // 1.0000001: bits 25-26 set, not an attribute word
            0x7984_09c2, // bits 2-7 not clear: not an attribute word
        ];
        let (mut lo, hi) = halves(&words);
        assert_eq!(repair_link_bits(&mut lo, &hi, 9), 2);
        let got: Vec<u32> = lo
            .as_chunks::<2>()
            .0
            .iter()
            .zip(hi.as_chunks::<2>().0)
            .map(|(l, h)| u16::from_le_bytes(*l) as u32 | (u16::from_le_bytes(*h) as u32) << 16)
            .collect();
        assert_eq!(got, [0x7884_0902, 0x7984_0b02, 0x7904_0a01, 0x7880_0902, 0x3f80_0001, 0x7984_09c2]);
    }

    #[test]
    fn dropouts_name_polygon_low_halves() {
        for g in games::ALL {
            for d in g.dropouts {
                assert!(polygon_pair(g, d.chip).is_some(), "{}: {} is no polygon low half", g.id, d.chip);
                assert!(d.bit == 8 || d.bit == 9, "{}: only link bits can be restored", g.id);
            }
        }
    }

    #[test]
    fn srallyc_samples_fill_the_sound_board_window() {
        // The sound programs' sample tables fill 0x800000-0xFFFFFF, `.31` first
        // and `.37` last (docs/games/srallyc.md, vf2.md).
        for g in games::ALL.iter().chain(games::IN_PROGRESS).filter(|g| ["srallyc", "vf2"].contains(&g.id)) {
            let s: Vec<_> = g.files.iter().filter(|f| f.role == Role::Samples).collect();
            assert_eq!(s.iter().map(|f| f.size).sum::<usize>(), 0x80_0000, "{}", g.id);
            assert!(s[0].name.ends_with(".31") && s[3].name.ends_with(".37"), "{}", g.id);
        }
    }

    #[test]
    fn manifests_have_program_pairs() {
        for g in games::ALL.iter().chain(games::IN_PROGRESS) {
            for role in [
                Role::Program,
                Role::MainData,
                Role::MainDataHigh,
                Role::CoproData,
                Role::Polygons,
                Role::Textures,
                Role::TgpData,
            ] {
                let n = g.files.iter().filter(|f| f.role == role).count();
                assert!(n % 2 == 0, "{}: {role:?} chips must come in pairs", g.id);
            }
        }
    }
}

#[cfg(test)]
mod locate_tests {
    //! Finding chips by content, with made-up chips (no ROM bytes anywhere).
    use super::*;
    use crate::zip::tests::archive_of;

    fn chip(seed: u8) -> Vec<u8> {
        (0..64u32).map(|i| (i as u8).wrapping_mul(seed | 1).wrapping_add(seed)).collect()
    }

    /// A game of four program chips (`hashed`: with their SHA-1s) on hotd's other settings.
    fn game(
        id: &'static str,
        parent: Option<&'static str>,
        names: [&'static str; 4],
        seeds: [u8; 4],
        hashed: bool,
    ) -> &'static Game {
        let files: Vec<RomFile> = names
            .iter()
            .zip(seeds)
            .map(|(&name, s)| RomFile {
                name,
                size: 64,
                sha1: if hashed { Box::leak(sha1_hex(&chip(s)).into_boxed_str()) } else { "" },
                role: Role::Program,
            })
            .collect();
        let mut g = games::find("hotd").unwrap().clone();
        g.id = id;
        g.parent = parent;
        g.files = Box::leak(files.into_boxed_slice());
        Box::leak(Box::new(g))
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("remodel-locate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn load(g: &'static Game, root: &Path, verify: bool) -> Result<(RomSet, Vec<String>), Error> {
        RomSet::load_with(g, root, verify, &mut HashCache::memory())
    }

    fn expect(set: &RomSet, seeds: [u8; 4]) {
        for (c, s) in set.chips.iter().zip(seeds) {
            assert_eq!(*c, chip(s));
        }
    }

    const SEEDS: [u8; 4] = [3, 5, 7, 9];
    const NAMES: [&str; 4] = ["a.1", "a.2", "a.3", "a.4"];

    #[test]
    fn renamed_members_of_a_zip_are_found() {
        let g = game("t_renamed", None, NAMES, SEEDS, true);
        let root = tmp("renamed");
        // Not the set's name, not the chips' names, and in a different order.
        let members: Vec<(String, Vec<u8>)> =
            [3, 1, 0, 2].iter().map(|&i| (format!("x/{i:02}.bin"), chip(SEEDS[i]))).collect();
        let refs: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        for method in [0, 8] {
            fs::write(root.join("whatever.zip"), archive_of(&refs, method)).unwrap();
            let (set, warnings) = load(g, &root, true).unwrap();
            expect(&set, SEEDS);
            assert!(warnings.is_empty());
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn loose_files_in_subfolders_are_found() {
        let g = game("t_loose", None, NAMES, SEEDS, true);
        let root = tmp("loose");
        fs::create_dir_all(root.join("dump/one/two")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        fs::write(root.join("dump/one/two/p.bin"), chip(SEEDS[0])).unwrap();
        fs::write(root.join("dump/one/q.bin"), chip(SEEDS[1])).unwrap();
        fs::write(root.join("other/r"), chip(SEEDS[2])).unwrap();
        fs::write(root.join("s.rom"), chip(SEEDS[3])).unwrap();
        fs::write(root.join("other/same-size-other-bytes"), chip(42)).unwrap();
        let (set, _) = load(g, &root, true).unwrap();
        expect(&set, SEEDS);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_split_clone_finds_its_chips_in_its_own_and_its_parents_zip() {
        let seeds = [11, 13, 15, 17];
        let kid = game("t_kid", Some("t_par"), ["k.1", "k.2", "k.3", "k.4"], seeds, true);
        let root = tmp("split");
        // The clone's zip has its own two chips (renamed), the parent's holds the other two (renamed).
        fs::write(root.join("t_kid.zip"), archive_of(&[("zz", &chip(seeds[0])), ("yy", &chip(seeds[1]))], 8)).unwrap();
        fs::write(root.join("t_par.zip"), archive_of(&[("qq", &chip(seeds[2])), ("pp", &chip(seeds[3]))], 8)).unwrap();
        let (set, warnings) = load(kid, &root, true).unwrap();
        expect(&set, seeds);
        assert!(warnings.is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_wrong_dump_under_the_right_name_gives_way_to_the_right_one_elsewhere() {
        let g = game("t_wrong", None, NAMES, SEEDS, true);
        let root = tmp("wrong-elsewhere");
        fs::create_dir_all(root.join("t_wrong")).unwrap();
        for (n, s) in NAMES.iter().zip(SEEDS) {
            fs::write(root.join("t_wrong").join(n), chip(s)).unwrap();
        }
        // a.2 is another dump of the same size under its name; the right one is in a zip, renamed.
        fs::write(root.join("t_wrong/a.2"), chip(99)).unwrap();
        fs::write(root.join("pack.zip"), archive_of(&[("renamed", &chip(SEEDS[1]))], 8)).unwrap();
        let (set, warnings) = load(g, &root, true).unwrap();
        expect(&set, SEEDS);
        assert!(warnings.is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_wrong_dump_with_no_right_one_is_named_and_still_plays() {
        let g = game("t_bad", None, NAMES, SEEDS, true);
        let root = tmp("wrong-only");
        fs::create_dir_all(root.join("t_bad")).unwrap();
        for (n, s) in NAMES.iter().zip(SEEDS) {
            fs::write(root.join("t_bad").join(n), chip(s)).unwrap();
        }
        fs::write(root.join("t_bad/a.3"), chip(99)).unwrap();
        let (set, warnings) = load(g, &root, false).unwrap();
        assert_eq!(set.chips[2], chip(99));
        assert_eq!(warnings.len(), 1);
        let w = &warnings[0];
        assert!(
            w.contains("a.3")
                && w.contains("t_bad/a.3")
                && w.contains(&sha1_hex(&chip(99)))
                && w.contains(g.files[2].sha1),
            "{w}"
        );
        // With verify it is an error that says the same.
        let e = load(g, &root, true).err().unwrap();
        assert!(matches!(&e, Error::Hash { name: "a.3", file, .. } if file.ends_with("t_bad/a.3")), "{e}");
        let text = e.to_string();
        assert!(text.contains("a.3") && text.contains("t_bad/a.3") && text.contains(g.files[2].sha1), "{text}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_chip_is_missing_and_a_manifest_without_hashes_goes_by_name() {
        let g = game("t_missing", None, NAMES, SEEDS, true);
        let root = tmp("missing");
        fs::write(root.join("a.1"), chip(SEEDS[0])).unwrap();
        assert!(matches!(load(g, &root, false).err().unwrap(), Error::Missing { name: "a.2", .. }));
        let plain = game("t_plain", None, NAMES, SEEDS, false);
        for (n, s) in NAMES.iter().zip(SEEDS) {
            fs::write(root.join(n), chip(s.wrapping_add(100))).unwrap();
        }
        let (set, warnings) = load(plain, &root, false).unwrap();
        expect(&set, SEEDS.map(|s| s.wrapping_add(100)));
        assert!(warnings.is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_game_is_listed_when_its_chips_are_there_under_any_names() {
        let g = game("t_listed", None, NAMES, SEEDS, true);
        let root = tmp("listed");
        let mut cache = HashCache::memory();
        assert!(!has_every_chip(g, &mut Locator::new(&root), &mut cache));
        fs::write(root.join("m.zip"), archive_of(&[("a", &chip(SEEDS[0])), ("b", &chip(SEEDS[1]))], 8)).unwrap();
        assert!(!has_every_chip(g, &mut Locator::new(&root), &mut cache), "two of four");
        fs::create_dir_all(root.join("loose")).unwrap();
        fs::write(root.join("loose/c"), chip(SEEDS[2])).unwrap();
        fs::write(root.join("loose/d"), chip(SEEDS[3])).unwrap();
        assert!(has_every_chip(g, &mut Locator::new(&root), &mut cache));
        let plain = game("t_listed_plain", None, NAMES, SEEDS, false);
        assert!(!has_every_chip(plain, &mut Locator::new(&root), &mut cache), "no hashes, no content match");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hashes_are_cached_between_runs_and_a_changed_file_is_hashed_again() {
        let g = game("t_cache", None, NAMES, SEEDS, true);
        let root = tmp("cache");
        let cache_file = root.join("cache/rom-sha1.tsv");
        // The set sits under `sets`, so the cache file is not among its files.
        let sets = root.join("sets");
        fs::create_dir_all(&sets).unwrap();
        let zip = |seeds: [u8; 4]| {
            let chips: Vec<Vec<u8>> = seeds.iter().map(|&s| chip(s)).collect();
            archive_of(&[("a", &chips[0]), ("b", &chips[1]), ("c", &chips[2]), ("d", &chips[3])], 8)
        };
        fs::write(sets.join("m.zip"), zip(SEEDS)).unwrap();
        let mut cache = HashCache::at(&cache_file);
        RomSet::load_with(g, &sets, true, &mut cache).unwrap();
        cache.save();
        let saved = fs::read_to_string(&cache_file).unwrap();
        assert!(saved.contains(g.files[0].sha1), "the cache holds the chips' hashes");
        assert_eq!(HashCache::at(&cache_file).len(), saved.lines().count());
        // A zip with other content (so another stamp or CRC) does not borrow the old entries.
        fs::write(sets.join("m.zip"), zip([1, 2, 4, 6])).unwrap();
        let mut cache = HashCache::at(&cache_file);
        assert!(RomSet::load_with(g, &sets, true, &mut cache).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
