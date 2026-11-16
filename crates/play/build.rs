//! Embed the art: every `assets/bezels/<set>.webp` becomes an entry of
//! `BEZELS` (`bezel.rs`) and every `assets/posters/<set>.webp` one of
//! `POSTERS` (the launcher's cards, `art.rs`), so adding a game's art needs
//! no code.

use std::path::Path;

/// `assets/<dir>/*.webp` as a static table `name` of (set, bytes).
fn embed(dir: &str, name: &str, doc: &str) -> String {
    // The crate's folder now, not `env!`'s: that is fixed when the build
    // script is compiled, and a target folder shared between checkouts
    // keeps the script, so a checkout since removed embedded no art.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dir = Path::new(&manifest).join("../../assets").join(dir);
    println!("cargo:rerun-if-changed={}", dir.display());
    let read = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut entries: Vec<(String, String)> = read
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "webp" {
                return None;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            let set = path.file_stem()?.to_str()?.to_string();
            Some((set, path.canonicalize().ok()?.display().to_string()))
        })
        .collect();
    assert!(!entries.is_empty(), "{}: no .webp art", dir.display());
    entries.sort();
    let mut out = format!("/// {doc}\npub static {name}: &[(&str, &[u8])] = &[\n");
    for (set, path) in entries {
        out += &format!("    ({set:?}, include_bytes!({path:?})),\n");
    }
    out + "];\n"
}

fn main() {
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    let bezels = embed("bezels", "BEZELS", "Bezel art by set name (WebP), from `assets/bezels`.");
    std::fs::write(out.join("bezels.rs"), bezels).unwrap();
    let posters = embed(
        "posters",
        "POSTERS",
        "Poster art by set name (WebP), from `assets/posters` (`placeholder` for games without one).",
    );
    std::fs::write(out.join("posters.rs"), posters).unwrap();
}
