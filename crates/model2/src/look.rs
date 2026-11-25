//! Looks (docs/uplift-studio.md, "Looks"; docs/texture-packs.md, "Looks"):
//! a render profile shipped with a texture pack, or on its own: ink
//! outlines, toon lighting with halftone, speed lines, and per-texture tags.
//! A modern enhancement, not hardware; render only, nothing here touches
//! the machine. This module is the file format and the parameters; the GPU
//! renderer draws them (`gpu::Renderer::set_look`).
//!
//! `look.txt` is line-based UTF-8 text like `pack.txt`: a keyword, a tab (or
//! spaces) and its value; `#` comments and blank lines skipped; `format`
//! first. Every parameter has a default, so a look file names only what it
//! changes. `tag` lines give a texture or surface its own treatment.

use crate::enhance::TexInfo;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// The file's name, in a pack directory or on its own.
pub const FILE: &str = "look.txt";
/// The highest `format` this build reads. It rises only when the meaning
/// of something a reader already understands changes; new parameters and
/// tag keys are added without a new format (readers ignore what they don't
/// know and say so).
pub const FORMAT: u32 = 1;

/// How a parameter is edited and written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// On or off (0 or 1).
    Flag,
    /// A whole number in `min..=max`; 0 is allowed where it means off.
    Steps,
    /// A number in `min..=max`.
    Number,
    /// `#rrggbb`.
    Colour,
}

/// One look parameter: its name in files and `--look-set`, its range and
/// what it does.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    /// The step of a slider or a pad press.
    pub step: f32,
    /// Shown by the player's Advanced page.
    pub label: &'static str,
    pub help: &'static str,
}

#[allow(clippy::too_many_arguments)]
const fn p(
    name: &'static str,
    kind: Kind,
    default: f32,
    min: f32,
    max: f32,
    step: f32,
    label: &'static str,
    help: &'static str,
) -> Param {
    Param { name, kind, default, min, max, step, label, help }
}

/// Every parameter, in the order the Advanced page and the docs list them.
/// Colours are three entries each (`_r`, `_g`, `_b` in `Params::values`,
/// 0 to 1) and are written as `#rrggbb`.
pub const PARAMS: [Param; 23] = [
    p("outline", Kind::Flag, 1.0, 0.0, 1.0, 1.0, "Ink outlines", "Lines where the picture's depth jumps and where one texture meets another."),
    p("outline_weight", Kind::Number, 1.5, 0.0, 8.0, 0.25, "Line weight", "Thickness in pixels on a 1080-line picture; it scales with the output's height."),
    p("outline_opacity", Kind::Number, 0.9, 0.0, 1.0, 0.05, "Line opacity", "How solid the ink is."),
    p("outline_depth", Kind::Number, 1.0, 0.0, 4.0, 0.1, "Depth lines", "How readily a jump in depth, or a crease, draws a line. 0 draws none from depth."),
    p("outline_ids", Kind::Number, 0.6, 0.0, 1.0, 0.05, "Texture lines", "Lines where one texture meets another at the same depth. 0 draws none."),
    p("outline_silhouette", Kind::Number, 1.0, 0.5, 4.0, 0.1, "Outer line weight", "Width of lines at depth jumps and creases (a figure's outer contour), as a multiple of the line weight. Texture lines keep the plain weight. 1 draws all alike."),
    p("outline_fade_near", Kind::Number, 0.0, 0.0, 1000000.0, 250.0, "Fade starts at", "Distance (game units) where lines begin to thin out, so far scenery isn't ink noise. 0 leaves it to the scene: 100 times its typical distance, gone at 800 times."),
    p("outline_fade_far", Kind::Number, 0.0, 0.0, 1000000.0, 250.0, "Gone at", "Distance (game units) where lines have faded out completely (used when the start is set)."),
    p("outline_r", Kind::Colour, 0.07, 0.0, 1.0, 0.0, "Ink colour", "The colour of the lines."),
    p("outline_g", Kind::Colour, 0.06, 0.0, 1.0, 0.0, "", ""),
    p("outline_b", Kind::Colour, 0.10, 0.0, 1.0, 0.0, "", ""),
    p("rim", Kind::Number, 0.0, 0.0, 1.0, 0.05, "Rim light", "A light edge just inside the silhouettes of textures tagged with `rim`."),
    p("toon_steps", Kind::Steps, 3.0, 0.0, 6.0, 1.0, "Toon steps", "The game's lighting in this many hard bands (2 to 6); 0 leaves it smooth. Needs smooth shading, which the style turns on."),
    p("toon_floor", Kind::Number, 0.45, 0.0, 1.0, 0.05, "Shadow level", "How bright the darkest band is, of the lit one."),
    p("toon_cut", Kind::Number, 0.35, 0.0, 0.9, 0.05, "Band start", "Brightness below which everything is the darkest band."),
    p("toon_gain", Kind::Number, 1.0, 0.5, 4.0, 0.1, "Band contrast", "Stretches the lighting before it is cut into bands, so a model's narrow range of light spans several. Above 1 shows more steps on lit models; 1 leaves the game's light as it is."),
    p("halftone", Kind::Number, 0.0, 0.0, 1.0, 0.05, "Halftone dots", "Dots of ink in the darkest band. 0 draws none."),
    p("halftone_pitch", Kind::Number, 6.0, 2.0, 24.0, 0.5, "Dot pitch", "Distance between dots in pixels on a 1080-line picture."),
    p("halftone_ramp", Kind::Number, 0.0, 0.0, 1.0, 0.05, "Dot growth", "How much the dots swell towards the darkest light, as a printed tone does. 0 keeps every dot the same size."),
    p("halftone_angle", Kind::Number, 45.0, 0.0, 90.0, 5.0, "Dot angle", "Rotation of the dot grid in degrees."),
    p("halftone_r", Kind::Colour, 0.05, 0.0, 1.0, 0.0, "Dot colour", "The colour of the dots."),
    p("halftone_g", Kind::Colour, 0.04, 0.0, 1.0, 0.0, "", ""),
    p("halftone_b", Kind::Colour, 0.09, 0.0, 1.0, 0.0, "", ""),
];

