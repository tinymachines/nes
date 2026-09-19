//! The scheduler: one master half-step counter, the CPU on every twelfth
//! tick and the PPU on every eighth, the interrupt lines sampled between
//! them, the frames collected as the PPU completes them.

use std::cell::RefCell;
use std::rc::Rc;

use nes_bus::cart::Cartridge;
use nes_bus::DotFrame;
use v2c02_fast::Position;
use v2a03_micro::rung::Rung;
use v6502_pins::PinEngine;

use crate::board::{Board, CpuBus};

/// Which of the twelve master half-steps the CPU's half-cycle lands on,
/// and which of the eight the PPU's dot does. MEASURED off the two
/// switch-level chips' own power-on recipes, each stepped on its master
/// clock from the last pulse of its reset: the 2A03's clk0 changes on
/// master half-steps 4, 16, 28, ... (`v2a03-sim/examples/clk-phase.rs`)
/// and the 2C02's pclk0 rises on 3, 11, 19, ... (`v2c02-sim/examples/
/// clk-phase.rs`), so with both started together a CPU half-cycle
/// begins on half-step 4 mod 12 and a dot on 3 mod 8. That is one of the
/// alignments the dividers can power up in; a console records the one
/// it ran in its stamp, and the alignment gate holds it.
///
/// `cpu_phase` is the master half-step of the CPU's first half-cycle,
/// a phi1, and every twelfth after it is the next; it ranges over a
/// whole CPU cycle, 0 to 23, because which half-steps a phi1 falls on
/// (against the dot's eight) is what the race gate sweeps, and the
/// values from 12 up put the phi1 where the values below put a phi2.
/// `ppu_phase` is the master half-step of the first dot, 0 to 7.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Alignment {
    pub cpu_phase: u8,
    pub ppu_phase: u8,
}

impl Alignment {
    pub const MEASURED: Alignment = Alignment { cpu_phase: 4, ppu_phase: 3 };
}

impl Default for Alignment {
    fn default() -> Alignment {
        Alignment::MEASURED
    }
}

/// One CPU half-cycle as the console drove it: the interrupt levels
/// presented before the step and the pin frame after it. What the
/// alignment gate replays on the switch-level 6502.
#[derive(Clone, Copy, Debug)]
pub struct CpuStep {
    /// The master half-step this CPU half-cycle ran on.
    pub master: u64,
    pub nmi: bool,
    pub irq: bool,
    pub frame: v6502_pins::PinFrame,
    /// RDY as the 2A03 feeds its 6502 core, which is what a 6502 sees:
    /// the package has no RDY pin, `frame.rdy` is the rung's account of
    /// the hold at the pins, and on release the die re-runs the held read
    /// with that level already high while the core is fed one cycle
    /// later (2a03's `rung.rs`). A record for the 6502 stack carries
    /// this one (nes-bench's trace plan, T1: the switch-level 6502 on
    /// the record agrees to the last half-cycle with it, and parts at
    /// the first sprite DMA with the pin's).
    pub core_rdy: bool,
}

pub struct Console {
    pub board: Rc<RefCell<Board>>,
    pub cpu: Rung,
    /// When Some, every CPU half-cycle is appended (gate 1's instrument;
    /// None costs nothing).
    pub cpu_trace: Option<Vec<CpuStep>>,
    pub alignment: Alignment,
    /// Master half-steps since power-on.
    pub master: u64,
    pub cpu_half_cycles: u64,
    pub dots: u64,
    /// Completed pictures, oldest first; a shell drains them.
    pub frames: Vec<DotFrame>,
    /// When Some, the APU's codes after every CPU half-cycle go through
    /// the sound (N7); None costs nothing.
    pub sound: Option<crate::sound::Sound>,
    /// The CPU's /RESET as the console drives it: true (released) from
    /// power-on, which the rung's own power-on sequence already covers.
    /// A shell or a runner holds it low for the front panel's button
    /// (`reset_button`). The PPU's /RES is not driven: v2c02-fast has no
    /// reset, so a warm reset here is the CPU's alone, and says so.
    pub res_n: bool,
}

/// Where the vertical sync begins in the PPU's frame, as the switch-level
/// 2C02 emits it (`2c02`'s `vsync-probe`: the sync-tip leg asserted from
/// row 244 dot 280 through row 245 dot 257, three such pulses a row
/// apart) and as the console's record shows it (nes-bench run
/// 20260918-135721: the broad pulse one line after the preceding
/// horizontal sync). In the PPU's own dot count, which is what
/// `Position` carries.
pub const VSYNC_ONSET: Position = Position { line: 244, dot: 280 };

/// Whether a position in the PPU's frame (261 first, then 0..=260) comes
/// after the vertical sync's onset, so the next sync is the following
/// frame's.
pub fn after_vsync_onset(p: Position) -> bool {
    p.line != nes_bus::LINES - 1 && (p.line > VSYNC_ONSET.line || (p.line == VSYNC_ONSET.line && p.dot >= VSYNC_ONSET.dot))
}

/// The index of the picture a capture triggered at a latch that fell at
/// `pos` in frame `fell_in` hands back (`run_to_picture_after_latch`).
pub fn picture_after_latch(fell_in: usize, pos: Position) -> usize {
    fell_in + if after_vsync_onset(pos) { 2 } else { 1 }
}

impl Console {
    pub fn new(cart: Box<dyn Cartridge>, chr_ram: Option<Vec<u8>>, alignment: Alignment) -> Console {
        Console::with_prg_ram(cart, chr_ram, alignment, false)
    }

