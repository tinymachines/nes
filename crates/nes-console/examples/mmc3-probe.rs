//! Where MMC3's counter is clocked, in the PPU's own frame: the probe
//! behind the scanline counter's timing.
//!
//!   cargo run --release -p nes-console --example mmc3-probe -- rom.nes [frames]
//!
//! Runs the ROM and, every time the board's filtered A12 rise fires,
//! prints the PPU position it fired at, the counter and latch it left
//! behind, and whether /IRQ went low. FROM=n starts the printing at
//! frame n; LINES=a-b prints only the rises on those PPU lines.
//!
//! What it is for: a game with a status bar splits the screen on this
//! interrupt, so a rise on the wrong dot moves the split. blargg's
//! `4-scanline_timing` measures the same thing from inside the machine,
//! to PPU clock accuracy, and this is how a disagreement with it is
//! located rather than guessed at.

use std::cell::RefCell;
use std::rc::Rc;

use nes_bus::cart::{Cartridge, Mmc3};
use nes_console::{ines, Alignment, Console};

/// The board, shared: the console takes a cartridge by value, so the
/// probe keeps a handle on the same one and delegates every call. It
/// adds nothing and decides nothing.
struct Shared(Rc<RefCell<Mmc3>>);

impl Cartridge for Shared {
    fn cpu_read(&mut self, a: u16) -> Option<u8> {
        self.0.borrow_mut().cpu_read(a)
    }
    fn cpu_write(&mut self, a: u16, v: u8) {
        self.0.borrow_mut().cpu_write(a, v)
    }
    fn chr_read(&mut self, a: u16) -> Option<u8> {
        self.0.borrow_mut().chr_read(a)
    }
    fn chr_write(&mut self, a: u16, v: u8) {
        self.0.borrow_mut().chr_write(a, v)
    }
    fn ciram(&self, ppu_a: u16) -> (bool, bool) {
        self.0.borrow().ciram(ppu_a)
    }
    fn ppu_bus(&mut self, ppu_a: u16, dot: u64) {
        self.0.borrow_mut().ppu_bus(ppu_a, dot)
    }
    fn irq(&self) -> bool {
        self.0.borrow().irq()
    }
    fn owns_chr_ram(&self) -> bool {
        self.0.borrow().owns_chr_ram()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let rom = ines::parse(&bytes).expect("iNES");
    assert_eq!(rom.mapper, 4, "this probe is about MMC3; {} is another board", rom.mapper);
    let chr = if rom.chr_ram { Vec::new() } else { rom.chr.clone() };
    let board = Rc::new(RefCell::new(Mmc3::new(rom.prg.clone(), chr, rom.mirroring).expect("MMC3")));
    let mut c = Console::with_prg_ram(Box::new(Shared(board.clone())), None, Alignment::default(), true);
    let from: usize = std::env::var("FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let lines: Option<(usize, usize)> = std::env::var("LINES").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    let mut seen = 0u64;
    for f in 0..frames {
        // One frame of master half-steps, looked at after each one.
        let target = c.frames.len() + 1;
        while c.frames.len() < target {
            c.master_half_step();
            let pos = c.board.borrow().ppu.position();
            let m = board.borrow();
            let (clocks, dot) = m.clocks();
            if clocks > seen {
                seen = clocks;
                let (counter, latch, irq) = m.irq_state();
                if f >= from && lines.is_none_or(|(a, b)| (a..=b).contains(&pos.line)) {
                    println!("frame {f}: clock {clocks} at line {} dot {} (console dot {dot}): counter {counter}, latch {latch}{}", pos.line, pos.dot, if irq { ", /IRQ low" } else { "" });
                }
            }
        }
    }
}
