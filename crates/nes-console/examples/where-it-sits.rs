//! Where a cartridge's program is, when it is not where it should be:
//! the opcode fetch addresses the console's CPU visits over a span of
//! frames, counted, with the busiest few printed.
//!
//!   cargo run --release -p nes-console --example where-it-sits -- game.nes [frames]
//!
//! A game that never draws is either waiting on something that has not
//! happened or looping over a byte it does not like. The histogram says
//! which address the loop is at. Then:
//!
//!   WRITES=1    every write above $2000, with the half-cycle it landed
//!               on: what the program managed before it stopped
//!   TRAP=xxxx   the hundred opcode fetches before the program first
//!               reaches that address, which is how it got there
//!   BUS=a-b     every CPU cycle in that half-cycle range, address, byte
//!               and direction, opcode fetches marked
//!
//! Used in that order it walks a crash back to its instruction: Super
//! Mario Bros. 2 sat on $FFF0/$FFF3, the trap showed it marching through
//! a bank's $FF padding from $EEE3, and the bus trace named the branch
//! that sent it there ($ED10, which should have gone to $ECE3).
//!
//! This is a locator, not an oracle: it says where, and the ROM's own
//! code says why.

use std::collections::HashMap;

use nes_console::{ines, Alignment, Console};
use v6502_pins::PinEngine;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let rom = ines::parse(&bytes).expect("iNES");
    println!("mapper {}, prg {} KiB, chr {} KiB{}", rom.mapper, rom.prg.len() / 1024, rom.chr.len() / 1024, if rom.chr_ram { " (CHR RAM)" } else { "" });
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let mut c = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
    let writes = std::env::var_os("WRITES").is_some();
    // TRAP=addr prints the last hundred opcode fetches before the
    // program first reaches that address, which is what says how it got
    // there when the address is a crash.
    let trap: Option<u16> = std::env::var("TRAP").ok().and_then(|v| u16::from_str_radix(&v, 16).ok());
    let bus: Option<(u64, u64)> = std::env::var("BUS").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    let mut last_bus: Option<(u64, u16)> = None;
    let mut history: std::collections::VecDeque<u16> = std::collections::VecDeque::new();
    let mut sprung = false;
    let mut seen: HashMap<u16, u64> = HashMap::new();
    let mut last_write = String::new();
    let mut shown = 0;
    for _ in 0..frames {
        let target = c.frames.len() + 1;
        while c.frames.len() < target {
            c.master_half_step();
            let f = c.cpu.pins();
            if f.sync && f.clk0 {
                *seen.entry(f.ab).or_default() += 1;
                if let Some(t) = trap {
                    // A fetch stands at the pins for several master
                    // half-steps; only its first look is an instruction.
                    if history.back() == Some(&f.ab) {
                        continue;
                    }
                    history.push_back(f.ab);
                    if history.len() > 100 {
                        history.pop_front();
                    }
                    if f.ab == t && !sprung {
                        sprung = true;
                        println!("first reached {t:04x} at CPU half-cycle {}; the hundred opcode fetches before it:", c.cpu_half_cycles);
                        let path: Vec<String> = history.iter().map(|a| format!("{a:04x}")).collect();
                        println!("{}", path.join(" "));
                    }
                }
            }
            // BUS=a-b prints every CPU cycle in that half-cycle range:
            // the address, the byte and which way it went. A crash is
            // read here, not reasoned about.
            if let Some((lo, hi)) = bus {
                let h = c.cpu_half_cycles;
                if (lo..=hi).contains(&h) && f.clk0 && last_bus != Some((h, f.ab)) {
                    last_bus = Some((h, f.ab));
                    println!("  hc {h}: {} {:04x} {:02x}{}", if f.rw { "read " } else { "write" }, f.ab, f.db, if f.sync { "   <- opcode" } else { "" });
                }
            }
            if writes && !f.rw && f.clk0 && (0x2000..0x8000).contains(&f.ab) {
                let line = format!("{:04x} <- {:02x}", f.ab, f.db);
                if line != last_write && shown < 400 {
                    println!("  hc {}: {line}", c.cpu_half_cycles);
                    last_write = line;
                    shown += 1;
                }
            }
        }
    }
    let mut v: Vec<(u16, u64)> = seen.into_iter().collect();
    v.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    let total: u64 = v.iter().map(|&(_, n)| n).sum();
    println!("{} distinct opcode addresses over {frames} frames, {total} fetches", v.len());
    for &(a, n) in v.iter().take(12) {
        println!("  {a:04x}: {n} ({:.1}%)", 100.0 * n as f64 / total as f64);
    }
}
