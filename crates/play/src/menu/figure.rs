//! The pictures on the Controls page: the pad, the wheel and pedals, and
//! the light gun (original art, `assets/controls/README.md`), where each of
//! their parts is, and the keyboard drawn from its keys.
//!
//! The art's parts are measured on the 1536 x 1024 originals and kept as
//! fractions of the picture, so the pictures can be drawn at any size; the
//! page's labels sit beside the picture with a leader line to each part
//! (`place_labels`). `debug_overlay` draws the parts over the art, to check
//! them by eye.

use crate::controls::PadStyle;
use egui::{pos2, vec2, Pos2, Rect};
use romset::PadButton;

/// A picture on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Art {
    /// A gamepad with its sticks offset (an Xbox-style pad).
    PadOffset,
    /// A gamepad with its sticks side by side (a PlayStation-style pad).
    PadSide,
    Gun,
    Wheel,
}

impl Art {
    #[cfg(test)]
    pub const ALL: [Art; 4] = [Art::PadOffset, Art::PadSide, Art::Gun, Art::Wheel];

    /// The art for a pad of this style.
    pub fn pad(style: PadStyle) -> Art {
        match style {
            PadStyle::Offset => Art::PadOffset,
            PadStyle::SideBySide => Art::PadSide,
        }
    }

    /// The image file, WebP, 1024 wide.
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Art::PadOffset => include_bytes!("../../assets/controls/pad-1.webp"),
            Art::PadSide => include_bytes!("../../assets/controls/pad-2.webp"),
            Art::Gun => include_bytes!("../../assets/controls/art-G.webp"),
            Art::Wheel => include_bytes!("../../assets/controls/art-W.webp"),
        }
    }

    /// The part of the image worth showing (the originals have wide black
    /// margins): left, top, right and bottom as fractions.
    pub fn crop(self) -> [f32; 4] {
        match self {
            Art::PadOffset | Art::PadSide => [0.05, 0.02, 0.95, 0.99],
            Art::Gun => [0.06, 0.05, 0.94, 0.95],
            Art::Wheel => [0.25, 0.02, 0.75, 0.97],
        }
    }

    /// Width over height of the cropped picture.
    pub fn aspect(self) -> f32 {
        let c = self.crop();
        (c[2] - c[0]) * 1536.0 / ((c[3] - c[1]) * 1024.0)
    }

    /// The picture, fitted and centred in `area`.
    pub fn fit(self, area: Rect) -> Rect {
        let a = self.aspect();
        let (w, h) = if area.width() / area.height() > a {
            (area.height() * a, area.height())
        } else {
            (area.width(), area.width() / a)
        };
        Rect::from_center_size(area.center(), vec2(w, h))
    }

    /// Where a point of the image (fractions of the whole image) is, with
    /// the cropped picture drawn in `at`.
    pub fn point(self, at: Rect, x: f32, y: f32) -> Pos2 {
        let c = self.crop();
        pos2(at.left() + (x - c[0]) / (c[2] - c[0]) * at.width(), at.top() + (y - c[1]) / (c[3] - c[1]) * at.height())
    }

    /// The cropped part's texture coordinates.
    pub fn uv(self) -> Rect {
        let c = self.crop();
        Rect::from_min_max(pos2(c[0], c[1]), pos2(c[2], c[3]))
    }
}

/// The art as an image for a texture. The originals are drawn on pure
/// black: that becomes see-through (a pixel's brightness is its opacity,
/// its colour premultiplied), so the picture sits on the card, glow and
/// all.
pub fn load(art: Art) -> Option<egui::ColorImage> {
    let img = crate::art::decode(art.bytes())?;
    let pixels = img
        .pixels
        .iter()
        .map(|p| {
            let a = p.r().max(p.g()).max(p.b());
            egui::Color32::from_rgba_premultiplied(p.r(), p.g(), p.b(), a)
        })
        .collect();
    Some(egui::ColorImage { size: img.size, source_size: img.source_size, pixels })
}

/// A part of a wheel and pedals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WheelPart {
    Rim,
    Hub,
    PaddleLeft,
    PaddleRight,
    /// The four buttons on the spokes: top left, bottom left, top right,
    /// bottom right.
    Button(u8),
    Brake,
    Gas,
}

/// A part of a light gun.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GunPart {
    Trigger,
    /// The round button on top.
    Thumb,
    /// The button on the side of the grip.
    Side,
    /// Where it points.
    Muzzle,
}

/// A part of a device's picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// A button, stick (its press) or trigger of the pad.
    Pad(PadButton),
    Wheel(WheelPart),
    Gun(GunPart),
}

/// How a part is outlined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A rounded box (a circle, if as wide as it's tall).
    Round,
    /// A ring (the wheel's rim): only its outline is lit.
    Ring,
}

