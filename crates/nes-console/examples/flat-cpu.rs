//! Rung 3 alone on a flat image of an NROM ROM (RAM below $2000, the
//! PRG above $8000, $2002 always reporting vblank so the program's waits
//! pass), logging the reads between consecutive $2007 writes: the same
//! instrument as the console's bus trace, minus the console.
//!
//!   cargo run --release -p nes-console --example flat-cpu -- rom.nes <half_cycles>

use v6502_micro::machine::{MicroBus, MicroCpu};
use v6502_pins::PinEngine;

struct Flat {
    ram: Vec<u8>,
    prg_ram: Vec<u8>,
    prg: Vec<u8>,
    log: Vec<String>,
    quiet: bool,
}

impl Flat {
    fn look(&self, a: u16) -> u8 {
        if a < 0x2000 {
            self.ram[(a & 0x7ff) as usize]
        } else if a == 0x2002 {
            0x80
        } else if (0x6000..0x8000).contains(&a) {
            self.prg_ram[(a - 0x6000) as usize]
        } else if a >= 0x8000 {
            self.prg[(a as usize - 0x8000) % self.prg.len()]
        } else {
            0
        }
    }
}

impl MicroBus for Flat {
    fn read(&mut self, a: u16) -> u8 {
        if !self.quiet {
            self.log.push(format!("{a:04x}"));
        }
        self.look(a)
    }
    fn write(&mut self, a: u16, v: u8) {
        if a < 0x2000 {
            self.ram[(a & 0x7ff) as usize] = v;
        } else if (0x6000..0x8000).contains(&a) {
            self.prg_ram[(a - 0x6000) as usize] = v;
        } else if a == 0x2007 && !self.quiet {
            let reads = std::mem::take(&mut self.log);
            println!("$2007 <- {v:02x} after reads: {}", reads.join(" "));
        }
    }
    fn peek(&mut self, a: u16) -> u8 {
        self.look(a)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).unwrap();
    let n: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let rom = nes_console::ines::parse(&bytes).unwrap();
    let mut cpu = MicroCpu::new();
    cpu.set_decimal_adjust(false);
    let quiet = std::env::var_os("QUIET").is_some();
    cpu.bus = Some(Box::new(Flat { ram: vec![0xa5; 0x800], prg_ram: vec![0; 0x2000], prg: rom.prg, log: Vec::new(), quiet }));
    cpu.power_cycle();
    let watch: Vec<u16> = std::env::var("WATCH").ok().map(|w| w.split(',').map(|x| u16::from_str_radix(x, 16).unwrap()).collect()).unwrap_or_default();
    let mut shown = 0;
    for _ in 0..n {
        cpu.half_step();
        let f = cpu.pins();
        if f.sync && f.clk0 && watch.contains(&f.ab) && shown < 2000 {
            let (a, x, y, sp, p, pc) = cpu.registers();
            println!("fetch at {:04x}: A={a:02x} X={x:02x} Y={y:02x} S={sp:02x} P={p:02x} PC={pc:04x}", f.ab);
            shown += 1;
        }
    }
    // Where it stands: the last reads, for a program that never wrote.
    let f = cpu.pins();
    println!("end: {}", v6502_pins::line(&f));
    // blargg's report window through the flat bus.
    let bus = cpu.bus.as_mut().unwrap();
    if bus.peek(0x6001) == 0xde && bus.peek(0x6002) == 0xb0 && bus.peek(0x6003) == 0x61 {
        let text: String = (0x6004..0x6200).map(|a| bus.peek(a)).take_while(|&x| x != 0).map(|x| x as char).collect();
        println!("$6000 result: {:02x}; text: {:?}", bus.peek(0x6000), text.trim());
    }
}
