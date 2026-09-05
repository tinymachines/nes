//! U8 against the SN74LS373 datasheet: transparent while LE is high,
//! holding from LE's fall. Then the sketch's test: an A12 watcher over
//! a synthetic line of the PPU's standard fetch schedule, sampling at
//! the latch's falls, sees exactly one rise per line.

use nes_glue::latch::{A12Watcher, Ls373};

#[test]
fn transparent_high_and_held_from_the_fall() {
    let mut u8_ = Ls373::new();
    assert_eq!(u8_.step(true, 0x12), (0x12, false));
    // Still high: follows a changing input.
    assert_eq!(u8_.step(true, 0x34), (0x34, false));
    // The fall: holds the value present as it fell.
    assert_eq!(u8_.step(false, 0x34), (0x34, true));
    // Low: the input may do anything, Q holds.
    assert_eq!(u8_.step(false, 0xff), (0x34, false));
    assert_eq!(u8_.step(false, 0x00), (0x34, false));
    // Rise again: transparent at once.
    assert_eq!(u8_.step(true, 0x56), (0x56, false));
}

#[test]
fn a_rising_edge_sample_agrees_only_while_the_input_holds_across_the_pulse() {
    // The 2C02 harness samples the address at ALE's rise. If AD is the
    // same for the whole pulse the two readings are one; if it changed
    // inside the pulse the latch (the datasheet) has the later value.
    let mut latch = Ls373::new();
    let pulse = [(false, 0x00u8), (true, 0xab), (true, 0xab), (false, 0xab)];
    let mut rise_sample = None;
    let mut prev = false;
    let mut q = 0;
    for &(le, ad) in &pulse {
        if le && !prev {
            rise_sample = Some(ad);
        }
        q = latch.step(le, ad).0;
        prev = le;
    }
    assert_eq!(Some(q), rise_sample, "stable AD: the harness's rise sample is the latch's Q");

    let mut latch = Ls373::new();
    let pulse = [(false, 0x00u8), (true, 0xab), (true, 0xcd), (false, 0xcd)];
    let mut rise_sample = None;
    let mut prev = false;
    let mut q = 0;
    for &(le, ad) in &pulse {
        if le && !prev {
            rise_sample = Some(ad);
        }
        q = latch.step(le, ad).0;
        prev = le;
    }
    assert_eq!(rise_sample, Some(0xab));
    assert_eq!(q, 0xcd, "AD moved inside the pulse: the datasheet keeps the value at the fall");
}

/// One visible line of the 2C02's standard fetch schedule as the
/// address bus shows it (the P3 plan's measured positions): background
/// tiles from pattern table 0 ($0xxx) on dots 1..256 and 321..336 with
/// the nametable and attribute fetches at $2xxx, sprites from pattern
/// table 1 ($1xxx) on dots 257..320. One ALE pulse per fetch.
fn standard_line() -> Vec<(bool, u16)> {
    let mut out = Vec::new();
    let mut pulse = |a: u16| {
        out.push((true, a));
        out.push((false, a));
    };
    for tile in 0..42u16 {
        let dot = 1 + tile * 8;
        if (257..321).contains(&dot) {
            // Sprite window: two garbage nametable fetches, then pattern 1.
            pulse(0x2000);
            pulse(0x2000);
            pulse(0x1000 | (tile & 7) << 4);
            pulse(0x1008 | (tile & 7) << 4);
        } else if dot <= 336 {
            pulse(0x2000 | tile);
            pulse(0x23c0 | (tile >> 2));
            pulse((tile & 0xff) << 4);
            pulse((tile & 0xff) << 4 | 8);
        }
    }
    out
}

#[test]
fn an_a12_watcher_at_the_latch_falls_sees_one_rise_per_line_only_with_the_mmc3_filter() {
    let mut latch = Ls373::new();
    let mut raw = A12Watcher::with_filter(0);
    let mut mmc3 = A12Watcher::with_filter(A12Watcher::MMC3_FILTER);
    let lines = 240;
    for _ in 0..lines {
        for (ale, a) in standard_line() {
            let (_, fell) = latch.step(ale, a as u8);
            if fell {
                raw.latched(a);
                mmc3.latched(a);
            }
        }
    }
    assert_eq!(raw.rises, 8 * lines, "unfiltered: every sprite's pattern fetch after a garbage nametable fetch is a rise");
    assert_eq!(mmc3.rises, lines, "filtered: one rise per line, at the sprite window's start");
}