/// `speed_lines` is separate: off by default and not in the Advanced page's
/// first rows.
pub const SPEED_LINES: Param = p(
    "speed_lines",
    Kind::Number,
    0.0,
    0.0,
    1.0,
    0.05,
    "Speed lines",
    "Streaks radiating from the middle of the picture (approximate: a fixed amount, not driven by speed).",
);

/// The names of the colour parameters (three entries make one).
const COLOURS: [(&str, &str); 2] = [("outline_colour", "outline_r"), ("halftone_colour", "halftone_r")];

/// A look's numbers: one per entry of [`PARAMS`], then `speed_lines`.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub values: [f32; PARAMS.len() + 1],
}

impl Default for Params {
    fn default() -> Self {
        let mut values = [0.0; PARAMS.len() + 1];
        for (v, p) in values.iter_mut().zip(PARAMS.iter()) {
            *v = p.default;
        }
        values[PARAMS.len()] = SPEED_LINES.default;
        Params { values }
    }
}

/// The parameter `name`'s entry (`speed_lines` last), and its place.
pub fn param(name: &str) -> Option<(usize, &'static Param)> {
    if name == SPEED_LINES.name {
        return Some((PARAMS.len(), &SPEED_LINES));
    }
    PARAMS.iter().enumerate().find(|(_, p)| p.name == name)
}

fn hex(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn unhex(s: &str) -> Option<[f32; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8].map(|b| b as f32 / 255.0))
}

impl Params {
    /// Parameter `name` as text (a colour name gives `#rrggbb`).
    pub fn get(&self, name: &str) -> Option<String> {
        if let Some((_, first)) = COLOURS.iter().find(|c| c.0 == name) {
            let (i, _) = param(first)?;
            return Some(hex([self.values[i], self.values[i + 1], self.values[i + 2]]));
        }
        let (i, p) = param(name)?;
        let v = self.values[i];
        Some(match p.kind {
            Kind::Flag => (if v >= 0.5 { "on" } else { "off" }).to_string(),
            Kind::Steps => format!("{}", v as u32),
            _ => format!("{v}"),
        })
    }

