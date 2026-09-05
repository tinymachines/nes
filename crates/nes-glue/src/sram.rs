//! U1 and U4, two Toshiba TMM2115 2,048 x 8 static RAMs: the CPU's work
//! RAM at $0000..$07FF (selected by /RAM CS for the whole $0000..$1FFF,
//! so mirrored four times by the address lines the part does not see) and
//! the PPU's CIRAM (selected by the cartridge's /CIRAM CE, addressed by
//! PPU A0..A9 and the cartridge's CIRAM A10).
//!
//! AUTHORED as an ideal SRAM: a byte at each of 2,048 addresses, written
//! when /WE is low and read otherwise. The datasheet's access time is
//! recorded as a constant and used by nothing yet, the sketch's own
//! posture: it matters only when someone models contention.
//!
//! Power-on contents are undefined in silicon. Here they are a fixed
//! pattern a caller can see is not data (`POWER_ON_FILL`), never zero,
//! so a program that reads RAM before writing it fails visibly rather
//! than by luck.

/// The -12 speed grade's address access time in nanoseconds, from the
/// TMM2115 datasheet. Which grade a given NES-001 carries varies by
/// board revision; unused until contention is modelled, and labelled.
pub const ACCESS_TIME_NS_GRADE_12: u32 = 120;

pub const POWER_ON_FILL: u8 = 0xa5;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tmm2115 {
    bytes: Box<[u8; 2048]>,
}

impl Default for Tmm2115 {
    fn default() -> Tmm2115 {
        Tmm2115::new()
    }
}

impl Tmm2115 {
    pub fn new() -> Tmm2115 {
        Tmm2115 { bytes: Box::new([POWER_ON_FILL; 2048]) }
    }

    /// The part sees eleven address lines; anything above them is the
    /// board's mirroring, not the chip's, so the address is masked here.
    pub fn read(&self, a: u16) -> u8 {
        self.bytes[(a & 0x7ff) as usize]
    }

    pub fn write(&mut self, a: u16, v: u8) {
        self.bytes[(a & 0x7ff) as usize] = v;
    }

    /// One bus access as the part sees it: /CS low selects, /WE low
    /// writes; a deselected part leaves the data bus alone (`None`).
    pub fn access(&mut self, cs_n: bool, we_n: bool, a: u16, d: u8) -> Option<u8> {
        if cs_n {
            return None;
        }
        if we_n {
            Some(self.read(a))
        } else {
            self.write(a, d);
            None
        }
    }
}