/// Where a part is: its centre and size as fractions of the whole image's
/// width and height, and where its label's leader line meets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub part: Part,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub shape: Shape,
    /// The leader line's end, if not the centre.
    pub pin: Option<(f32, f32)>,
}

impl Spot {
    /// Where the leader line meets the part.
    pub fn pin(&self) -> (f32, f32) {
        self.pin.unwrap_or((self.x, self.y))
    }
}

/// A spot from a measurement on the 1536 x 1024 original, in pixels:
/// centre and size.
fn m(part: Part, x: f32, y: f32, w: f32, h: f32) -> Spot {
    Spot { part, x: x / 1536.0, y: y / 1024.0, w: w / 1536.0, h: h / 1024.0, shape: Shape::Round, pin: None }
}

/// The parts of `art`.
pub fn spots(art: Art) -> Vec<Spot> {
    use PadButton::*;
    let pad = |b: PadButton, x, y, w, h| m(Part::Pad(b), x, y, w, h);
    match art {
        Art::PadOffset => vec![
            pad(LeftStick, 412.0, 358.0, 190.0, 190.0),
            pad(RightStick, 1015.0, 617.0, 190.0, 190.0),
            pad(DPadUp, 542.0, 530.0, 75.0, 70.0),
            pad(DPadDown, 542.0, 685.0, 75.0, 70.0),
            pad(DPadLeft, 465.0, 606.0, 70.0, 75.0),
            pad(DPadRight, 620.0, 606.0, 70.0, 75.0),
            pad(North, 1116.0, 252.0, 96.0, 96.0),
            pad(West, 1028.0, 348.0, 96.0, 96.0),
            pad(East, 1202.0, 348.0, 96.0, 96.0),
            pad(South, 1116.0, 436.0, 96.0, 96.0),
            pad(Select, 692.0, 410.0, 88.0, 44.0),
            pad(Start, 845.0, 410.0, 88.0, 44.0),
            pad(LeftShoulder, 425.0, 142.0, 230.0, 60.0),
            pad(RightShoulder, 1115.0, 142.0, 230.0, 60.0),
            pad(LeftTrigger, 402.0, 75.0, 135.0, 75.0),
            pad(RightTrigger, 1134.0, 78.0, 128.0, 75.0),
        ],
        Art::PadSide => vec![
            pad(LeftStick, 598.0, 578.0, 175.0, 175.0),
            pad(RightStick, 937.0, 578.0, 175.0, 175.0),
            pad(DPadUp, 431.0, 283.0, 75.0, 70.0),
            pad(DPadDown, 431.0, 428.0, 75.0, 70.0),
            pad(DPadLeft, 358.0, 355.0, 70.0, 75.0),
            pad(DPadRight, 505.0, 355.0, 70.0, 75.0),
            pad(North, 1103.0, 260.0, 92.0, 92.0),
            pad(West, 1021.0, 350.0, 92.0, 92.0),
            pad(East, 1183.0, 350.0, 92.0, 92.0),
            pad(South, 1103.0, 430.0, 92.0, 92.0),
            pad(Select, 700.0, 380.0, 72.0, 38.0),
            pad(Start, 834.0, 380.0, 72.0, 38.0),
            pad(LeftShoulder, 448.0, 165.0, 205.0, 50.0),
            pad(RightShoulder, 1088.0, 165.0, 205.0, 50.0),
            pad(LeftTrigger, 382.0, 100.0, 125.0, 75.0),
            pad(RightTrigger, 1150.0, 98.0, 125.0, 75.0),
        ],
        Art::Gun => {
            let g = |p, x, y, w, h| m(Part::Gun(p), x, y, w, h);
            vec![
                g(GunPart::Trigger, 610.0, 540.0, 90.0, 140.0),
                g(GunPart::Thumb, 478.0, 125.0, 150.0, 55.0),
                g(GunPart::Side, 365.0, 555.0, 115.0, 115.0),
                g(GunPart::Muzzle, 1310.0, 322.0, 180.0, 240.0),
            ]
        }
        Art::Wheel => {
            let w = |p, x, y, w, h| m(Part::Wheel(p), x, y, w, h);
            let rim = Spot {
                shape: Shape::Ring,
                pin: Some((497.0 / 1536.0, 250.0 / 1024.0)),
                ..w(WheelPart::Rim, 765.0, 325.0, 570.0, 570.0)
            };
            vec![
                rim,
                w(WheelPart::Hub, 765.0, 328.0, 150.0, 150.0),
                w(WheelPart::PaddleLeft, 612.0, 238.0, 75.0, 105.0),
                w(WheelPart::PaddleRight, 915.0, 235.0, 75.0, 105.0),
                w(WheelPart::Button(0), 607.0, 308.0, 52.0, 52.0),
                w(WheelPart::Button(1), 607.0, 358.0, 52.0, 52.0),
                w(WheelPart::Button(2), 912.0, 305.0, 52.0, 52.0),
                w(WheelPart::Button(3), 912.0, 358.0, 52.0, 52.0),
                w(WheelPart::Brake, 672.0, 795.0, 125.0, 150.0),
                w(WheelPart::Gas, 856.0, 790.0, 110.0, 170.0),
            ]
        }
    }
}

