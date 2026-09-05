//! The console's CPU at the pins, half-cycle by half-cycle, over the
//! plumbing gate's cartridge: what the program does and when.
//!
//!   cargo run --release -p nes-console --example trace-cpu -- <from_hc> <to_hc>

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, program};
use nes_console::{Alignment, Console};
use v6502_pins::{line, PinEngine};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let from: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let to: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(120);
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    println!("h=0 {}", line(&c.cpu.pins()));
    let mut last = c.cpu_half_cycles;
    while c.cpu_half_cycles < to {
        c.master_half_step();
        if c.cpu_half_cycles != last {
            last = c.cpu_half_cycles;
            let f = c.cpu.pins();
            let interesting = (!f.rw && f.clk0) || (f.rw && !f.clk0 && f.ab == 0x2002 && f.db & 0x80 != 0) || (f.sync && !f.clk0 && f.ab >= 0x8100);
            if last >= from && (std::env::var_os("ALL").is_some() || interesting) {
                let ppu = c.board.borrow().ppu.position();
                println!("h={last:>6} {} ppu=({},{}) open_bus={:02x}", line(&f), ppu.line, ppu.dot, c.board.borrow().open_bus);
            }
        }
    }
}
