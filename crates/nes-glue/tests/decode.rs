//! U3 against the SN74LS139A function table, and the two wired halves
//! against the NES-001 map. The M2 term is the test the sketch names:
//! A15 high with M2 low must not select the cartridge.

use nes_glue::decode::{cpu_decode, ls139_half, ppu_a13_n, romsel_n};

#[test]
fn the_function_table_row_for_row() {
    // The datasheet's table: G high forces all high; otherwise exactly
    // the output numbered by (B, A) is low.
    for g in [false, true] {
        for b in [false, true] {
            for a in [false, true] {
                let y = ls139_half(g, b, a);
                if g {
                    assert_eq!(y, [true; 4], "G high, B={b} A={a}");
                } else {
                    let sel = (b as usize) << 1 | a as usize;
                    for (i, &level) in y.iter().enumerate() {
                        assert_eq!(level, i != sel, "G low, B={b} A={a}, Y{i}");
                    }
                }
            }
        }
    }
}

#[test]
fn half_a_selects_ram_below_2000_and_the_ppu_below_4000_and_nothing_above() {
    for a in (0u32..0x10000).step_by(0x100) {
        let a = a as u16;
        let d = cpu_decode(a, true);
        assert_eq!(!d.ram_cs_n, a < 0x2000, "/RAM CS at {a:04x}");
        assert_eq!(!d.ppu_cs_n, (0x2000..0x4000).contains(&a), "/PPU CS at {a:04x}");
    }
    // Mirrors: the SRAM's own lines stop at A10, so every 2 KiB page below
    // $2000 selects; the PPU's stop at A2, so every 8 bytes below $4000 do.
    assert!(!cpu_decode(0x1fff, true).ram_cs_n);
    assert!(!cpu_decode(0x3ff8, true).ppu_cs_n);
}

#[test]
fn romsel_needs_a15_and_m2_together() {
    for a in [0x8000u16, 0xc000, 0xffff] {
        assert!(!romsel_n(a, true), "{a:04x} with M2 high selects the cartridge");
        assert!(romsel_n(a, false), "{a:04x} with M2 LOW must not: the M2 term");
    }
    for a in [0x0000u16, 0x7fff] {
        assert!(romsel_n(a, true), "{a:04x} is below the cartridge's window");
        assert!(romsel_n(a, false));
    }
    // Half A is disabled by A15, so the cartridge's window never also
    // selects RAM or the PPU.
    let d = cpu_decode(0x8000, true);
    assert!(d.ram_cs_n && d.ppu_cs_n && !d.romsel_n);
}

#[test]
fn ppu_a13_n_is_the_inverter() {
    assert!(ppu_a13_n(0x1fff));
    assert!(!ppu_a13_n(0x2000));
    assert!(!ppu_a13_n(0x3fff));
}
