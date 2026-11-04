//! Picks up recompiled MB86235 programs from `$REMODEL_RECOMP_DIR` (written
//! by `make recomp` to out/recomp/; derived from game code, so never in the
//! source tree) and generates a registry of them. Without it the registry
//! is empty and every program runs on the interpreter.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    // Opt-in: compiling recompiled programs takes minutes, so ordinary
    // builds (and `make check`) leave them out. `make recomp` sets this.
    println!("cargo:rerun-if-env-changed=REMODEL_RECOMP_DIR");
    let dir = std::env::var_os("REMODEL_RECOMP_DIR").map(PathBuf::from);
    if let Some(dir) = &dir {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
    let mut modules: Vec<(String, PathBuf)> = dir
        .iter()
        .flat_map(std::fs::read_dir)
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_prefix("tgp_")?.strip_suffix(".rs")?.to_string();
            println!("cargo:rerun-if-changed={}", e.path().display());
            Some((stem, e.path()))
        })
        .collect();
    modules.sort();
    let mut s = String::new();
    for (stem, path) in &modules {
        let _ = writeln!(
            s,
            "#[allow(clippy::all)]\nmod tgp_{stem} {{\n    include!({:?});\n}}",
            path.canonicalize().unwrap()
        );
    }
    s.push_str("/// The recompiled `step` for a program, if one was built.\n#[allow(clippy::match_single_binding)]\n");
    s.push_str("pub fn lookup<E: mb86235::cpu::External>(hash: u64) -> Option<fn(&mut mb86235::cpu::Tgp, &mut E) -> mb86235::cpu::Step> {\n    match hash {\n");
    for (stem, _) in &modules {
        let _ = writeln!(s, "        tgp_{stem}::HASH => Some(tgp_{stem}::step::<E>),");
    }
    s.push_str("        _ => None,\n    }\n}\n\n/// How many recompiled programs this build contains.\npub const COUNT: usize = ");
    let _ = writeln!(s, "{};", modules.len());
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("recomp_registry.rs");
    std::fs::write(out, s).unwrap();
}
