//! `soundtrack-import`: gather a folder of audio files into a soundtrack
//! directory. WAV and MP2 are copied; other formats go through `ffmpeg`, so
//! the core only ever reads WAV and MP2.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Extensions taken as audio. WAV and MP2 are read natively.
const AUDIO: &[&str] = &[
    "wav", "mp2", "mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "wma", "aif", "aiff", "wv", "ape", "mka", "webm",
];

/// The track number a file name starts with (`03 - Title.flac` is 3).
fn leading_number(stem: &str) -> Option<u8> {
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|&n| n > 0)
}

pub fn import(src: &Path, dest: &Path) -> Result<(), String> {
    let mut files: Vec<_> = std::fs::read_dir(src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no audio files in {}", src.display()));
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let ffmpeg = Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success());

    // Numbered files keep their number; the rest fill the gaps in order.
    let stems: Vec<String> =
        files.iter().map(|p| p.file_stem().unwrap_or_default().to_string_lossy().into_owned()).collect();
    let mut taken: Vec<u8> = stems.iter().filter_map(|s| leading_number(s)).collect();
    let mut tracks = vec![];
    for s in &stems {
        let n = match leading_number(s) {
            Some(n) if tracks.iter().all(|&t| t != n) => n,
            _ => {
                let n = (1..=u8::MAX).find(|n| !taken.contains(n)).ok_or("too many files")?;
                taken.push(n);
                n
            }
        };
        tracks.push(n);
    }

    let mut manifest = String::new();
    let _ = writeln!(manifest, "# Made by `remodel soundtrack-import` from {}; see docs/mods.md.", src.display());
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by_key(|&i| tracks[i]);
    for i in order {
        let (path, n) = (&files[i], tracks[i]);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let name = if ext == "wav" || ext == "mp2" {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            std::fs::copy(path, dest.join(&name)).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("track {n:2}: copied {name}");
            name
        } else if ffmpeg {
            let name = format!("{}.wav", stems[i]);
            let out = Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(path)
                .args(["-map", "0:a:0", "-ac", "2", "-c:a", "pcm_s16le"])
                .arg(dest.join(&name))
                .output()
                .map_err(|e| format!("ffmpeg: {e}"))?;
            if !out.status.success() {
                return Err(format!("ffmpeg failed on {}: {}", path.display(), String::from_utf8_lossy(&out.stderr)));
            }
            println!("track {n:2}: converted {} -> {name}", path.display());
            name
        } else {
            println!("track {n:2}: skipped {} (needs ffmpeg on PATH to convert to WAV)", path.display());
            continue;
        };
        let _ = write!(manifest, "\n[track {n}]\nfile = {name}\n");
    }
    let tracks_txt = dest.join("tracks.txt");
    if tracks_txt.exists() {
        println!("{} exists, left alone; the new entries would be:\n{manifest}", tracks_txt.display());
    } else {
        std::fs::write(&tracks_txt, &manifest).map_err(|e| format!("{}: {e}", tracks_txt.display()))?;
        println!("wrote {}", tracks_txt.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn leading_number() {
        assert_eq!(super::leading_number("03 - Title"), Some(3));
        assert_eq!(super::leading_number("12"), Some(12));
        assert_eq!(super::leading_number("Title"), None);
        assert_eq!(super::leading_number("0 intro"), None);
    }
}