    /// The same with 8 KiB of cartridge RAM at $6000, the test
    /// cartridges' reporting window (`Board::prg_ram`).
    pub fn with_prg_ram(cart: Box<dyn Cartridge>, chr_ram: Option<Vec<u8>>, alignment: Alignment, prg_ram: bool) -> Console {
        let board = Board::new(cart, chr_ram, prg_ram);
        let cpu = Rung::with_bus(Box::new(CpuBus(board.clone())), v2a03_micro::STACK_AT_H0_MEASURED);
        Console { board, cpu, cpu_trace: None, alignment, master: 0, cpu_half_cycles: 0, dots: 0, frames: Vec::new(), sound: None, res_n: true }
    }

    /// One master half-step: the PPU dot and the CPU half-cycle that
    /// fall on it, PPU first (its /INT and the APU's IRQ are what the
    /// CPU samples as its half-cycle begins).
    pub fn master_half_step(&mut self) {
        let m = self.master;
        if m % 8 == self.alignment.ppu_phase as u64 {
            {
                // The dot the cartridge is about to see its bus on.
                let b = self.board.borrow();
                b.cart.borrow_mut().dot = self.dots;
            }
            let frame = self.board.borrow_mut().ppu.step_dot();
            if let Some(f) = frame {
                self.frames.push(f);
            }
            self.dots += 1;
        }
        if m >= self.alignment.cpu_phase as u64 && (m - self.alignment.cpu_phase as u64).is_multiple_of(12) {
            // Where this half-cycle begins inside the dot last stepped,
            // for the PPU's timed reads.
            let into_dot = ((m + 8 - self.alignment.ppu_phase as u64) % 8) as u8;
            self.board.borrow_mut().half_steps_into_dot = into_dot;
            let nmi = self.board.borrow().ppu.nmi_asserted();
            let irq = {
                let apu = self.cpu.apu.borrow();
                apu.frame_irq || apu.dmc.irq
            } || self.board.borrow().cart.borrow().cart.irq();
            self.cpu.set_inputs(self.res_n, !irq, !nmi, true, false);
            self.cpu.half_step();
            self.cpu_half_cycles += 1;
            if let Some(s) = self.sound.as_mut() {
                s.push(self.cpu.apu.borrow().codes());
            }
            if let Some(t) = self.cpu_trace.as_mut() {
                let core_rdy = v6502_pins::PinEngine::pins(&self.cpu.core).rdy;
                t.push(CpuStep { master: m, nmi, irq, frame: self.cpu.pins(), core_rdy });
            }
        }
        self.master += 1;
    }

    pub fn run_master(&mut self, n: u64) {
        for _ in 0..n {
            self.master_half_step();
        }
    }

    /// Run until latch `t` (the bench's poll index) has fallen, at most
    /// `ceiling` frames: the frame it fell in and where in the PPU's
    /// frame the strobe fell.
    pub fn run_to_latch(&mut self, t: u64, ceiling: usize) -> Result<(usize, Position), String> {
        let mut ran = 0usize;
        while self.board.borrow().pads[0].latches <= t {
            if ran >= ceiling {
                return Err(format!("latch {t} was not reached in {ceiling} frames (the game polled {} times in them)", self.board.borrow().pads[0].latches));
            }
            self.run_frames(1);
            ran += 1;
        }
        let pos = self.board.borrow().latch_positions[t as usize].1;
        Ok((self.frames.len() - 1, pos))
    }

    /// Run to the picture a capture triggered at latch `t` hands back,
    /// and return its index in `frames`. The recovery anchors on the
    /// first vertical sync after the trigger and returns the picture
    /// that follows it; the PPU's frame runs 261 (pre-render) then
    /// 0..=260 and its picture completes at the end of line 260, so a
    /// poll before the sync's onset is answered by the picture after
    /// the frame it fell in, and a poll after the onset (a game that
    /// polls late in the blank, Super Mario Bros. at line 251) by the
    /// one after that. Found by `split-score` on the first scrolling
    /// frame the bench captured (2026-09-18): the earlier rule, always
    /// the next picture, was one frame early there and could not have
    /// been caught on a still picture.
    pub fn run_to_picture_after_latch(&mut self, t: u64, ceiling: usize) -> Result<(usize, Position), String> {
        let (fell_in, pos) = self.run_to_latch(t, ceiling)?;
        let target = picture_after_latch(fell_in, pos);
        self.run_frames(target - fell_in);
        Ok((target, pos))
    }

    /// Run until `n` more frames have completed.
    pub fn run_frames(&mut self, n: usize) {
        let target = self.frames.len() + n;
        while self.frames.len() < target {
            self.master_half_step();
        }
    }

    /// The front panel's reset button, pressed for `hold` master half-steps
    /// and released: the CPU's warm reset (the rung's measured freewheel),
    /// the PPU, the cartridge and every memory left as they were. The
    /// pad's latch count is NOT zeroed; a runner that counts from the
    /// release (as the bench's bridge does) takes the count here.
    pub fn reset_button(&mut self, hold: u64) {
        self.res_n = false;
        self.run_master(hold);
        self.res_n = true;
    }

    /// The controllers, as the shell sets them.
    pub fn pad(&self, i: usize) -> nes_glue::controller::Buttons {
        self.board.borrow().pads[i].buttons
    }

    pub fn set_pad(&mut self, i: usize, b: nes_glue::controller::Buttons) {
        self.board.borrow_mut().pads[i].buttons = b;
    }
}
