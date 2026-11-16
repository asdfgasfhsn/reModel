//! What the settings rows say for a game, shared by the in-game menu
//! (`menu`) and the launcher's settings list (`launcher`): the picture
//! preset's help line, the run-ahead row's choices and help line, and
//! whether this computer has time for run-ahead.

use crate::settings::{GameSettings, Preset, MAX_RUN_AHEAD};

/// The CRT row's choices (strength in percent, label).
pub const CRT: [(u8, &str); 4] = [(0, "Off"), (30, "Light"), (60, "Medium"), (100, "Strong")];

/// The CRT row's help line.
pub const CRT_HELP: &str = "Scanlines, a soft tube edge and a slight curve, like an arcade monitor.";

/// A style's small tag in the picture menu.
pub fn style_tag(p: Preset) -> &'static str {
    match p {
        Preset::Original => "as the cabinet",
        Preset::Enhanced => "recommended",
        Preset::Custom => "from Advanced",
        _ => "",
    }
}

/// A style's one plain sentence in the picture menu, for `game` (what
/// Smooth modern turns on is the game's: `GameSettings::apply_preset`).
pub fn style_line(p: Preset, game: Option<&romset::Game>) -> String {
    match p {
        Preset::Original => "The picture as the arcade board draws it: nothing added.".into(),
        Preset::Sharp => "Clean edges and crisp textures. The same picture, crisper.".into(),
        Preset::Crt => "Scanlines and a soft tube edge, like an arcade monitor.".into(),
        Preset::Custom => "Your own mix of the advanced settings.".into(),
        Preset::Enhanced => {
            let mut d = GameSettings::default();
            d.apply_preset(Preset::Enhanced, game);
            let mut text = "Sharp".to_string();
            if d.shading != gpu::Shading::Flat {
                text += ", with smooth lighting";
            }
            if d.smooth {
                text += " and in-between frames for fast screens";
            }
            text + ", set up for this game."
        }
    }
}

/// The bloom row's choices (strength in percent, label).
pub const BLOOM: [(u8, &str); 4] = [(0, "Off"), (25, "Light"), (50, "Medium"), (100, "Strong")];

/// The help line of the bloom row.
pub const BLOOM_HELP: &str = "Bright parts of the 3D picture glow. The HUD is left sharp.";

/// The run-ahead row's choices (frames, label); a game offers the first
/// `run_ahead_max + 1`.
pub const RUN_AHEAD: [(u32, &str); 4] = [(0, "Off"), (1, "1 frame"), (2, "2 frames"), (3, "3 frames")];

/// The frames of run-ahead worth offering for `game` (none: the most any
/// game takes), and the most the row lists.
pub fn run_ahead_max(game: Option<&romset::Game>) -> u32 {
    game.map_or(MAX_RUN_AHEAD, |g| g.run_ahead_max as u32)
}

/// Whether the run-ahead row is worth showing: the game has some to offer,
/// or it is set anyway (so it can be turned off).
pub fn run_ahead_shown(game: Option<&romset::Game>, set: u32) -> bool {
    run_ahead_max(game) > 0 || set > 0
}

/// What the machine costs a displayed frame, measured with no run-ahead:
/// the game's emulation of one frame, the rest of the main thread's work
/// for it (copying the frame for drawing, the overlay), drawing it, and the
/// time the display allows (all in seconds); and whether drawing runs on
/// its own thread alongside the next frame's emulation (docs/play.md, "The
/// frame loop").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameCost {
    pub emulate: f64,
    pub overhead: f64,
    pub draw: f64,
    pub budget: f64,
    pub overlap: bool,
}

impl FrameCost {
    /// Share of the display's frame time to plan on using: the player gives
    /// run-ahead up at 95%, and wants room to spare.
    const USABLE: f64 = 0.85;

    /// The frames of run-ahead this machine can afford: running N ahead
    /// emulates N+1 frames for each one displayed, besides drawing it. On
    /// one thread they add up; with the drawing on its own thread, N fits
    /// if the N+1 frames do, and the drawing does by itself.
    pub fn run_ahead_room(&self) -> u32 {
        if self.emulate <= 0.0 || !self.emulate.is_finite() {
            return MAX_RUN_AHEAD;
        }
        let usable = self.budget * Self::USABLE;
        let left = if !self.overlap {
            usable - self.overhead - self.draw
        } else if self.draw > usable {
            return 0;
        } else {
            usable - self.overhead
        };
        let frames = (left / self.emulate).floor();
        (frames - 1.0).clamp(0.0, MAX_RUN_AHEAD as f64) as u32
    }
}

/// The room the note speaks of: what the computer has time for, up to what
/// the game takes (or what is set anyway).
fn usable(room: u32, max: u32, chosen: u32) -> u32 {
    room.min(max.max(chosen))
}

