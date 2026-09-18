//! A bench script played on the model the way the head plays it on the
//! part, time included: `POWER ON` is the model's power-on, `WAIT s S`
//! runs the console `s` seconds of master clock, `RESET` holds the CPU's
//! /RESET low for the head's pulse (`RESET_HOLD_S`, half a second) and
//! releases it (`Console::reset_button`: the CPU's warm reset, the PPU
//! and the cartridge left as they were), and from each `RESET` the latch
//! indices count from zero again, as the bridge's do. `SET`, `AT` and
//! `WAIT n` are the bridge's words on those indices; `MODE`, `ARM`,
//! `TRIG`, `CAPTURE` and `POWER OFF` are the head's alone and skipped.
//!
//! `pad-log` plays only the `AT` lines, from power-on, which is not what
//! the part saw: every script on the part powers the console and then
//! pulses its reset, so the part's latch zero follows a warm reset and
//! the model's followed power. This is the runner for that difference.
//! Its first use (2026-09-18): the multicart's menu, which seemed to
//! ignore Start on the part after one reset, takes it here after one
//! reset and after two, at latch 203 as from power-on; the part, asked
//! again with the head's WAIT fixed, takes it too (nes-bench
//! open-items).
//!
//! Prints, per latch after the last `RESET`, `P <index> <fall line>
//! <fall dot>` (the strobe's fall in the PPU's frame, where poll-line.py
//! reads the part's), and a summary of the runs of latches by the line
//! they polled at, which is where a menu (line 119) and a game (line
//! 250) tell themselves apart.
//!
//!   cargo run --release -p nes-console --example bench-script -- rom.nes script.txt [tail latches]

use nes_console::{ines, Console};
use nes_glue::controller::Buttons;

/// Master half-steps per second: two per cycle of the 21.477272 MHz
/// master clock.
const HALF_STEPS_PER_S: f64 = 2.0 * 21_477_272.0;
/// The head's reset pulse (nes-bench head/headd.py `RESET_HOLD_S`).
const RESET_HOLD_S: f64 = 0.5;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let script = std::fs::read_to_string(&args[2]).expect("script");
    let tail: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let mut c = Console::with_prg_ram(cart, chr_ram, nes_console::knobs::alignment_from_env(), true);
    nes_console::knobs::configure_from_env(&mut c);
    c.board.borrow_mut().pads[0].log_polls = true;
    let mut base = 0u64;
    let mut last_at = 0u64;
    let mut resets = 0;
    for line in script.lines() {
        let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        match f.as_slice() {
            ["WAIT", s, "S"] => c.run_master((s.parse::<f64>().expect("seconds") * HALF_STEPS_PER_S) as u64),
            ["RESET"] => {
                c.reset_button((RESET_HOLD_S * HALF_STEPS_PER_S) as u64);
                base = c.board.borrow().pads[0].latches;
                resets += 1;
                println!("# RESET {resets} released at master {} (frame {}), latch {base} since power", c.master, c.frames.len());
            }
            ["SET", b] => c.set_pad(0, Buttons::from_byte(u8::from_str_radix(b, 16).expect("hex byte"))),
            ["AT", n, b] => {
                let n: u64 = n.parse().expect("latch");
                last_at = last_at.max(n);
                c.board.borrow_mut().pads[0].schedule.push((base + n, u8::from_str_radix(b, 16).expect("hex byte")));
            }
            ["WAIT", n] => {
                let n: u64 = n.parse().expect("latch");
                c.run_to_latch(base + n, 100_000).expect("the latch");
            }
            _ => {}
        }
    }
    // Past the last scheduled press by `tail` latches, so the answer to
    // it is on the record.
    let _ = c.run_to_latch(base + last_at + tail, 100_000);
    let b = c.board.borrow();
    let mut runs: Vec<(u64, u64, usize)> = Vec::new();
    for (i, (_, fall)) in b.latch_positions.iter().enumerate().skip(base as usize) {
        let k = i as u64 - base;
        println!("P {k} {} {}", fall.line, fall.dot);
        match runs.last_mut() {
            Some(r) if r.2 == fall.line => r.1 = k,
            _ => runs.push((k, k, fall.line)),
        }
    }
    let summary: Vec<String> = runs.iter().map(|(a, z, l)| format!("{a}..{z}@{l}")).collect();
    println!("# after reset {resets}: latches by poll line: {}", summary.join(" "));
}
