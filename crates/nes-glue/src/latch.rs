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

/// A PPU A12 watcher of the MMC3 kind, as an instrument for the latch's
/// test and for N5: counts A12's rising edges as seen at the latch's
/// falling edges, when a whole PPU address is valid on the cartridge
/// edge, and only when A12 had been low for at least `filter` such
/// falls first. With the filter at 0 it counts every rise, and the
/// latch's test shows why a mapper cannot use that: inside the sprite
/// window the two garbage nametable fetches between sprites take A12
/// low for two falls, so a raw count sees eight rises a line where the
/// MMC3 (whose filter is stated as A12 low for three or more M2 falls,
/// about nine dots) sees one. AUTHORED; the unit is latch falls, two per
/// fetch cycle of the PPU's measured schedule.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct A12Watcher {
    last_a12: bool,
    low_for: u32,
    pub filter: u32,
    pub rises: u32,
}

impl A12Watcher {
    /// The MMC3's filter in latch falls: three, which is six dots of the
    /// schedule, longer than the garbage-fetch gap and shorter than any
    /// background span.
    pub const MMC3_FILTER: u32 = 3;

    pub fn with_filter(filter: u32) -> A12Watcher {
        A12Watcher { filter, ..A12Watcher::default() }
    }

    /// Feed the address valid at one latch fall.
    pub fn latched(&mut self, ppu_a: u16) {
        let a12 = ppu_a & 0x1000 != 0;
        if a12 {
            if !self.last_a12 && self.low_for >= self.filter {
                self.rises += 1;
            }
            self.low_for = 0;
        } else {
            self.low_for += 1;
        }
        self.last_a12 = a12;
    }
}
