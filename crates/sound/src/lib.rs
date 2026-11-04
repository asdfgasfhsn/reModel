//! Model 2 sound: the SCSP (YMF292) sound chip, its effect DSP, the sound
//! board around them (68EC000 + SCSP) and a WAV writer.
//!
//! Model 2-generic: nothing here knows about a particular game; the board
//! runs whatever sound program it is given. See `docs/hw/sound.md`.

pub mod board;
pub mod dsp;
pub mod scsp;
pub mod wav;

pub use board::SoundBoard;
pub use scsp::Scsp;
