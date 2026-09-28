//! Saved states, end to end: the whole console saved where a CPU cycle
//! ends, loaded into a console just powered on from the same ROM, runs on
//! exactly as the console that never stopped. Each restored console runs
//! a whole frame beside the original under the same pads and the same
//! reset button: the CPU's pins at every master half-step, every picture
//! in full, every sound sample, and the whole state again at the end.
//!
//! The in-repo cartridge polls the pad, paints what it read, and plays a
//! looping DMC sample whose fetches land on the poll (so splits fall
//! inside DMC stalls, sprite DMAs, the pad's shifts and a reset's hold).
//! blargg's MMC3 test adds the scanline counter and its IRQ when
//! NES_TEST_ROMS has it (SKIP by name otherwise); REQUIRE_TEST_ROMS=1
//! insists. `NES_STATE_ROM=<path>` runs the same check on any cartridge
//! by hand (`--ignored`), which is how a real game is held to it without
//! the game ever entering the repository.
//!
//! MUTATE_STATE=1 is each chip's own sabotage (the 2A03's DMC fetch, the
//! 2C02's sprite units, the MMC3's counter), and this must go red.

use std::path::PathBuf;

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{ines, testrom, Alignment, Console, Sound};
use nes_glue::controller::Buttons;
use v6502_pins::PinEngine;

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// What the player does at a master half-step: pads, and the reset
/// button held for 2,000 half-steps once, as the page drives them.
fn inputs(c: &mut Console, m: u64, script: &[u8]) {
    if m % 97_003 == 0 {
        let b = script[(m / 97_003) as usize % script.len()];
        c.set_pad(0, Buttons::from_byte(b));
        c.set_pad(1, Buttons::from_byte(b.rotate_left(3)));
    }
    if m == 1_700_011 {
        c.res_n = false;
    }
    if m == 1_702_011 {
        c.res_n = true;
    }
}

/// Runs `make`'s console for `master_end` master half-steps, splitting at
/// the first cycle's end every `every`; each restored console is held to
/// the original for a frame. Returns (splits, splits inside a stall,
/// splits inside the reset's hold, half-steps the cartridge held /IRQ).
fn holds(make: &dyn Fn() -> Console, master_end: u64, every: u64) -> (usize, usize, usize, u64) {
    const RUN: u64 = 89_342 * 8;
    let script = noise(64, 0x5eed);
    let mut a = make();
    let mut live: Vec<(u64, Console)> = Vec::new();
    let (mut splits, mut stalled, mut in_reset, mut irq) = (0, 0, 0, 0u64);
    let mut next = 0u64;
    // Stalls and the reset's hold are a few percent of the time, so they
    // get splits of their own: the first cycle's end inside each, at most
    // one per 100,000 half-steps.
    let mut next_odd = 0u64;
    for m in 0..master_end + RUN {
        let odd = !a.cpu.pins().rdy || !a.res_n;
        let due = m >= next || (odd && m >= next_odd);
        if m < master_end && due && a.at_cycle_end() {
            let saved = a.save_state().expect("a state at a cycle's end");
            let mut b = make();
            b.load_state(&saved).expect("a state loads into the same cartridge");
            assert_eq!(b.save_state().unwrap(), saved, "at {m}: a loaded console saves the same state");
            stalled += !a.cpu.pins().rdy as usize;
            in_reset += !a.res_n as usize;
            live.push((m, b));
            splits += 1;
            // Both clocks move on whichever split this was: a regular one
            // falling due inside a long stall once split at every cycle's
            // end until the stall ended (a 240-frame run took an hour).
            if m >= next {
                next = m + every;
            }
            if odd {
                next_odd = m + 100_000;
            }
        }
        inputs(&mut a, m, &script);
        a.master_half_step();
        irq += a.board.borrow().cart.borrow().cart.irq() as u64;
        let pins = a.cpu.pins();
        let pictures = std::mem::take(&mut a.frames);
        let sound = a.sound.as_mut().map(|s| std::mem::take(&mut s.out));
        for (at, b) in live.iter_mut() {
            inputs(b, m, &script);
            b.master_half_step();
            assert_eq!(b.cpu.pins(), pins, "split at {at}, master {m}: the CPU's pins");
            let got = std::mem::take(&mut b.frames);
            assert_eq!(got.len(), pictures.len(), "split at {at}, master {m}: a picture on one side only");
            for (g, w) in got.iter().zip(&pictures) {
                assert!(g.colour == w.colour && g.emphasis == w.emphasis, "split at {at}: the picture at master {m}");
            }
            assert!(b.sound.as_mut().map(|s| std::mem::take(&mut s.out)) == sound, "split at {at}, master {m}: the sound");
        }
        if a.at_cycle_end() && live.iter().any(|(at, _)| m + 1 - at >= RUN) {
            let now = a.save_state().unwrap();
            live.retain(|(at, b)| {
                if m + 1 - at < RUN {
                    return true;
                }
                assert!(b.save_state().unwrap() == now, "split at {at}: the whole console a frame on");
                false
            });
        }
    }
    (splits, stalled, in_reset, irq)
}

