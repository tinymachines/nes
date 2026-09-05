//! The TMM2115 as an ideal 2 KiB SRAM: eleven address lines, /CS and
//! /WE, undefined power-on contents that are visibly not data.

use nes_glue::sram::{Tmm2115, ACCESS_TIME_NS_GRADE_12, POWER_ON_FILL};

#[test]
fn two_kib_addressed_by_eleven_lines() {
    let mut u1 = Tmm2115::new();
    for a in 0..0x800u16 {
        u1.write(a, (a as u8).wrapping_mul(7));
    }
    for a in 0..0x800u16 {
        assert_eq!(u1.read(a), (a as u8).wrapping_mul(7));
    }
    // Lines above A10 are not the part's: the board's mirroring.
    assert_eq!(u1.read(0x0800), u1.read(0x0000));
    assert_eq!(u1.read(0x1fff), u1.read(0x07ff));
}

#[test]
fn chip_select_and_write_enable_as_the_datasheet_says() {
    let mut u1 = Tmm2115::new();
    assert_eq!(u1.access(true, true, 0x10, 0x00), None, "deselected: the bus is left alone");
    assert_eq!(u1.access(true, false, 0x10, 0x5a), None, "deselected: no write either");
    assert_eq!(u1.read(0x10), POWER_ON_FILL);
    assert_eq!(u1.access(false, false, 0x10, 0x5a), None, "a write drives nothing back");
    assert_eq!(u1.access(false, true, 0x10, 0xff), Some(0x5a));
}

#[test]
fn power_on_contents_are_a_pattern_no_program_should_mistake_for_data() {
    let u1 = Tmm2115::new();
    assert_ne!(POWER_ON_FILL, 0);
    assert!((0..0x800u16).all(|a| u1.read(a) == POWER_ON_FILL));
    assert_eq!(ACCESS_TIME_NS_GRADE_12, 120, "the constant is a datasheet transcription, labelled unused");
}
