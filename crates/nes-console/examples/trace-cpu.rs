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
    let cart: Nrom = match std::env::var("ROM") {
        Ok(path) => {
            let rom = nes_console::ines::parse(&std::fs::read(path).unwrap()).unwrap();
            rom.nrom().unwrap()
        }
        Err(_) if std::env::var_os("POLL").is_some() => {
            // loop: BIT $2002; BPL loop; JMP loop
            let mut p = vec![0x2cu8, 0x02, 0x20, 0x10, 0xfb, 0x4c, 0x00, 0x80];
            p.resize(0x8000, 0xea);
            p[0x7ffc] = 0x00;
            p[0x7ffd] = 0x80;
            Nrom::new(p, chr(), Mirroring::Vertical).unwrap()
        }
        Err(_) => Nrom::new(program(), chr(), Mirroring::Vertical).unwrap(),
    };
    let chr_ram = std::env::var("ROM").ok().map(|_| vec![0u8; 0x2000]);
    let mut c = Console::new(Box::new(cart), chr_ram, Alignment::default());
    let watch: Vec<u16> = std::env::var("WATCH").ok().map(|w| w.split(',').map(|x| u16::from_str_radix(x, 16).unwrap()).collect()).unwrap_or_default();
    println!("h=0 {}", line(&c.cpu.pins()));
    let mut last = c.cpu_half_cycles;
    while c.cpu_half_cycles < to {
        c.master_half_step();
        if c.cpu_half_cycles != last {
            last = c.cpu_half_cycles;
            let f = c.cpu.pins();
            if f.sync && f.clk0 && watch.contains(&f.ab) {
                let (a, x, y, sp, p, pc) = c.cpu.core.registers();
                println!("fetch at {:04x} h={last}: A={a:02x} X={x:02x} Y={y:02x} S={sp:02x} P={p:02x} PC={pc:04x}", f.ab);
                continue;
            }
            if !watch.is_empty() {
                continue;
            }
            if std::env::var_os("IRQS").is_some() {
                // Interrupt entries: a fetch at the IRQ or NMI vector's target.
                let apu = c.cpu.apu.borrow();
                static mut LAST_IRQ: bool = false;
                let irq_now = apu.frame_irq || apu.dmc.irq;
                let changed = unsafe { irq_now != LAST_IRQ };
                unsafe { LAST_IRQ = irq_now };
                if changed {
                    println!("h={last} APU irq line {} (frame_irq={} dmc={})", if irq_now { "ASSERTED" } else { "released" }, apu.frame_irq, apu.dmc.irq);
                }
                continue;
            }
            if std::env::var_os("READS2002").is_some() {
                if f.rw && !f.clk0 && f.ab & 0xe007 == 0x2002 {
                    let ppu = c.board.borrow().ppu.position();
                    println!("h={last} $2002 -> {:02x} ppu=({},{}) into_dot={}", f.db, ppu.line, ppu.dot, c.board.borrow().half_steps_into_dot);
                }
                continue;
            }
            let interesting = (!f.rw && f.clk0) || (f.rw && !f.clk0 && f.ab == 0x2002 && f.db & 0x80 != 0) || (f.sync && !f.clk0 && f.ab >= 0x8100);
            if last >= from && (std::env::var_os("ALL").is_some() || interesting) {
                let ppu = c.board.borrow().ppu.position();
                println!("h={last:>6} {} ppu=({},{}) open_bus={:02x}", line(&f), ppu.line, ppu.dot, c.board.borrow().open_bus);
            }
        }
    }
}
