//! The list scan: block facts ([`Block`], level L1) for the chip path,
//! where no native geometrizer runs. It walks a frame's display list as the
//! geometrizer read it (the words fed, jumps already followed) without
//! executing anything: a table of operand counts for the command set the
//! programs share (docs/hw/geometrizer.md §2) and the few per-program
//! extras (`topskatr`'s fn 0x11/0x1B/0x1C, `lastbrnx`'s fn 0x1A-0x1C, the 2B
//! aliases 0x11-0x1C), tracking the focal lengths (fn 0x09), the light
//! vector (fn 0x0A) and the mode (fn 0x07), with one [`Block`] per object
//! (fn 0x01/0x11) or direct-data (fn 0x02) command. Anything it doesn't
//! know gives up (None): the frame is then shaded from the stream alone.

use super::side::Block;

/// The geometrizer the list is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanBoard {
    /// Model 2C: a TGP program (back faces sent unlit).
    C,
    /// Model 2B: a SHARC program (its own aliases, back faces lit, rounding
    /// toward zero).
    B,
    /// Model 2A: the fixed-function geometrizer (2C's command set).
    A,
}

/// A program's own commands, where known (else the shared set).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanVariant {
    #[default]
    Shared,
    /// 2C `overrev`: fn 0x0E always takes at least one block.
    Overrev,
    /// 2C `topskatr`: fn 0x11 (compact object), 0x1B (second matrix), 0x1C.
    TopSkater,
    /// 2B `lastbrnx`: fn 0x1A (one operand), 0x1B (not handled), 0x1C (one).
    Lastbrnx,
}

/// What [`scan_list`] scans for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanProgram {
    pub board: ScanBoard,
    pub variant: ScanVariant,
}

/// The state the scan carries from frame to frame (as a program keeps it):
/// None until the list has set it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanState {
    pub focus: Option<[u32; 2]>,
    pub light: Option<[u32; 3]>,
    pub mode: Option<u32>,
}

/// A count as the programs take it: 0 is 2^32 (more than any list).
fn count(n: u32) -> Option<usize> {
    (n != 0).then_some(n as usize)
}

/// Scan `words` (a frame's list as fed to the geometrizer) for its block
/// facts, from and into `state`. None if the list holds a command the scan
/// doesn't know (the state is then forgotten).
pub fn scan_list(words: &[u32], prog: ScanProgram, state: &mut ScanState) -> Option<Vec<Block>> {
    let r = scan(words, prog, state);
    if r.is_none() {
        *state = ScanState::default();
    }
    r
}

