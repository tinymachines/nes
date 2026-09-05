//! U3, a 74LS139 dual 2-to-4 line decoder, and the 74HC04 inverter that
//! gives the cartridge PPU /A13.
//!
//! AUTHORED from the SN74LS139A datasheet's function table (outputs are
//! active LOW; an enable held high forces all four outputs high) and the
//! NES-001 wiring:
//!
//! - half A: 1G = CPU A15, 1A = A13, 1B = A14. So with A15 low, 1Y0 (A14
//!   A13 = 00) is /RAM CS for $0000..$1FFF and 1Y1 (01) is /PPU CS for
//!   $2000..$3FFF; 1Y2 and 1Y3 ($4000..$7FFF) are not used on the board
//!   (the APU registers decode inside the 2A03, and a cartridge decodes
//!   $6000..$7FFF itself). With A15 high the half is disabled and both
//!   selects deassert.
//! - half B: 2G = ground (always enabled), 2A = M2, 2B = A15. 2Y3 (both
//!   high) is /ROMSEL at cartridge pin 50: not (A15 and M2). The M2 term
//!   is what a cartridge relies on to see only valid bus cycles, and it
//!   is what the test holds.
//!
//! Every function here takes and returns pin LEVELS, `true` = high, so
//! active-low outputs read `false` when they select. The `_n` suffix is
//! nes-bus's convention.

/// The 74LS139's function table for one half: (G, B, A) to the four
/// outputs' levels, Y0..Y3. Authored from the datasheet; the test
/// re-derives every row from it.
pub fn ls139_half(g_n: bool, b: bool, a: bool) -> [bool; 4] {
    if g_n {
        return [true; 4];
    }
    let selected = (b as usize) << 1 | a as usize;
    let mut y = [true; 4];
    y[selected] = false;
    y
}

/// Half A as wired: `(ram_cs_n, ppu_cs_n)` from CPU A13, A14, A15.
pub fn cpu_selects(a: u16) -> (bool, bool) {
    let y = ls139_half(a & 0x8000 != 0, a & 0x4000 != 0, a & 0x2000 != 0);
    (y[0], y[1])
}

/// Half B as wired: /ROMSEL from A15 and M2.
pub fn romsel_n(a: u16, m2: bool) -> bool {
    ls139_half(false, a & 0x8000 != 0, m2)[3]
}

/// The whole CPU-side decode for one bus state, as the three active-low
/// lines the board produces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CpuDecode {
    pub ram_cs_n: bool,
    pub ppu_cs_n: bool,
    pub romsel_n: bool,
}

pub fn cpu_decode(a: u16, m2: bool) -> CpuDecode {
    let (ram_cs_n, ppu_cs_n) = cpu_selects(a);
    CpuDecode { ram_cs_n, ppu_cs_n, romsel_n: romsel_n(a, m2) }
}

/// The 74HC04 gate behind cartridge pin 58: PPU /A13 is PPU A13
/// inverted. The cartridge uses it to enable CIRAM for $2000..$3FFF
/// (nes-bus's NROM wires /CIRAM CE to it).
pub fn ppu_a13_n(ppu_a: u16) -> bool {
    ppu_a & 0x2000 == 0
}
