//! Where MMC3's counter is clocked, in the PPU's own frame: the probe
//! behind the scanline counter's timing.
//!
//!   cargo run --release -p nes-console --example mmc3-probe -- [rom.nes] [frames]
//!
//! With no ROM it runs its own cartridge: `MODE=08` or `MODE=10` puts
//! the background and the sprites in one pattern table or the other,
//! rendering goes on, the counter is set to reload every clock, and the
//! program spins. That is the shape blargg's `4-scanline_timing`
//! measures in, with nothing else moving, so the spacing of the clocks
//! can be read straight off. `SPACING=1` prints the gap in dots between
//! each clock and the one before it instead of the state, which is how
//! a line that is not 341 dots long shows itself.
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

use nes_bus::cart::{Cartridge, Mirroring, Mmc3};
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

/// The probe's own cartridge: 32 KiB of PRG whose program puts both
/// tables where `mode` says, turns rendering on, sets the counter's
/// latch to 0 (so it reloads and fires on every clock) and spins. The
/// code sits in the last 8 KiB, which MMC3 fixes at $E000 in both of
/// its PRG modes, so it runs whatever the bank registers power up as.
fn own_cartridge(mode: u8) -> Mmc3 {
    let mut prg = vec![0u8; 0x8000];
    let code: &[u8] = &[
        0xa9, 0x00, 0x8d, 0x00, 0xc0, // LDA #$00 ; STA $C000: the latch
        0x8d, 0x01, 0xc0, // STA $C001: reload at the next clock
        0x8d, 0x01, 0xe0, // STA $E001: interrupts enabled
        0xa9, mode, 0x8d, 0x00, 0x20, // LDA #mode ; STA $2000
        0xa9, 0x18, 0x8d, 0x01, 0x20, // LDA #$18 ; STA $2001: both shown
        0x4c, 0x15, 0xe0, // JMP here
    ];
    let base = prg.len() - 0x2000;
    prg[base..base + code.len()].copy_from_slice(code);
    let n = prg.len();
    prg[n - 4..n - 2].copy_from_slice(&[0x00, 0xe0]); // reset vector $E000
    // And an IRQ handler that acknowledges and returns, so a board
    // firing every line does not bury the program in interrupts.
    prg[base + 0x0100..base + 0x0100 + 6].copy_from_slice(&[0x8d, 0x00, 0xe0, 0x8d, 0x01, 0xe0]);
    prg[base + 0x0106] = 0x40; // RTI
    prg[n - 2..n].copy_from_slice(&[0x00, 0xe1]); // IRQ vector $E100
    Mmc3::new(prg, vec![0u8; 0x2000], Mirroring::Vertical).expect("MMC3")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = args.get(1).filter(|a| a.ends_with(".nes"));
    let frames: usize = args.iter().skip(1).find_map(|s| s.parse().ok()).unwrap_or(4);
    let board = Rc::new(RefCell::new(match rom_path {
        Some(path) => {
            let bytes = std::fs::read(path).expect("rom");
            let rom = ines::parse(&bytes).expect("iNES");
            assert_eq!(rom.mapper, 4, "this probe is about MMC3; {} is another board", rom.mapper);
            let chr = if rom.chr_ram { Vec::new() } else { rom.chr.clone() };
            Mmc3::new(rom.prg.clone(), chr, rom.mirroring).expect("MMC3")
        }
        None => {
            let mode = std::env::var("MODE").ok().and_then(|v| u8::from_str_radix(&v, 16).ok()).unwrap_or(0x08);
            println!("the probe's own cartridge: $2000 = {mode:02x}");
            own_cartridge(mode)
        }
    }));
    let mut c = Console::with_prg_ram(Box::new(Shared(board.clone())), None, Alignment::default(), true);
    let from: usize = std::env::var("FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let lines: Option<(usize, usize)> = std::env::var("LINES").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    let mut seen = 0u64;
    let spacing = std::env::var_os("SPACING").is_some();
    let mut last_at: Option<(usize, usize, u64)> = None;
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
                    if spacing {
                        let gap = last_at.map(|(_, _, d)| dot - d).map(|g| g.to_string()).unwrap_or_else(|| "-".into());
                        println!("frame {f}: clock {clocks} at line {:3} dot {:3}: {gap} dots since the last", pos.line, pos.dot);
                    } else {
                        println!("frame {f}: clock {clocks} at line {} dot {} (console dot {dot}): counter {counter}, latch {latch}{}", pos.line, pos.dot, if irq { ", /IRQ low" } else { "" });
                    }
                }
                last_at = Some((pos.line, pos.dot, dot));
            }
        }
    }
}