fn scan(words: &[u32], prog: ScanProgram, st: &mut ScanState) -> Option<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut i = 0usize;
    let two_b = prog.board == ScanBoard::B;
    let (skater, lastbrnx) = (prog.variant == ScanVariant::TopSkater, prog.variant == ScanVariant::Lastbrnx);
    let board_flags = if two_b { Block::TRUNCATE } else { Block::BACK_UNLIT };
    // Operand `k` words on, if the list has it (the chip then gets END).
    let at = |k: usize| words.get(k).copied();
    let block = |st: &ScanState, oba: u32, flags: u32| {
        let mut f = flags | board_flags | Block::SCANNED;
        let focus = st.focus;
        if focus.is_none() {
            f |= Block::UNKNOWN;
        }
        if st.light.is_none() || st.mode.is_none() {
            f |= Block::PARTIAL;
        }
        let [fx, fy] = focus.unwrap_or([0; 2]);
        Block {
            oba,
            flags: f,
            fx: f32::from_bits(fx),
            fy: f32::from_bits(fy),
            light: st.light.unwrap_or([0; 3]).map(f32::from_bits),
            first: 0,
            count: 0,
            key: 0,
            xf: [0.0; 24],
        }
    };
    while i < words.len() {
        let w = words[i];
        i += 1;
        let fnc = (w >> 23) & 0x1f;
        // 2B's aliases of the commands below.
        let f = if two_b && (0x11..=0x1c).contains(&fnc) && !(lastbrnx && fnc >= 0x1a) && fnc != 0x14 && fnc != 0x16 {
            fnc - 0x10
        } else {
            fnc
        };
        match f {
            0x00 => {}
            0x01 => {
                let oba = at(i + 2)?;
                let mode = st.mode.unwrap_or(0);
                let compact = fnc == 0x11 && skater;
                let mut flags = 0;
                if compact {
                    flags |= Block::COMPUTED | Block::COMPACT;
                } else if mode & 2 != 0 {
                    flags |= Block::COMPUTED;
                }
                if mode & 1 != 0 {
                    flags |= Block::SPECULAR;
                }
                blocks.push(block(st, oba, flags));
                i += 4;
            }
            0x11 if skater => {
                let oba = at(i + 2)?;
                let mode = st.mode.unwrap_or(0);
                let flags = Block::COMPUTED | Block::COMPACT | if mode & 1 != 0 { Block::SPECULAR } else { 0 };
                blocks.push(block(st, oba, flags));
                i += 4;
            }
            0x02 => {
                blocks.push(block(st, 0, Block::DIRECT));
                i += 8;
                // Polygons: an attribute word, 5 words (8 for a quad); the
                // list ends them with an attribute word of type 0. The 2B
                // program takes the first one whatever its type.
                let mut first = two_b;
                loop {
                    let attr = at(i)?;
                    i += 1;
                    if attr & 3 == 0 && !first {
                        break;
                    }
                    first = false;
                    i += if attr & 1 != 0 { 8 } else { 5 };
                }
            }
            0x03 => i += 6,
            0x04 | 0x14 => i += 2 + count(at(i + 1)?)?,
            0x05 => {
                let n = at(i + 1)?;
                i += 2 + if two_b {
                    count(n)?
                } else {
                    if n & 0xffff_8000 != 0 || at(i)? & 0xff00_0000 != 0 {
                        return None;
                    }
                    ((n & 0x3fff) as usize).max(1)
                };
            }
            0x06 => i += 2 + 2 * count(at(i + 1)?)?,
            0x07 => {
                st.mode = Some(at(i)?);
                i += 1;
            }
            0x08 | 0x10 | 0x16 => i += 1,
            0x09 => {
                st.focus = Some([at(i)?, at(i + 1)?]);
                i += 2;
            }
            0x0a => {
                st.light = Some([at(i)?, at(i + 1)?, at(i + 2)?]);
                i += 3;
            }
            0x0b => i += 12,
            0x0c => i += 3,
            0x0d => i += 2,
            0x0e => {
                // The board test: a walking one, then blocks of three words.
                for k in 0..32 {
                    if at(i + k)? != 1 << k && !(prog.variant == ScanVariant::Overrev || skater) {
                        return None;
                    }
                }
                i += 32;
                let n = at(i)?;
                i += 1;
                if n == 0 && prog.variant == ScanVariant::Overrev {
                    return None;
                }
                i += 3 * n as usize;
            }
            0x0f => return Some(blocks),
            0x1f if two_b => return Some(blocks),
            0x1a if lastbrnx => i += 1,
            0x1c if lastbrnx => i += 1,
            0x1b if skater => i += 12,
            0x1c if skater => {}
            _ => return None,
        }
    }
    Some(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::test_scene::{cmd, f, list};

    const C: ScanProgram = ScanProgram { board: ScanBoard::C, variant: ScanVariant::Shared };

    #[test]
    fn blocks_and_state() {
        let mut st = ScanState::default();
        let b = scan_list(&list(3), C, &mut st).unwrap();
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].oba, b[0].fx, b[0].fy, b[0].light), (0x80_0000, 500.0, 480.0, [0.3, -0.5, 0.81]));
        assert_eq!(b[0].flags, Block::BACK_UNLIT | Block::SCANNED);
        assert_eq!(b[1].flags, Block::BACK_UNLIT | Block::SCANNED | Block::COMPUTED | Block::SPECULAR);
        assert_eq!(b[2].flags, Block::BACK_UNLIT | Block::SCANNED | Block::DIRECT);
        // The state carries to the next frame; before any, unknown.
        let next = [cmd(0x01), 0, 0, 0x80_0100, 0, cmd(0x0f)];
        let b = scan_list(&next, C, &mut st).unwrap();
        assert_eq!((b[0].fx, b[0].light, b[0].flags & Block::SPECULAR), (500.0, [-0.6, 0.2, 0.77], Block::SPECULAR));
        let b = scan_list(&next, C, &mut ScanState::default()).unwrap();
        assert_eq!(b[0].flags & (Block::UNKNOWN | Block::PARTIAL), Block::UNKNOWN | Block::PARTIAL);
        // A command it doesn't know: gives up, forgets the state.
        assert!(scan_list(&[cmd(0x12), cmd(0x0f)], C, &mut st).is_none());
        assert_eq!(st, ScanState::default());
    }

    /// 2B's aliases (0x11-0x1C), `lastbrnx`'s and `topskatr`'s own
    /// commands, the 2B direct data's first polygon.
    #[test]
    fn program_commands() {
        let b2 = ScanProgram { board: ScanBoard::B, variant: ScanVariant::Shared };
        let words = [cmd(0x19), f(2.0), f(3.0), cmd(0x1a), 1, 2, 3, cmd(0x17), 2, cmd(0x11), 0, 0, 7, 0, cmd(0x1f)];
        let b = scan_list(&words, b2, &mut ScanState::default()).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].oba, b[0].fx, b[0].light.map(f32::to_bits)), (7, 2.0, [1, 2, 3]));
        assert_eq!(b[0].flags, Block::TRUNCATE | Block::SCANNED | Block::COMPUTED);
        let lb = ScanProgram { board: ScanBoard::B, variant: ScanVariant::Lastbrnx };
        let words = [cmd(0x1a), 1, cmd(0x1c), 0x80c010, cmd(0x01), 0, 0, 7, 0, cmd(0x0f)];
        assert_eq!(scan_list(&words, lb, &mut ScanState::default()).unwrap().len(), 1);
        assert!(scan_list(&[cmd(0x1b), cmd(0x0f)], lb, &mut ScanState::default()).is_none());
        let ts = ScanProgram { board: ScanBoard::C, variant: ScanVariant::TopSkater };
        let words = [cmd(0x1b), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, cmd(0x1c), cmd(0x11), 0, 0, 9, 0, cmd(0x0f)];
        let b = scan_list(&words, ts, &mut ScanState::default()).unwrap();
        assert_eq!(b[0].flags & (Block::COMPACT | Block::COMPUTED), Block::COMPACT | Block::COMPUTED);
        assert!(scan_list(&words, C, &mut ScanState::default()).is_none(), "not the shared program's");
        // Direct data whose first attribute word is of type 0: the 2B
        // program still takes a polygon (then reads on to the next type 0);
        // the 2C ones none.
        let x = cmd(0x12);
        let direct = [cmd(0x02), 0, 0, 0, 0, 0, 0, 0, 0, 0, x, x, x, x, x, 0, cmd(0x01), 0, 0, 5, 0, cmd(0x0f)];
        assert_eq!(scan_list(&direct, b2, &mut ScanState::default()).unwrap().len(), 2);
        assert!(scan_list(&direct, C, &mut ScanState::default()).is_none());
    }
}
