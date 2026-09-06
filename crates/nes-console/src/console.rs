//! The scheduler: one master half-step counter, the CPU on every twelfth
//! tick and the PPU on every eighth, the interrupt lines sampled between
//! them, the frames collected as the PPU completes them.

use std::cell::RefCell;
use std::rc::Rc;

use nes_bus::cart::Cartridge;
use nes_bus::DotFrame;
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
        Console { board, cpu, cpu_trace: None, alignment, master: 0, cpu_half_cycles: 0, dots: 0, frames: Vec::new(), sound: None }
    }

    /// One master half-step: the PPU dot and the CPU half-cycle that
    /// fall on it, PPU first (its /INT and the APU's IRQ are what the
    /// CPU samples as its half-cycle begins).
    pub fn master_half_step(&mut self) {
        let m = self.master;
        if m % 8 == self.alignment.ppu_phase as u64 {
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
            };
            self.cpu.set_inputs(true, !irq, !nmi, true, false);
            self.cpu.half_step();
            self.cpu_half_cycles += 1;
            if let Some(s) = self.sound.as_mut() {
                s.push(self.cpu.apu.borrow().codes());
            }
            if let Some(t) = self.cpu_trace.as_mut() {
                t.push(CpuStep { master: m, nmi, irq, frame: self.cpu.pins() });
            }
        }
        self.master += 1;
    }

    pub fn run_master(&mut self, n: u64) {
        for _ in 0..n {
            self.master_half_step();
        }
    }

    /// Run until `n` more frames have completed.
    pub fn run_frames(&mut self, n: usize) {
        let target = self.frames.len() + n;
        while self.frames.len() < target {
            self.master_half_step();
        }
    }

    /// The controllers, as the shell sets them.
    pub fn pad(&self, i: usize) -> nes_glue::controller::Buttons {
        self.board.borrow().pads[i].buttons
    }

    pub fn set_pad(&mut self, i: usize, b: nes_glue::controller::Buttons) {
        self.board.borrow_mut().pads[i].buttons = b;
    }
}