/// Whether `chosen` is more than this computer has time for (the note
/// warns of it).
pub fn run_ahead_warns(room: Option<u32>, max: u32, chosen: u32) -> bool {
    room.is_some_and(|r| chosen > usable(r, max, chosen))
}

/// What this computer can do about the run-ahead row, said in its help
/// line: `room` is [`FrameCost::run_ahead_room`] (None until measured),
/// `max` what the game offers, `chosen` the setting.
pub fn run_ahead_note(room: Option<u32>, max: u32, chosen: u32) -> Option<String> {
    Some(match usable(room?, max, chosen) {
        0 => "This computer is too slow for run-ahead.".to_string(),
        n if chosen > n => format!(
            "This computer has time for about {n}: {chosen} is more than it can keep up with, and is switched off \
             if the game falls behind."
        ),
        n => format!("This computer has time for about {n}."),
    })
}

/// The help line of the run-ahead row: what it does for `game`, what this
/// computer can do (`room`), and whether it was switched off.
pub fn run_ahead_help(game: Option<&romset::Game>, dropped: bool, room: Option<u32>, chosen: u32) -> String {
    let mut text = match game {
        None => "Hides input lag at the cost of CPU time.".to_string(),
        Some(g) if g.run_ahead_max > g.input_lag => format!(
            "Hides a frame of lag from the punches, and makes the stick judder: the stick has none to hide. \
             Costs about {}x the game's CPU time.",
            g.run_ahead_max + 1
        ),
        Some(g) => format!(
            "Hides {} of this game's {} of input lag, at the cost of CPU time (about {}x the game's); more only \
             judders.",
            frames(g.run_ahead_max as u32),
            frames(g.input_lag as u32),
            g.run_ahead_max + 1
        ),
    };
    if let Some(note) = run_ahead_note(room, run_ahead_max(game), chosen) {
        text += " ";
        text += &note;
    }
    if dropped {
        text += " Switched off: this machine could not keep up. Choosing it again tries again.";
    }
    text
}

fn frames(n: u32) -> String {
    if n == 1 {
        "1 frame".into()
    } else {
        format!("{n} frames")
    }
}

