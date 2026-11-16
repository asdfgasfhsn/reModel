//! What the menu shows of a save slot besides the state itself: when it was
//! saved and by which reModel, and its thumbnail, a small PNG kept beside
//! the state (`quick.save` has `quick.png`, `slot1.save` `slot1.png`). The
//! state file's format and hash don't change; a state without a thumbnail
//! (saved by an older reModel) just shows a placeholder.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A thumbnail's size in pixels: the game's 4:3 frame, twice the size the
/// slots page shows it at.
pub const THUMB_SIZE: (u32, u32) = (224, 168);

/// A slot that has been saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotInfo {
    /// When the state file was written.
    pub time: SystemTime,
    /// The reModel that wrote it, if the file says.
    pub made_by: Option<String>,
    /// This build can load it (the file's format isn't newer than its own).
    pub loads: bool,
    /// Its thumbnail, if there is one, and when that was written.
    pub thumb: Option<(PathBuf, SystemTime)>,
}

/// Where the thumbnail of the state at `save` is.
pub fn thumb_path(save: &Path) -> PathBuf {
    save.with_extension("png")
}

/// The slot at `save`, if it's saved: its file's time and header (the first
/// bytes only; the state isn't read).
pub fn read_info(save: &Path) -> Option<SlotInfo> {
    let time = std::fs::metadata(save).and_then(|m| m.modified()).ok()?;
    let mut head = Vec::new();
    std::fs::File::open(save).ok()?.take(128).read_to_end(&mut head).ok()?;
    let info = model2::save_info(&head);
    let thumb = thumb_path(save);
    let thumb = std::fs::metadata(&thumb).and_then(|m| m.modified()).ok().map(|t| (thumb, t));
    Some(SlotInfo {
        time,
        made_by: info.as_ref().and_then(|i| i.made_by.clone()),
        loads: info.is_some_and(|i| i.loads),
        thumb,
    })
}

/// Write the thumbnail `rgba` (`w` x `h`, alpha ignored) to `path` on its
/// own thread: encoding it isn't the game's business. Written beside its
/// name first so a reader never sees half a file.
pub fn write_thumb(path: PathBuf, w: u32, h: u32, rgba: Vec<u8>) {
    let spawned = std::thread::Builder::new().name("thumbnail".into()).spawn(move || {
        let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let png = png::encode_rgb(w as usize, h as usize, &rgb);
        let part = path.with_extension("png.part");
        let done = std::fs::write(&part, png).and_then(|_| std::fs::rename(&part, &path));
        if let Err(e) = done {
            eprintln!("{}: can't keep the thumbnail: {e}", path.display());
            let _ = std::fs::remove_file(&part);
        }
    });
    if let Err(e) = spawned {
        eprintln!("can't start the thumbnail thread: {e}");
    }
}

/// A thumbnail's PNG as `[w, h]` and RGBA, if it reads.
pub fn read_thumb(path: &Path) -> Option<([usize; 2], Vec<u8>)> {
    let img = png::decode(&std::fs::read(path).ok()?).ok()?;
    (img.width > 0 && img.height > 0 && img.rgba.len() == img.width * img.height * 4)
        .then_some(([img.width, img.height], img.rgba))
}

/// A date and time as the local clock shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    /// Days since 1970-01-01 of the local date.
    pub days: i64,
    pub minute: u32,
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (year, month 1 to 12, day) of `days` since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The local clock at `t` (UTC where the platform's zone isn't known).
pub fn local(t: SystemTime) -> Clock {
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };
    #[cfg(unix)]
    {
        // SAFETY: localtime_r fills the `tm` it is given from a valid time_t.
        let tm = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            let t: libc::time_t = secs as libc::time_t;
            if !libc::localtime_r(&t, &mut tm).is_null() {
                Some(tm)
            } else {
                None
            }
        };
        if let Some(tm) = tm {
            let days = days_from_civil(i64::from(tm.tm_year) + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32);
            return Clock { days, minute: (tm.tm_hour * 60 + tm.tm_min) as u32 };
        }
    }
    Clock { days: secs.div_euclid(86_400), minute: (secs.rem_euclid(86_400) / 60) as u32 }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Today 14:32", "Yesterday 21:08", "3 Nov 19:44" (with the year when it
/// isn't this one's).
pub fn describe(at: Clock, now: Clock) -> String {
    let time = format!("{:02}:{:02}", at.minute / 60, at.minute % 60);
    match now.days - at.days {
        0 => format!("Today {time}"),
        1 => format!("Yesterday {time}"),
        _ => {
            let (y, m, d) = civil_from_days(at.days);
            let year = civil_from_days(now.days).0;
            let y = if y == year { String::new() } else { format!(" {y}") };
            format!("{d} {}{y} {time}", MONTHS[m as usize - 1])
        }
    }
}

/// How long ago, roughly.
pub fn age(since: Duration) -> String {
    let s = since.as_secs();
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

/// The slot's "when" line: the date and the age.
pub fn when(t: SystemTime) -> String {
    format!("{} · {}", describe(local(t), local(SystemTime::now())), age(t.elapsed().unwrap_or_default()))
}

/// The slot's meta line: who saved it, and whether this build loads it.
pub fn meta(info: &SlotInfo) -> String {
    let by = info
        .made_by
        .as_deref()
        .map_or("an older reModel".to_string(), |v| format!("reModel {}", v.trim_start_matches('v')));
    format!("{by} · {}", if info.loads { "loads in this version" } else { "needs a newer reModel" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        let d = days_from_civil(2026, 11, 3);
        assert_eq!(civil_from_days(d), (2026, 11, 3));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28), 2, "2000 was a leap year");
    }

    #[test]
    fn dates_read_by_how_far_back() {
        let now = Clock { days: days_from_civil(2026, 11, 16), minute: 15 * 60 };
        let at = |days_back: i64, minute: u32| Clock { days: now.days - days_back, minute };
        assert_eq!(describe(at(0, 14 * 60 + 32), now), "Today 14:32");
        assert_eq!(describe(at(1, 21 * 60 + 8), now), "Yesterday 21:08");
        assert_eq!(describe(at(13, 19 * 60 + 44), now), "3 Nov 19:44");
        assert_eq!(describe(at(400, 9), now), "12 Oct 2025 00:09");
    }

    #[test]
    fn ages_and_meta_lines() {
        assert_eq!(age(Duration::from_secs(30)), "just now");
        assert_eq!(age(Duration::from_secs(12 * 60)), "12 min ago");
        assert_eq!(age(Duration::from_secs(5 * 3600)), "5 h ago");
        assert_eq!(age(Duration::from_secs(3 * 86400)), "3 days ago");
        let mut info = SlotInfo { time: SystemTime::now(), made_by: Some("v0.9.0".into()), loads: true, thumb: None };
        assert_eq!(meta(&info), "reModel 0.9.0 · loads in this version");
        info.loads = false;
        info.made_by = None;
        assert_eq!(meta(&info), "an older reModel · needs a newer reModel");
    }

    #[test]
    fn thumbnails_sit_beside_their_state() {
        assert_eq!(thumb_path(Path::new("a/saves/quick.save")), Path::new("a/saves/quick.png"));
        assert_eq!(thumb_path(Path::new("a/saves/slot3.save")), Path::new("a/saves/slot3.png"));
    }
}