fn with_sound(mut c: Console) -> Console {
    c.sound = Some(Sound::default());
    c
}

#[test]
fn the_console_restored_anywhere_runs_on_as_if_it_had_never_stopped() {
    let make = || {
        let cart = Nrom::new(testrom::pad_paint_program(true), testrom::chr(), Mirroring::Vertical).expect("NROM");
        with_sound(Console::with_prg_ram(Box::new(cart), None, Alignment::default(), true))
    };
    // Four frames, a split every 29,989 half-steps: odd against the
    // CPU's twenty-four and the dot's eight, so they walk both.
    let (splits, stalled, in_reset, _) = holds(&make, 4 * 89_342 * 8, 29_989);
    assert!(splits > 90, "{splits} splits");
    assert!(stalled > 0, "no split landed inside a stall");
    assert!(in_reset > 0, "no split landed inside the reset's hold");
}

fn console_from(path: &PathBuf) -> Console {
    let bytes = std::fs::read(path).expect("rom");
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    with_sound(Console::with_prg_ram(rom.cart().expect("a board this console has"), chr_ram, Alignment::default(), true))
}

#[test]
fn an_mmc3_restored_mid_frame_counts_its_scanlines_on() {
    let base = std::env::var("NES_TEST_ROMS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("roms").join("nes-test-roms"));
    let rom = base.join("mmc3_test_2").join("rom_singles").join("4-scanline_timing.nes");
    if !rom.is_file() {
        if std::env::var_os("REQUIRE_TEST_ROMS").is_some() {
            panic!("REQUIRE_TEST_ROMS set and {} is missing", rom.display());
        }
        eprintln!("SKIP: no {} (NES_TEST_ROMS)", rom.display());
        return;
    }
    // Long enough for the test to be running its IRQs, not initialising.
    let (splits, _, _, irq) = holds(&|| console_from(&rom), 40 * 89_342 * 8, 400_009);
    assert!(splits > 60, "{splits} splits");
    assert!(irq > 0, "the cartridge never raised its IRQ, so the counter was never tested");
}

/// By hand, on any cartridge: NES_STATE_ROM=<path> cargo test --release
/// --test state -- --ignored (NES_STATE_FRAMES, 240 by default).
#[test]
#[ignore]
fn a_cartridge_named_by_hand_restores_anywhere() {
    let path = PathBuf::from(std::env::var("NES_STATE_ROM").expect("NES_STATE_ROM names a cartridge"));
    let frames: u64 = std::env::var("NES_STATE_FRAMES").ok().and_then(|f| f.parse().ok()).unwrap_or(240);
    let t = std::time::Instant::now();
    let (splits, stalled, _, _) = holds(&|| console_from(&path), frames * 89_342 * 8, 1_000_003);
    eprintln!("{frames} frames: {splits} splits, {stalled} inside a stall, {:.1} s", t.elapsed().as_secs_f64());
}
