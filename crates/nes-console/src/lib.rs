//! The console: the 2A03's ladder rung and the 2C02's on one master
//! clock, wired through the authored glue and the cartridge edge. This
//! crate is the one place in the family that plugs anything into
//! anything; the chips know nothing of it (the sketch's rule), and the
//! glue takes levels in and gives levels out.
//!
//! The unit is the MASTER HALF-STEP (one toggle of the 21.477272 MHz
//! clock). The CPU's half-cycle is twelve of them and the PPU's dot is
//! eight, so a CPU cycle is three dots, and `Console::master_half_step`
//! advances whichever chips fall on this tick. Where inside the twelve
//! and the eight each chip's edge falls is the power-on alignment, a
//! property of the two dividers' reset; the console records the one it
//! runs in its stamp (`Alignment`), and the alignment gate (N5 gate 1)
//! is what holds it to the switch-level chips.
//!
//! Wall-clock pacing is not here; a shell that wants real time paces the
//! frames this produces.

#![forbid(unsafe_code)]

pub mod board;
pub mod console;
pub mod ines;
pub mod testrom;

pub use board::Board;
pub use console::{Alignment, Console};