    /// Set parameter `name` from text (`on`/`off` for a flag, `#rrggbb` for
    /// a colour); an error says what is wrong. Numbers outside the range
    /// are refused, not clamped, so a typo in a file is seen.
    pub fn set(&mut self, name: &str, text: &str) -> Result<(), String> {
        if let Some((_, first)) = COLOURS.iter().find(|c| c.0 == name) {
            let c = unhex(text).ok_or_else(|| format!("`{name}` wants a colour as #rrggbb, not `{text}`"))?;
            let (i, _) = param(first).ok_or("no such colour")?;
            self.values[i..i + 3].copy_from_slice(&c);
            return Ok(());
        }
        let (i, p) = param(name).ok_or_else(|| format!("no look parameter `{name}`"))?;
        let v = match (p.kind, text) {
            (Kind::Flag, "on" | "true" | "1") => 1.0,
            (Kind::Flag, "off" | "false" | "0") => 0.0,
            (Kind::Flag, _) => return Err(format!("`{name}` is `on` or `off`, not `{text}`")),
            (Kind::Colour, _) => {
                return Err(format!(
                    "`{name}` is part of a colour: set `{}_colour`",
                    name.trim_end_matches(['r', 'g', 'b']).trim_end_matches('_')
                ))
            }
            (Kind::Steps, _) => {
                let n: u32 = text.parse().map_err(|_| format!("`{name}` wants a whole number, not `{text}`"))?;
                if n == 1 || n as f32 > p.max {
                    return Err(format!("`{name}` is 0 (off) or 2 to {}, not {n}", p.max));
                }
                n as f32
            }
            (Kind::Number, _) => {
                let v: f32 = text.parse().map_err(|_| format!("`{name}` wants a number, not `{text}`"))?;
                if !v.is_finite() || v < p.min || v > p.max {
                    return Err(format!("`{name}` is {} to {}, not {text}", p.min, p.max));
                }
                v
            }
        };
        self.values[i] = v;
        Ok(())
    }

    /// A numeric parameter's value.
    pub fn num(&self, name: &str) -> f32 {
        param(name).map_or(0.0, |(i, _)| self.values[i])
    }

    /// Whether the ink outline pass has anything to draw.
    pub fn outline_on(&self) -> bool {
        self.num("outline") >= 0.5 && self.num("outline_weight") > 0.0 && self.num("outline_opacity") > 0.0
    }

    /// The toon steps in use (0, or 2 to 6).
    pub fn toon_steps(&self) -> u32 {
        self.num("toon_steps") as u32
    }

    /// Whether the halftone has anything to draw (it needs toon bands).
    pub fn halftone_on(&self) -> bool {
        self.toon_steps() >= 2 && self.num("halftone") > 0.0
    }

    /// Whether any screen-space pass is needed: outlines, rim light or
    /// speed lines (and with them the texture-ID target).
    pub fn needs_ids(&self) -> bool {
        self.outline_on() || self.num("speed_lines") > 0.0
    }

    /// Whether anything at all changes the picture.
    pub fn any(&self) -> bool {
        self.needs_ids() || self.toon_steps() >= 2
    }
}

/// One texture's treatment (`tag STEM key=value ...`); a field left out
/// follows the look's parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tag {
    /// `outline=W`: this texture's line weight as a multiple of the look's
    /// (`0` or `none`: never outlined, as HUD pieces and signs should be).
    pub outline: Option<f32>,
    /// `toon=N` (2 to 6) or `toon=off`: this texture's lighting bands.
    pub toon: Option<u32>,
    /// `rim=X` (0 to 1): its rim light.
    pub rim: Option<f32>,
    /// `emissive`: self-lit, drawn at full brightness whatever the game's
    /// lighting says.
    pub emissive: bool,
    /// Other `k=v` fields, as written: ignored.
    pub other: Vec<String>,
}

/// A look: its parameters and per-texture tags.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub format: u32,
    pub name: Option<String>,
    pub game: Option<String>,
    pub params: Params,
    /// `tag` lines by stem (`tex_<key>_<w>x<h>` or `surf_<key>_<w>x<h>`).
    pub tags: BTreeMap<String, Tag>,
    /// Lines with a keyword this build doesn't know, as written: ignored.
    pub unknown: Vec<String>,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            format: FORMAT,
            name: None,
            game: None,
            params: Params::default(),
            tags: BTreeMap::new(),
            unknown: vec![],
        }
    }
}

