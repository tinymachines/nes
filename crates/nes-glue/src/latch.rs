//! U8, a 74LS373 octal transparent latch, between the PPU's multiplexed
//! AD0..AD7 and the cartridge's PPU A0..A7.
//!
//! AUTHORED from the SN74LS373 datasheet: while the latch enable (LE,
//! wired to the PPU's ALE) is HIGH the outputs follow the inputs; when LE
//! goes LOW the outputs hold whatever the inputs were as it fell. /OE is
//! grounded on the board, so the outputs always drive.
//!
//! Two consequences a reader of the 2C02's harness should know. The
//! harness samples the address on ALE's RISE (it reads the chip's own
//! address bus, which is the same value throughout the pulse), so it and
//! this latch agree whenever AD is stable across the pulse; the datasheet
//! decides the case where they differ, and `tests/latch.rs` shows both.
//! And PPU A8..A13 never pass through this part: they are the chip's own
//! pins, valid for the whole access, which is why an A12 watcher (an
//! MMC3's IRQ counter, in this crate an instrument) samples them at the
//! latch's falling edge rather than through it.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Ls373 {
    q: u8,
    le: bool,
}

impl Ls373 {
    pub fn new() -> Ls373 {
        Ls373::default()
    }

    /// Present LE and D for one instant; returns Q as the datasheet
    /// says, and whether this instant was LE's falling edge (the moment
    /// a downstream watcher samples on).
    pub fn step(&mut self, le: bool, d: u8) -> (u8, bool) {
        let fell = self.le && !le;
        if le {
            self.q = d;
        }
        self.le = le;
        (self.q, fell)
    }

    pub fn q(&self) -> u8 {
        self.q
    }
}

/// The PPU A12 watcher lives in `nes-bus` now, beside the board that
/// listens to it. It was authored here as an instrument while no mapper
/// existed, counting in latch falls with a filter of three; MMC3 (nes-bus
/// 0.1.3) counts the same line in dots with a filter of nine, and two
/// numbers for one filter is how they drift. Re-exported so this crate's
/// own test still reaches it.
pub use nes_bus::cart::A12Watcher;
