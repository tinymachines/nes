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
/// and which of the eight the PPU's dot does. AUTHORED for now: the
/// switch-level dividers' power-on phases (the 2A03's ÷12 and the
/// 2C02's ÷4 relative to one master clock started together) are the
/// measurement N5's alignment gate is written from, and until it lands
/// the console runs the default and stamps it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Alignment {
    pub cpu_phase: u8,
    pub ppu_phase: u8,
}

pub struct Console {
    pub board: Rc<RefCell<Board>>,
    pub cpu: Rung,
    pub alignment: Alignment,
    /// Master half-steps since power-on.
    pub master: u64,
    pub cpu_half_cycles: u64,
    pub dots: u64,
    /// Completed pictures, oldest first; a shell drains them.
    pub frames: Vec<DotFrame>,
}

impl Console {
    pub fn new(cart: Box<dyn Cartridge>, chr_ram: Option<Vec<u8>>, alignment: Alignment) -> Console {
        let board = Board::new(cart, chr_ram);
        let cpu = Rung::with_bus(Box::new(CpuBus(board.clone())), v2a03_micro::STACK_AT_H0_MEASURED);
        Console { board, cpu, alignment, master: 0, cpu_half_cycles: 0, dots: 0, frames: Vec::new() }
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
        if m % 12 == self.alignment.cpu_phase as u64 {
            let nmi = self.board.borrow().ppu.nmi_asserted();
            let irq = {
                let apu = self.cpu.apu.borrow();
                apu.frame_irq || apu.dmc.irq
            };
            self.cpu.set_inputs(true, !irq, !nmi, true, false);
            self.cpu.half_step();
            self.cpu_half_cycles += 1;
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
