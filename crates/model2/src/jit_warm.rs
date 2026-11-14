//! A pre-flight for the JITs' hot-block list (docs/jit.md, "Pre-flight"):
//! a headless coin-start run on the JITs that notes the blocks they
//! translate in a list, so that a later boot translates them on the
//! workers before the game reaches them. Like a shader-cache precompile:
//! it changes when code is translated, never what the game does.

use crate::{geo::GeoMode, input::InputScript, jit, Machine};
use romset::RomSet;
use std::sync::Arc;

/// Instructions a warm run takes by default: `make bench`'s session
/// (boot, attract, coin, start, into the race).
pub const STEPS: u64 = 600_000_000;

/// Run `roms`' game for `steps` i960 instructions from power-on with a
/// coin and start (and the geometrizer in `geo`), on the JITs noting
/// their blocks in `list`, all on this thread (the translation is the
/// workers'). Returns the frames run.
pub fn run(roms: &RomSet, list: Arc<jit::HotList>, steps: u64, geo: GeoMode) -> Result<u64, String> {
    let script = InputScript::parse("coin-start", &roms.game.controls)?;
    let mut m = Machine::new(roms);
    m.bus.geo.set_mode(geo);
    m.bus.geo.set_threaded(false);
    m.enable_jit_with(jit::Config::default(), Some(list))?;
    let mut frame = m.frames;
    while m.cpu.icount < steps {
        m.run_slice(steps - m.cpu.icount).map_err(|s| format!("stopped: {s:?}"))?;
        if m.frames != frame {
            frame = m.frames;
            script.apply(frame, &mut m.bus);
        }
    }
    Ok(m.frames)
}
