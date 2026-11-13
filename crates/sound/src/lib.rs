//! Model 2 sound: the SCSP (YMF292) sound chip, its effect DSP, the sound
//! board around them (68EC000 + SCSP), the original Model 2's sound board
//! (68000, two MultiPCMs and an OPN2 FM chip) and a WAV writer.
//!
//! Model 2-generic: nothing here knows about a particular game; the board
//! runs whatever sound program it is given. See `docs/hw/sound.md`.

pub mod board;
pub mod dsp;
pub mod multipcm;
pub mod opn2;
pub mod pcm_board;
pub mod scsp;
pub mod wav;

pub use board::SoundBoard;
pub use pcm_board::PcmBoard;
pub use scsp::Scsp;