fn tag_stem_ok(stem: &str) -> bool {
    let rest = stem.strip_prefix("tex_").or_else(|| stem.strip_prefix("surf_"));
    let Some((key, size)) = rest.and_then(|r| r.split_once('_')) else { return false };
    let size_ok = size
        .split_once('x')
        .is_some_and(|(w, h)| !w.is_empty() && !h.is_empty() && w.bytes().chain(h.bytes()).all(|b| b.is_ascii_digit()));
    key.len() == 16 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) && size_ok
}

impl Look {
    /// The look the Comic picture style draws with no pack: the defaults,
    /// with bold ink (5 px at 1080 lines, more readily from depth and
    /// texture boundaries), three toon bands with a clear dark one, and
    /// halftone dots in it. Tuned on Top Skater, Virtua Fighter 2, Daytona
    /// USA and Super GT (docs/texture-packs.md, "Looks").
    pub fn comic() -> Look {
        let mut l = Look { name: Some("Comic".into()), ..Look::default() };
        for (k, v) in [
            ("outline_weight", "5"),
            ("outline_depth", "1.5"),
            ("outline_ids", "0.8"),
            ("toon_floor", "0.35"),
            ("toon_cut", "0.3"),
            ("halftone", "0.6"),
        ] {
            let _ = l.params.set(k, v);
        }
        l
    }

