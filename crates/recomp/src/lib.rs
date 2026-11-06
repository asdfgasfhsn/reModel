//! The front ends call [`register`] at startup. In an ordinary build this
//! crate registers nothing, so every program runs on the interpreters. A
//! recompiled build (`RECOMP=1`, docs/recompiler.md) is made from the
//! generated workspace in out/recomp/, which patches this crate with a
//! generated registry of the same name that depends on every recompiled
//! program's crate and registers them all.

/// Hand the build's recompiled programs to model2 (none here).
pub fn register() {}