/// A key on the keyboard picture: its code, its left edge and row in key
/// units, and its width.
pub struct KeyCap {
    pub code: &'static str,
    pub x: f32,
    pub row: u8,
    pub w: f32,
}

/// The keys the keyboard picture draws: the main block (digits, three
/// letter rows, the spacebar row) and the arrows. Other keys a game is
/// bound to (the numeric pad's) aren't pictured.
pub fn keyboard() -> Vec<KeyCap> {
    fn run(out: &mut Vec<KeyCap>, row: u8, x0: f32, codes: &[&'static str]) {
        for (i, code) in codes.iter().enumerate() {
            out.push(KeyCap { code, x: x0 + i as f32, row, w: 1.0 });
        }
    }
    let mut k = Vec::new();
    run(
        &mut k,
        0,
        0.0,
        &["Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9", "Digit0"],
    );
    run(&mut k, 1, 0.5, &["KeyQ", "KeyW", "KeyE", "KeyR", "KeyT", "KeyY", "KeyU", "KeyI", "KeyO", "KeyP"]);
    run(&mut k, 2, 0.75, &["KeyA", "KeyS", "KeyD", "KeyF", "KeyG", "KeyH", "KeyJ", "KeyK", "KeyL", "Semicolon"]);
    k.push(KeyCap { code: "Enter", x: 10.75, row: 2, w: 1.75 });
    k.push(KeyCap { code: "ShiftLeft", x: 0.0, row: 3, w: 1.25 });
    run(&mut k, 3, 1.25, &["KeyZ", "KeyX", "KeyC", "KeyV", "KeyB", "KeyN", "KeyM", "Comma", "Period", "Slash"]);
    k.push(KeyCap { code: "ShiftRight", x: 11.25, row: 3, w: 1.5 });
    k.push(KeyCap { code: "ControlLeft", x: 0.0, row: 4, w: 1.25 });
    k.push(KeyCap { code: "AltLeft", x: 1.25, row: 4, w: 1.25 });
    k.push(KeyCap { code: "Space", x: 2.5, row: 4, w: 5.5 });
    k.push(KeyCap { code: "AltRight", x: 8.0, row: 4, w: 1.25 });
    k.push(KeyCap { code: "ControlRight", x: 9.25, row: 4, w: 1.25 });
    k.push(KeyCap { code: "ArrowUp", x: 14.0, row: 3, w: 1.0 });
    k.push(KeyCap { code: "ArrowLeft", x: 13.0, row: 4, w: 1.0 });
    k.push(KeyCap { code: "ArrowDown", x: 14.0, row: 4, w: 1.0 });
    k.push(KeyCap { code: "ArrowRight", x: 15.0, row: 4, w: 1.0 });
    k
}

/// The keyboard picture's size in key units.
pub const KEYBOARD_UNITS: (f32, f32) = (16.0, 5.0);

/// What a key's cap says.
pub fn cap_text(code: &str) -> &str {
    match code {
        "Space" => "Space",
        "Enter" => "Enter",
        "ShiftLeft" | "ShiftRight" => "Shift",
        "ControlLeft" | "ControlRight" => "Ctrl",
        "AltLeft" | "AltRight" => "Alt",
        "Semicolon" => ";",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        other => romset::controls::key_symbol(other),
    }
}

/// A label for a part: where its leader line meets the picture, what it
/// says, and whether it's the focused action's.
#[derive(Clone, Debug)]
pub struct Tag {
    pub at: Pos2,
    pub text: String,
    pub lit: bool,
}

/// A label put in its place: its box (beside the picture, text hugging the
/// side nearest it), the leader line's ends and whether it's on the left.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub label: Rect,
    pub from: Pos2,
    pub to: Pos2,
    pub left: bool,
    pub index: usize,
}

