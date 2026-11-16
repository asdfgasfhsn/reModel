//! `VERSION`: `git describe --tags --dirty --always` (v0.6.0-42-gabc1234),
//! or the crate's version where git or the history isn't there (a source
//! tarball). Rebuilt when HEAD, the refs or the tags move; `-dirty` is as of
//! the last such rebuild (watching the index would rebuild every binary on
//! each `git add`).

use std::path::PathBuf;
use std::process::Command;

/// The crate's folder as this build sees it (not `env!`'s, fixed when the
/// script was compiled: a target folder shared between checkouts keeps the
/// script, and would ask another, maybe removed, checkout).
fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(manifest_dir()).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=REMODEL_VERSION");
    let fallback = std::env::var("CARGO_PKG_VERSION").unwrap();
    let described = std::env::var("REMODEL_VERSION").ok().filter(|v| !v.is_empty()).or_else(|| {
        let v = git(&["describe", "--tags", "--dirty", "--always"])?;
        // Tracking: HEAD, its branch, packed refs and tags. A worktree's HEAD
        // is in its own git dir, the refs in the common one.
        let branch = git(&["symbolic-ref", "-q", "HEAD"]).unwrap_or_default();
        let places: [(&str, Vec<&str>); 2] =
            [("--absolute-git-dir", vec!["HEAD"]), ("--git-common-dir", vec!["packed-refs", "refs/tags", &branch])];
        for (flag, files) in places {
            if let Some(d) = git(&["rev-parse", flag]) {
                let d = PathBuf::from(d);
                let d = if d.is_relative() { manifest_dir().join(d) } else { d };
                for f in files.into_iter().filter(|f| !f.is_empty()) {
                    println!("cargo:rerun-if-changed={}", d.join(f).display());
                }
            }
        }
        Some(v)
    });
    // `--always` gives a bare hash when there are no tags (a shallow clone):
    // keep the crate version in front of it.
    let version = match described {
        Some(v) if v.starts_with('v') => v,
        Some(v) => format!("{fallback}-g{v}"),
        None => fallback,
    };
    println!("cargo:rustc-env=REMODEL_BUILD_VERSION={version}");
}