/// What a style does for `game`, as the help line of the preset row
/// (`temporal_supported`: the GPU can do temporal anti-aliasing; `geo_chip`:
/// the game's chip draws).
pub fn preset_help(p: Preset, game: Option<&romset::Game>, temporal_supported: bool, geo_chip: bool) -> String {
    match p {
        Preset::Original => "The arcade look: nothing that changes the picture.".into(),
        Preset::Sharp => "4x anti-aliasing and mipmapped texture filtering, nothing else.".into(),
        Preset::Custom => "Choose each picture setting yourself.".into(),
        Preset::Crt => "Sharp, with scanlines, a mask and a soft edge like an arcade monitor.".into(),
        Preset::Enhanced => {
            let mut d = GameSettings::default();
            d.apply_preset(Preset::Enhanced, game);
            let mut parts = Vec::new();
            parts.push(match d.temporal {
                _ if d.temporal > 0 && geo_chip => "4x anti-aliasing (temporal needs the native geometrizer)",
                _ if d.temporal > 0 && !temporal_supported => "4x anti-aliasing (this GPU can't do temporal)",
                0 => "4x anti-aliasing",
                _ => "temporal anti-aliasing",
            });
            if d.depth {
                parts.push("the depth buffer");
            }
            if d.shading != gpu::Shading::Flat {
                parts.push("smooth shading");
            }
            if !geo_chip {
                parts.push("smooth motion");
            }
            let mut text = format!("For this game: {}", parts.join(", "));
            if d.run_ahead > 0 {
                text += &format!(", run-ahead {}", d.run_ahead);
            }
            text + "."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_follows_the_frame_budget() {
        let ms = |e: f64, d: f64| FrameCost {
            emulate: e / 1000.0,
            overhead: 0.0,
            draw: d / 1000.0,
            budget: 1.0 / 60.0,
            overlap: false,
        };
        // 16.7 ms a frame, 85% usable: 14.2 ms.
        assert_eq!(ms(1.0, 2.0).run_ahead_room(), 3, "plenty: capped at the most");
        assert_eq!(ms(3.0, 2.0).run_ahead_room(), 3, "4 frames fit, one is the game's own");
        assert_eq!(ms(4.0, 2.0).run_ahead_room(), 2);
        assert_eq!(ms(6.0, 2.0).run_ahead_room(), 1);
        assert_eq!(ms(8.0, 2.0).run_ahead_room(), 0, "one frame fits: no room to run ahead");
        assert_eq!(ms(20.0, 2.0).run_ahead_room(), 0, "not even one");
        assert_eq!(ms(5.0, 30.0).run_ahead_room(), 0, "drawing alone takes too long");
        // A faster display leaves less.
        let fast = FrameCost { emulate: 0.004, overhead: 0.0, draw: 0.002, budget: 1.0 / 120.0, overlap: false };
        assert_eq!(fast.run_ahead_room(), 0);
    }

    #[test]
    fn room_with_drawing_alongside() {
        let ms = |e: f64, o: f64, d: f64| FrameCost {
            emulate: e / 1000.0,
            overhead: o / 1000.0,
            draw: d / 1000.0,
            budget: 1.0 / 60.0,
            overlap: true,
        };
        // 14.2 ms usable: the frames and the drawing each get all of it.
        assert_eq!(ms(4.0, 0.0, 10.0).run_ahead_room(), 2, "3 frames fit beside a 10 ms draw");
        assert_eq!(ms(4.0, 2.5, 10.0).run_ahead_room(), 1, "the copy and overlay take from the frames");
        assert_eq!(ms(3.5, 0.5, 12.0).run_ahead_room(), 2, "vf2 with heavy settings");
        assert_eq!(ms(2.0, 0.0, 15.0).run_ahead_room(), 0, "drawing alone takes too long");
        // On one thread the same machine has room for none.
        let one = FrameCost { overlap: false, ..ms(3.5, 0.5, 12.0) };
        assert_eq!(one.run_ahead_room(), 0);
    }

    #[test]
    fn the_note_says_what_this_computer_can_do() {
        assert_eq!(run_ahead_note(None, 3, 2), None, "nothing measured yet: nothing said");
        assert!(run_ahead_note(Some(2), 3, 0).unwrap().contains("about 2"));
        assert!(run_ahead_note(Some(0), 3, 0).unwrap().contains("too slow"));
        // A choice past the room is warned of; within it, not.
        let n = run_ahead_note(Some(1), 3, 3).unwrap();
        assert!(n.contains("about 1") && n.contains("3 is more") && n.contains("falls behind"), "{n}");
        assert!(!run_ahead_note(Some(1), 3, 1).unwrap().contains("more than"));
        // A computer with room for more than the game takes says the most.
        assert!(run_ahead_note(Some(3), 1, 0).unwrap().contains("about 1"));
        assert!(run_ahead_note(Some(0), 3, 2).unwrap().contains("too slow"));
        assert!(run_ahead_warns(Some(1), 3, 2) && !run_ahead_warns(Some(1), 3, 1));
        assert!(!run_ahead_warns(None, 3, 3) && !run_ahead_warns(Some(0), 3, 0));
    }

    #[test]
    fn run_ahead_help_says_what_it_does_for_the_game() {
        let help = |id: &str, dropped| run_ahead_help(romset::games::playable(id), dropped, None, 0);
        let h = help("sgt24h", false);
        assert!(h.contains("3 frames") && h.contains("4 frames"), "{h}");
        let h = help("stcc", false);
        assert!(h.contains("1 frame") && !h.contains("1 frames"), "{h}");
        for id in ["schamp", "lastbrnx"] {
            let h = help(id, false);
            assert!(h.contains("punches") && h.contains("judder"), "{id}: {h}");
        }
        assert!(!help("stcc", false).contains("too slow") && !help("stcc", false).contains("could not keep up"));
        assert!(help("stcc", true).contains("could not keep up"));
        assert!(run_ahead_help(None, false, None, 0).contains("input lag"));
        assert_eq!(run_ahead_max(romset::games::playable("sgt24h")), 3);
        assert_eq!(run_ahead_max(romset::games::playable("hotd")), 0);
        assert!(!run_ahead_shown(romset::games::playable("hotd"), 0));
        assert!(run_ahead_shown(romset::games::playable("hotd"), 1), "a stray setting can be turned off");
        // With an estimate, the line says what the machine can do.
        let sgt = romset::games::playable("sgt24h");
        let h = run_ahead_help(sgt, false, Some(2), 0);
        assert!(h.contains("4 frames") && h.ends_with("about 2."), "{h}");
        let h = run_ahead_help(sgt, false, Some(0), 3);
        assert!(h.contains("too slow for run-ahead"), "{h}");
        assert!(run_ahead_help(sgt, false, Some(1), 3).contains("3 is more"));
    }

    #[test]
    fn the_help_line_says_what_the_preset_does() {
        let stcc = romset::games::playable("stcc");
        assert!(preset_help(Preset::Enhanced, stcc, true, false).contains("smooth shading"));
        assert!(!preset_help(Preset::Enhanced, stcc, true, false).contains("depth buffer"));
        assert!(preset_help(Preset::Enhanced, stcc, true, false).contains("run-ahead 1"));
        assert!(preset_help(Preset::Enhanced, stcc, true, true).contains("native geometrizer"));
        assert!(preset_help(Preset::Enhanced, romset::games::playable("bel"), true, false).contains("depth buffer"));
    }
}
