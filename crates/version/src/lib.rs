//! The build's version string, for display only: it never enters emulation
//! or state hashes. `v0.6.0-42-gabc1234` between releases, `v0.7.0` on a
//! tag, `-dirty` for uncommitted changes (as of the last rebuild); the
//! crate's version when git isn't available. `REMODEL_VERSION` in the
//! environment overrides it at build time.

pub const VERSION: &str = env!("REMODEL_BUILD_VERSION");
