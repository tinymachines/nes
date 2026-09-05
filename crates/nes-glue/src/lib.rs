//! The NES-001 mainboard's glue: the handful of parts between the two
//! chips and the cartridge edge, each a few lines held to its datasheet
//! and labelled AUTHORED. Nothing here goes through halfphi; nothing here
//! is measured off a die. The console sketch's milestone N4
//! (`docs/nes-end-to-end-v0_2.md`) is this crate, and each part carries
//! its own small test against the datasheet's own truth table or timing
//! statement. Where a value cannot be checked without a bench (the
//! SRAM's access time, the reset chain's hold) it is a named constant
//! with the capture that will replace it named beside it.
//!
//! The rule the sketch states and this crate keeps: a chip crate never
//! knows what is on the other side of its pins. The glue knows nothing
//! about the chips either; it takes levels in and gives levels out. The
//! console (N5) is the only thing that plugs anything into anything.
//!
//! Provenance for the wiring: the NES-001 schematic as documented on the
//! nesdev wiki (fetched 2026-09-05), the same provenance level as
//! nes-bus's pin tables; the part behaviours are their datasheets
//! (Texas Instruments SN74LS139A, SN74LS373, SN74LS368A; Toshiba
//! TMM2115; the 74HC04 family).

#![forbid(unsafe_code)]

pub mod controller;
pub mod decode;
pub mod latch;
pub mod reset;
pub mod sram;