    /// Parse `look.txt`. An error says what is wrong where; unknown
    /// keywords are not errors (`unknown`); a `format` above [`FORMAT`] is
    /// refused (`check`'s reason).
    pub fn parse(text: &str) -> Result<Look, String> {
        let mut l = Look::default();
        let mut first = true;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: String| format!("{FILE} line {}: {e}", i + 1);
            let (kw, value) = match line.split_once(['\t', ' ']) {
                Some((k, v)) => (k, v.trim()),
                None => (line, ""),
            };
            if first && kw != "format" {
                return Err(at("the first line must be `format N`".into()));
            }
            match kw {
                "format" if first => {
                    l.format = value
                        .parse::<u32>()
                        .ok()
                        .filter(|&n| n >= 1)
                        .ok_or_else(|| at(format!("`format` wants a whole number from 1, not `{value}`")))?;
                    // A newer format's other lines may mean anything.
                    l.check()?;
                }
                "format" => return Err(at("`format` given twice".into())),
                "name" | "game" => {
                    let v = (!value.is_empty()).then(|| value.to_string());
                    if kw == "name" {
                        l.name = v;
                    } else {
                        l.game = v;
                    }
                }
                "tag" => {
                    let mut fields = value.split_whitespace();
                    let stem = fields.next().ok_or_else(|| at("`tag` wants a texture stem".into()))?;
                    if !tag_stem_ok(stem) {
                        return Err(at(format!(
                            "`{stem}` isn't a texture or surface stem (tex_<key>_<w>x<h> or surf_<key>_<w>x<h>)"
                        )));
                    }
                    let mut t = Tag::default();
                    for f in fields {
                        let (k, v) = f.split_once('=').unwrap_or((f, ""));
                        match (k, v) {
                            ("emissive", "" | "on") => t.emissive = true,
                            ("outline", "none" | "off") => t.outline = Some(0.0),
                            ("outline", v) => {
                                t.outline = Some(
                                    v.parse::<f32>()
                                        .ok()
                                        .filter(|x| (0.0..=4.0).contains(x))
                                        .ok_or_else(|| at(format!("`outline` is none or 0 to 4, not `{v}`")))?,
                                )
                            }
                            ("toon", "off") => t.toon = Some(0),
                            ("toon", v) => {
                                t.toon = Some(
                                    v.parse::<u32>()
                                        .ok()
                                        .filter(|x| (2..=6).contains(x))
                                        .ok_or_else(|| at(format!("`toon` is off or 2 to 6, not `{v}`")))?,
                                )
                            }
                            ("rim", v) => {
                                t.rim = Some(
                                    v.parse::<f32>()
                                        .ok()
                                        .filter(|x| (0.0..=1.0).contains(x))
                                        .ok_or_else(|| at(format!("`rim` is 0 to 1, not `{v}`")))?,
                                )
                            }
                            _ => t.other.push(f.to_string()),
                        }
                    }
                    if l.tags.insert(stem.to_string(), t).is_some() {
                        return Err(at(format!("`tag {stem}` given twice")));
                    }
                }
                _ if param(kw).is_some() || COLOURS.iter().any(|c| c.0 == kw) => {
                    l.params.set(kw, value).map_err(at)?;
                }
                _ => l.unknown.push(line.to_string()),
            }
            first = false;
        }
        if first {
            return Err(format!("{FILE}: no `format N` line"));
        }
        if l.params.num("outline_fade_near") > 0.0
            && l.params.num("outline_fade_far") < l.params.num("outline_fade_near")
        {
            return Err(format!("{FILE}: `outline_fade_far` is nearer than `outline_fade_near`"));
        }
        Ok(l)
    }

    /// Whether the look changes anything drawn: a parameter does, or a tag
    /// makes a texture emissive or gives it its own toon steps.
    pub fn active(&self) -> bool {
        self.params.any() || self.tags.values().any(|t| t.emissive || t.toon.is_some_and(|n| n >= 2))
    }

    /// The look as `parse` reads it back (parameters at their defaults are
    /// left out).
    pub fn to_text(&self) -> String {
        let mut s = String::from("# reModel look (docs/texture-packs.md, \"Looks\")\n");
        let _ = writeln!(s, "format\t{}", self.format);
        for (k, v) in [("name", &self.name), ("game", &self.game)] {
            if let Some(v) = v {
                let _ = writeln!(s, "{k}\t{v}");
            }
        }
        let d = Params::default();
        for pm in PARAMS.iter().filter(|p| p.kind != Kind::Colour).chain([&SPEED_LINES]) {
            if self.params.get(pm.name) != d.get(pm.name) {
                let _ = writeln!(s, "{}\t{}", pm.name, self.params.get(pm.name).unwrap_or_default());
            }
        }
        for (name, _) in COLOURS {
            if self.params.get(name) != d.get(name) {
                let _ = writeln!(s, "{name}\t{}", self.params.get(name).unwrap_or_default());
            }
        }
        for (stem, t) in &self.tags {
            s += "tag\t";
            s += stem;
            match t.outline {
                Some(0.0) => s += "\toutline=none",
                Some(w) => {
                    let _ = write!(s, "\toutline={w}");
                }
                None => {}
            }
            match t.toon {
                Some(0) => s += "\ttoon=off",
                Some(n) => {
                    let _ = write!(s, "\ttoon={n}");
                }
                None => {}
            }
            if let Some(r) = t.rim {
                let _ = write!(s, "\trim={r}");
            }
            if t.emissive {
                s += "\temissive";
            }
            for o in &t.other {
                s += "\t";
                s += o;
            }
            s += "\n";
        }
        for l in &self.unknown {
            s += l;
            s += "\n";
        }
        s
    }

    /// Whether this build may use the look: the reason it must not (a
    /// `format` it doesn't know). Never guesses.
    pub fn check(&self) -> Result<(), String> {
        if self.format > FORMAT {
            return Err(format!(
                "the look is format {}, this reModel reads up to {FORMAT}: it needs a newer reModel",
                self.format
            ));
        }
        Ok(())
    }

    /// The look in `path` (a `look.txt`, or a directory holding one).
    /// `Ok(None)`: there is no such file.
    pub fn load(path: &Path) -> Result<Option<Look>, String> {
        let file = if path.is_dir() { path.join(FILE) } else { path.to_path_buf() };
        match std::fs::read_to_string(&file) {
            Ok(text) => Look::parse(&text).map(Some).map_err(|e| format!("{}: {e}", file.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", file.display())),
        }
    }

    /// One line for the reader to print about what it ignores, if anything.
    pub fn notice(&self) -> Option<String> {
        let first = self.unknown.first()?;
        let first = first.split(['\t', ' ']).next().unwrap_or(first);
        Some(format!(
            "{FILE}: ignoring {} line{} this reModel doesn't know (first: `{first}`)",
            self.unknown.len(),
            if self.unknown.len() == 1 { "" } else { "s" }
        ))
    }

    /// The tag for a texture: its image's (`tex_..`), else its surface's
    /// (`surf_..`).
    pub fn tag_for(&self, info: &TexInfo) -> Option<&Tag> {
        if self.tags.is_empty() {
            return None;
        }
        self.tags.get(&info.name()).or_else(|| self.tags.get(&info.surface_name()))
    }

    /// Apply `NAME=VALUE` overrides (`--look-set`); the first bad one is
    /// the error.
    pub fn set_all(&mut self, sets: &[String]) -> Result<(), String> {
        for s in sets {
            let (k, v) = s.split_once('=').ok_or_else(|| format!("`{s}` isn't NAME=VALUE"))?;
            self.params.set(k.trim(), v.trim())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "tex_0123456789abcdef_64x64";

    #[test]
    fn defaults_round_trip_to_nothing() {
        let l = Look::default();
        let text = l.to_text();
        assert!(!text.contains("outline_weight"), "{text}");
        assert_eq!(Look::parse(&text).unwrap(), l);
    }

    #[test]
    fn a_look_round_trips() {
        let mut l = Look { name: Some("Test comic".into()), ..Look::default() };
        l.params.set("outline_weight", "2.5").unwrap();
        l.params.set("outline_colour", "#102030").unwrap();
        l.params.set("toon_steps", "2").unwrap();
        l.params.set("halftone", "0.5").unwrap();
        l.params.set("speed_lines", "0.25").unwrap();
        l.params.set("outline", "off").unwrap();
        l.tags.insert(
            KEY.into(),
            Tag { outline: Some(0.0), toon: Some(0), rim: Some(0.5), emissive: true, other: vec![] },
        );
        l.tags.insert("surf_fedcba9876543210_8x16".into(), Tag { outline: Some(2.0), toon: Some(4), ..Tag::default() });
        l.unknown.push("shiny\t1".into());
        let text = l.to_text();
        let back = Look::parse(&text).unwrap();
        assert_eq!(back, l, "{text}");
        assert_eq!(back.params.get("outline_colour").unwrap(), "#102030");
    }

    #[test]
    fn an_unknown_format_is_refused_cleanly() {
        let e = Look::parse("format\t2\noutline_weight\t1\n").unwrap_err();
        assert!(e.contains("format 2") && e.contains("newer reModel"), "{e}");
        assert!(Look::parse("outline_weight\t1\n").unwrap_err().contains("format"));
        assert!(Look::parse("").unwrap_err().contains("no `format"));
        assert!(Look::parse("format\t0\n").is_err());
        assert!(Look::parse("format\t1\nformat\t1\n").unwrap_err().contains("twice"));
    }

    #[test]
    fn bad_values_say_where() {
        for (text, want) in [
            ("outline_weight\t99", "line 2"),
            ("toon_steps\t1", "line 2"),
            ("outline_colour\tred", "colour"),
            ("outline\tmaybe", "on` or `off"),
            ("tag\tnot-a-stem\toutline=none", "stem"),
            ("tag\ttex_0123456789abcdef_64x64\ttoon=9", "toon"),
            ("outline_fade_near\t900\noutline_fade_far\t100", "nearer"),
        ] {
            let e = Look::parse(&format!("format\t1\n{text}\n")).unwrap_err();
            assert!(e.contains(want), "{text}: {e}");
        }
        let dup = format!("format\t1\ntag\t{KEY}\toutline=0\ntag\t{KEY}\trim=1\n");
        assert!(Look::parse(&dup).unwrap_err().contains("twice"));
    }

    #[test]
    fn unknown_keywords_and_fields_are_kept_not_refused() {
        let l = Look::parse(&format!("format\t1\nnew_thing\t3\ntag\t{KEY}\toutline=none glow=2\n")).unwrap();
        assert_eq!(l.unknown, ["new_thing\t3"]);
        assert_eq!(l.tags[KEY].other, ["glow=2"]);
        assert!(l.notice().unwrap().contains("new_thing"));
    }

    #[test]
    fn set_all_applies_overrides() {
        let mut l = Look::default();
        l.set_all(&["outline_weight=3".into(), "toon_steps=2".into()]).unwrap();
        assert_eq!((l.params.num("outline_weight"), l.params.toon_steps()), (3.0, 2));
        assert!(l.set_all(&["nonsense=1".into()]).is_err());
        assert!(l.set_all(&["outline_weight".into()]).is_err());
    }

    #[test]
    fn what_needs_what() {
        let mut l = Look::default();
        assert!(l.params.outline_on() && l.params.needs_ids() && l.params.any());
        l.params.set("outline", "off").unwrap();
        l.params.set("toon_steps", "0").unwrap();
        assert!(!l.params.any());
        l.params.set("speed_lines", "0.5").unwrap();
        assert!(l.params.needs_ids());
    }
}