/// Put `tags`' labels beside `picture` inside `card`: those whose part is on
/// the picture's left half on the left, the others on the right, each side
/// top to bottom in the parts' order, `pitch` apart at least (pushed down,
/// then back up if the last runs off the card's foot, `top..bottom`).
pub fn place_labels(tags: &[Tag], card: Rect, picture: Rect, pitch: f32, top: f32, bottom: f32) -> Vec<Placed> {
    let mut out = Vec::new();
    let gap = pitch * 0.55;
    for left in [true, false] {
        let mut side: Vec<usize> = (0..tags.len()).filter(|&i| (tags[i].at.x < picture.center().x) == left).collect();
        side.sort_by(|&a, &b| tags[a].at.y.total_cmp(&tags[b].at.y));
        let mut ys: Vec<f32> = side.iter().map(|&i| tags[i].at.y).collect();
        for i in 0..ys.len() {
            let min = if i == 0 { top + pitch / 2.0 } else { ys[i - 1] + pitch };
            ys[i] = ys[i].max(min);
        }
        for i in (0..ys.len()).rev() {
            let max = if i + 1 == ys.len() { bottom - pitch / 2.0 } else { ys[i + 1] - pitch };
            ys[i] = ys[i].min(max);
        }
        let (x0, x1) =
            if left { (card.left() + gap, picture.left() - gap) } else { (picture.right() + gap, card.right() - gap) };
        for (&i, &y) in side.iter().zip(&ys) {
            let label = Rect::from_x_y_ranges(x0..=x1, y - pitch / 2.0..=y + pitch / 2.0);
            let from = pos2(if left { x1 } else { x0 }, y);
            out.push(Placed { label, from, to: tags[i].at, left, index: i });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_art_decodes_and_is_see_through_where_black() {
        for art in Art::ALL {
            let img = load(art).unwrap_or_else(|| panic!("{art:?}"));
            assert_eq!(img.size, [1024, 683], "{art:?}");
            assert_eq!(img.pixels[0].a(), 0, "{art:?}: the black corner is clear");
            assert!(img.pixels.iter().any(|p| p.a() > 200), "{art:?}: has a body");
        }
    }

    #[test]
    fn the_four_images_are_small_and_every_part_is_on_its_art() {
        let total: usize = Art::ALL.iter().map(|a| a.bytes().len()).sum();
        assert!(total < 400_000, "{total} bytes");
        for art in Art::ALL {
            let c = art.crop();
            for s in spots(art) {
                assert!(s.x - s.w / 2.0 >= c[0] - 0.02 && s.x + s.w / 2.0 <= c[2] + 0.02, "{art:?} {:?}", s.part);
                assert!(s.y - s.h / 2.0 >= c[1] - 0.02 && s.y + s.h / 2.0 <= c[3] + 0.02, "{art:?} {:?}", s.part);
            }
        }
    }

    /// Both pad pictures have every button, stick and trigger once.
    #[test]
    fn the_pads_have_every_part() {
        for art in [Art::PadOffset, Art::PadSide] {
            let s = spots(art);
            for b in PadButton::ALL {
                let n = s.iter().filter(|s| s.part == Part::Pad(b)).count();
                assert_eq!(n, 1, "{art:?}: {b:?}");
            }
        }
    }

    #[test]
    fn a_point_of_the_image_follows_the_crop() {
        let art = Art::PadOffset;
        let at = art.fit(Rect::from_min_size(pos2(10.0, 20.0), vec2(500.0, 400.0)));
        let c = art.crop();
        assert_eq!(art.point(at, c[0], c[1]), at.min);
        assert!((art.point(at, c[2], c[3]) - at.max).length() < 0.01);
        assert!((at.width() / at.height() - art.aspect()).abs() < 0.01);
    }

    #[test]
    fn the_keyboard_has_no_overlapping_keys() {
        let keys = keyboard();
        for (i, a) in keys.iter().enumerate() {
            assert!(a.x + a.w <= KEYBOARD_UNITS.0 && (a.row as f32) < KEYBOARD_UNITS.1, "{}", a.code);
            for b in &keys[i + 1..] {
                if a.row == b.row {
                    assert!(a.x + a.w <= b.x + 0.001 || b.x + b.w <= a.x + 0.001, "{} and {}", a.code, b.code);
                }
            }
        }
    }

    #[test]
    fn labels_keep_apart_inside_the_card() {
        let card = Rect::from_min_size(pos2(0.0, 0.0), vec2(776.0, 300.0));
        let picture = Rect::from_center_size(card.center(), vec2(400.0, 280.0));
        // Many parts at nearly one height, on both sides.
        let tags: Vec<Tag> = (0..14)
            .map(|i| Tag {
                at: pos2(if i % 2 == 0 { 250.0 } else { 520.0 }, 20.0 + i as f32),
                text: "x".into(),
                lit: false,
            })
            .collect();
        let placed = place_labels(&tags, card, picture, 30.0, 0.0, 300.0);
        assert_eq!(placed.len(), tags.len());
        for p in &placed {
            assert!(card.contains_rect(p.label), "{p:?}");
            assert_eq!(p.left, tags[p.index].at.x < picture.center().x);
            assert!(if p.left { p.label.right() <= picture.left() } else { p.label.left() >= picture.right() });
        }
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                if a.left == b.left {
                    assert!(!a.label.shrink(0.5).intersects(b.label.shrink(0.5)), "{a:?} {b:?}");
                }
            }
        }
    }
}
