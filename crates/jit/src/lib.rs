//! A JIT for i960 code: blocks of the game's program translated to native
//! machine code at run time with Cranelift, from the ROM (or RAM) the
//! player was pointed at. Nothing game-derived is built into the binary.
//!
//! It is the static recompiler's back end moved to run time: the same
//! block rules, the same lowering ([`i960::lower`]), the same calls into
//! the interpreter for what isn't translated, and the same timing (one
//! `Host::retire` per instruction, a return to the dispatcher as soon as an
//! interrupt is due, a frame ends or the budget runs out), so a JIT run is
//! instruction for instruction an interpreted one. docs/jit.md has the
//! design and the measurements.
//!
//! The copro programs have JITs of their own on the same Cranelift setup
//! ([`mb86234`], [`mb86235`]): each word lowered as its interpreter's step
//! defines it, the interpreter's timing kept word for word (docs/jit.md,
//! "Copro JITs").
//!
//! Off the browser build only: on `wasm32` the crate is empty.

#[cfg(not(target_arch = "wasm32"))]
mod copro;
#[cfg(not(target_arch = "wasm32"))]
pub mod mb86234;
#[cfg(not(target_arch = "wasm32"))]
pub mod mb86235;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use copro::CoproStats;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
