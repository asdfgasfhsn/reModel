//! Zilog Z80: interpreter.
//!
//! Board-agnostic; the first user is the Model 2 DSB1 music board, whose Z80
//! drives the MPEG decoder. Written from Zilog's Z80 CPU User Manual and the
//! publicly documented undocumented behaviour (flag bits 3 and 5, MEMPTR,
//! the index-register halves, SLL, DDCB register copies), and checked
//! against the zexdoc and zexall instruction exercisers (`make z80-test`).

pub mod cpu;

pub use cpu::{Bus, Cpu};
