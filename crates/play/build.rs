//! Embed the bezel art: every `assets/bezels/<set>.webp` becomes an entry
//! of `BEZELS` (`bezel.rs`), so adding a game's bezel needs no code.

use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/bezels");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<(String, String)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
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
    entries.sort();
    let mut out = String::from(
        "/// Bezel art by set name (WebP), from `assets/bezels`.\npub static BEZELS: &[(&str, &[u8])] = &[\n",
    );
    for (set, path) in entries {
        out += &format!("    ({set:?}, include_bytes!({path:?})),\n");
    }
    out += "];\n";
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("bezels.rs");
    std::fs::write(dest, out).unwrap();
}
