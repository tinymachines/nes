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

/// How far behind the board a cartridge's /IRQ reaches the core, in
/// master half-steps: twelve to a CPU half-cycle, eight to a PPU dot.
///
/// AUTHORED, with blargg's `mmc3_test_2/4-scanline_timing` as the
/// oracle and nothing else. That ROM brackets the interrupt's arrival
/// to ONE PPU clock: it runs the same twelve cases twice, a clock apart,
/// and each pair must land on opposite sides of a fixed instruction
/// (`asl irq_flag` in its handler against an `inc` in line, so $21 means
/// the interrupt came first and $22 that it did not). A bracket that
/// tight cannot be met by handing the core the board's own level at
/// whatever CPU half-cycle comes next, which is what this console did
/// until 2026-09-20: a CPU half-cycle is a dot and a half, so stepping
/// by one steps over the answer.
///
/// `examples/irq-sweep` is the measurement: every delay from 0 up, the
/// ROM run at each, and what each one reports. The ROM allows fourteen
/// through twenty-one and no further, eight values wide, and seventeen
/// is the middle of that band. Re-measure it after anything that moves
/// where the line is watched: making the watch exact rather than every
/// master half-step shifted the whole band by one, which is what a
/// zero point moving looks like.
///
/// Two things this number is NOT. It is not a propagation time anybody
/// measured on a part. And at fourteen to twenty-one master half-steps
/// it is more than half a CPU cycle, which is far too long for a wire
/// from pin 15 to the CPU: whatever it is standing in for, most of it
/// is likely where inside its cycle the core samples IRQ, which is
/// rung 3's business and not the cartridge's. Both are the same open
/// item in nes-bench: a scope on pin 15 against the CPU's phi2 turns
/// the middle of a band into a number, and would say which end of the
/// path the slack belongs to.
pub const CART_IRQ_DELAY: u64 = 17;

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
    /// The master half-step the cartridge's /IRQ went low at, or None
    /// while it is open. The core is not given it until
    /// `cart_irq_delay` half-steps have passed.
    pub(crate) cart_irq_low_since: Option<u64>,
    /// How far the cartridge's /IRQ is behind the board that drives it,
    /// in master half-steps (twelve to a CPU half-cycle, eight to a PPU
    /// dot). [`CART_IRQ_DELAY`] is where the number comes from; a probe
    /// sweeps it.
    pub cart_irq_delay: u64,
    /// When Some, every CPU cycle's bus, the registers at each opcode
    /// fetch, the inputs and the pictures are appended (`record`'s trace,
    /// for the roof's flow tools); None costs nothing.
    pub trace: Option<crate::record::Trace>,
    /// When Some, every pad change, reset press and picture is logged
    /// (`record`'s input log: the recording a replay plays back).
    pub inputs: Option<crate::record::InputLog>,
    /// Pictures completed since power-on.
    pub frames_done: u64,
    /// The newest picture's digest and the master half-step after the one
    /// it completed on, kept while a trace or a log is on (a replay holds
    /// the log to them).
    pub last_frame_digest: Option<u32>,
    pub last_frame_master: Option<u64>,
}

/// The PPU's registers and position, as `Console::ppu_status` reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PpuStatus {
    pub line: usize,
    pub dot: usize,
    pub ctrl: u8,
    pub mask: u8,
    pub v: u16,
    pub t: u16,
    pub fine_x: u8,
    pub w: bool,
    pub oamaddr: u8,
    pub vbl: bool,
    /// Where sprite 0 hit this frame, (line, dot), or None so far.
    pub spr0_hit: Option<(usize, usize)>,
    pub spr_overflow: bool,
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
        Console { board, cpu, cpu_trace: None, alignment, master: 0, cpu_half_cycles: 0, dots: 0, frames: Vec::new(), sound: None, res_n: true, cart_irq_low_since: None, cart_irq_delay: CART_IRQ_DELAY, trace: None, inputs: None, frames_done: 0, last_frame_digest: None, last_frame_master: None }
    }

    /// The cartridge RAM at $6000..$7FFF as it stands, or None where the
    /// board fits none. This is `Board::prg_ram`, the console's own 8 KiB
    /// (the one the browser build fits and blargg's cartridges report
    /// through), which answers before a board's own RAM when both exist:
    /// what a game with a battery saves lands here, so this is what a
    /// save file is.
    pub fn battery_ram(&self) -> Option<Vec<u8>> {
        self.board.borrow().prg_ram.clone()
    }

    /// Put a saved cartridge RAM back: what a battery kept across
    /// power-off. Refused with the reason when the board fits no RAM or
    /// the bytes are not the RAM's size, rather than loading half a save.
    pub fn set_battery_ram(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut b = self.board.borrow_mut();
        match b.prg_ram.as_mut() {
            None => Err("this board fits no cartridge RAM".into()),
            Some(ram) if ram.len() != bytes.len() => Err(format!("the cartridge RAM is {} bytes and the save is {}", ram.len(), bytes.len())),
            Some(ram) => {
                ram.copy_from_slice(bytes);
                Ok(())
            }
        }
    }

    // ------------------------------------------------------------------
    // Reads, for a debugger: what the machine holds, without moving it.
    // Every one is a look at state the model already keeps; none steps
    // anything, updates the open bus, or touches a mapper's counters.
    // The CHR is deliberately not here: the only read path through a
    // cartridge is the PPU's, and on a counting board it ticks the A12
    // filter (MMC3) or flips the latch (MMC2), so a look at CHR-ROM is the
    // file's to give, and CHR-RAM the console's (`chr_ram`).
    // ------------------------------------------------------------------

    /// The CPU's registers as the core holds them: A, X, Y, S, P, PC.
    pub fn cpu_registers(&self) -> (u8, u8, u8, u8, u8, u16) {
        self.cpu.core.registers()
    }

    /// The address the core last fetched an opcode from, and the opcode.
    pub fn last_fetch(&self) -> (u16, u8) {
        self.cpu.core.last_fetch()
    }

    /// A byte of the CPU bus with no side effect: the core's own operand
    /// look (`Board::peek`). RAM and the cartridge answer; a register
    /// answers with the open bus, because reading it would move it.
    pub fn peek(&self, a: u16) -> u8 {
        self.board.borrow_mut().peek(a)
    }

    /// Palette RAM, $3F00..$3F1F, as the PPU holds it.
    pub fn ppu_palette(&self) -> [u8; 32] {
        self.board.borrow().ppu.palette
    }

    /// OAM, the 64 sprites' four bytes each, as the PPU holds it.
    pub fn ppu_oam(&self) -> [u8; 256] {
        self.board.borrow().ppu.oam
    }

    /// The PPU's registers and where its beam is.
    pub fn ppu_status(&self) -> PpuStatus {
        let b = self.board.borrow();
        let p = &b.ppu;
        let pos = p.position();
        PpuStatus {
            line: pos.line,
            dot: pos.dot,
            ctrl: p.ctrl,
            mask: p.mask,
            v: p.v,
            t: p.t,
            fine_x: p.fine_x,
            w: p.w,
            oamaddr: p.oamaddr,
            vbl: p.vbl,
            spr0_hit: p.spr0_hit,
            spr_overflow: p.spr_overflow,
        }
    }

    /// The nametable RAM (U4, 2 KiB) as the chip holds it, in its own
    /// address order; how the cartridge maps it is the mirroring.
    pub fn ciram(&self) -> Vec<u8> {
        let b = self.board.borrow();
        let c = b.cart.borrow();
        (0..0x800u16).map(|a| c.ciram.read(a)).collect()
    }

    /// The console's CHR-RAM, on a board that declared no CHR and keeps
    /// it here; None where the cartridge carries its own CHR.
    pub fn chr_ram(&self) -> Option<Vec<u8>> {
        self.board.borrow().cart.borrow().chr_ram.clone()
    }

    /// The pattern memory as the picture chip sees it at this instant, 8
    /// KiB: the console's own CHR-RAM where it keeps one, otherwise the
    /// cartridge's CHR through the board's banks as they stand. None for
    /// a board that cannot save its state, because the read below leans
    /// on putting it back.
    ///
    /// Read from the board directly and not over the PPU bus, so a board
    /// that counts the address line (`ppu_bus`) never sees the sweep.
    /// A board's own read can still have a side effect of its own: the
    /// MMC2 latch moves on a fetch of its trigger tiles, and a sweep that
    /// let it move would read every byte after the trigger from the bank
    /// the trigger chose. So the board's state is saved once and put back
    /// after EVERY read, which reads each byte with the banks exactly as
    /// they stood, and leaves the board as it was. That is one state load
    /// a byte, a clone of the board's registers (and its CHR-RAM, on a
    /// board that keeps one): a few milliseconds, asked for by a window
    /// that is usually closed.
    pub fn chr(&self) -> Option<Vec<u8>> {
        let b = self.board.borrow();
        let mut c = b.cart.borrow_mut();
        if let Some(ram) = &c.chr_ram {
            return Some(ram.clone());
        }
        let saved = c.cart.save_state()?;
        let mut out = Vec::with_capacity(0x2000);
        for a in 0..0x2000u16 {
            out.push(c.cart.chr_read(a).unwrap_or(0));
            if c.cart.load_state(&saved).is_err() {
                return None;
            }
        }
        Some(out)
    }

    /// One master half-step: the PPU dot and the CPU half-cycle that
    /// fall on it, PPU first (its /INT and the APU's IRQ are what the
    /// CPU samples as its half-cycle begins).
    /// The cartridge's /IRQ is a LINE, not a value read when the core
    /// happens to look: the board pulls it low inside a PPU dot and it
    /// reaches the core some way after (`CART_IRQ_DELAY`). Watched at
    /// the only two points that can move it, a PPU bus access and a CPU
    /// write, which is exact and a quarter the cost of looking every
    /// master half-step (about 15% of the console's rate, measured on
    /// Super Mario Bros. 3).
    fn watch_cart_irq(&mut self, m: u64) {
        if self.board.borrow().cart.borrow().cart.irq() {
            self.cart_irq_low_since.get_or_insert(m);
        } else {
            self.cart_irq_low_since = None;
        }
    }

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
                if self.trace.is_some() || self.inputs.is_some() {
                    let d = crate::record::frame_digest(&f);
                    self.last_frame_digest = Some(d);
                    self.last_frame_master = Some(m + 1);
                    if let Some(t) = self.trace.as_mut() {
                        t.frame(d, self.frames_done as u32);
                    }
                    if let Some(l) = self.inputs.as_mut() {
                        l.push(crate::record::FRAME, 0, 0, d, m + 1);
                    }
                }
                self.frames_done += 1;
                self.frames.push(f);
            }
            self.dots += 1;
            self.watch_cart_irq(m);
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
            } || self.cart_irq_low_since.is_some_and(|t| m >= t + self.cart_irq_delay);
            self.cpu.set_inputs(self.res_n, !irq, !nmi, true, false);
            self.cpu.half_step();
            self.cpu_half_cycles += 1;
            // A write to $E000/$E001, or a $2006 that clocked the
            // counter, moves the line inside this half-step.
            self.watch_cart_irq(m);
            if let Some(s) = self.sound.as_mut() {
                s.push(self.cpu.apu.borrow().codes());
            }
            if self.trace.is_some() {
                self.trace_cycle(nmi, irq);
            }
            if let Some(t) = self.cpu_trace.as_mut() {
                let core_rdy = v6502_pins::PinEngine::pins(&self.cpu.core).rdy;
                t.push(CpuStep { master: m, nmi, irq, frame: self.cpu.pins(), core_rdy });
            }
        }
        self.master += 1;
    }

    /// The phi2 half of a CPU cycle into the trace (`record`): the bus,
    /// and after an opcode fetch the registers.
    fn trace_cycle(&mut self, nmi: bool, irq: bool) {
        use crate::record::*;
        let f = self.cpu.pins();
        if !f.clk0 {
            return;
        }
        let (prg, line) = {
            let b = self.board.borrow();
            let prg = if f.rw { b.cart.borrow().cart.prg_offset(f.ab) } else { None };
            (prg, b.ppu.position().line as u16)
        };
        let flags = if f.rw { F_READ } else { 0 } | if f.sync { F_SYNC } else { 0 } | if nmi { F_NMI } else { 0 } | if irq { F_IRQ } else { 0 } | if f.rdy { 0 } else { F_HELD };
        let t = self.trace.as_mut().unwrap();
        t.cycle(f.ab, f.db, flags, prg);
        if f.sync && f.rdy {
            let (a, x, y, s, p, _) = self.cpu.core.registers();
            t.regs((a, x, y, s, p), line);
        }
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

    /// Breakpoints: run until `n` more pictures complete, or until the CPU
    /// begins fetching an opcode at one of `at` (SYNC rising with that
    /// address on the bus), and say which. The console stops inside that
    /// fetch, the instruction about to run. A fetch already under way when
    /// this is called does not count, so running on from a stop goes past
    /// the instruction it stopped at. With no addresses this is
    /// `run_frames`: the check costs nothing when nothing is set.
    pub fn run_frames_until(&mut self, n: usize, at: &[u16]) -> Option<u16> {
        if at.is_empty() {
            self.run_frames(n);
            return None;
        }
        let target = self.frames.len() + n;
        let mut was = self.cpu.pins().sync;
        // MUTATE_BREAK=1 lets the fetch under way count, so a run from a
        // stop never leaves it, and tests/breakpoints.rs must go red.
        let mutate = std::env::var_os("MUTATE_BREAK").is_some();
        while self.frames.len() < target {
            self.master_half_step();
            let p = self.cpu.pins();
            if p.sync && (mutate || !was) && at.contains(&p.ab) {
                return Some(p.ab);
            }
            was = p.sync;
        }
        None
    }

    // ------------------------------------------------------------------
    // Steps, for a debugger: the machine moved by one of its own units.
    // Every one is `master_half_step` some number of times, so what a
    // step does is exactly what running does, only stopped sooner. A
    // frame that completes inside a step lands in `frames` as always.
    // ------------------------------------------------------------------

    /// Master half-steps until the CPU has run `n` more half-cycles.
    pub fn step_cpu_half_cycles(&mut self, n: u64) -> u64 {
        let target = self.cpu_half_cycles + n;
        let mut took = 0;
        while self.cpu_half_cycles < target {
            self.master_half_step();
            took += 1;
        }
        took
    }

    /// Master half-steps until the next instruction begins: the CPU's
    /// SYNC pin rising, which is the opcode fetch. A ceiling in master
    /// half-steps bounds a core that never fetches (held in reset, or in
    /// DMA); the return is the half-steps taken either way.
    pub fn step_instruction(&mut self, ceiling: u64) -> u64 {
        let mut was = self.cpu.pins().sync;
        let mut took = 0;
        while took < ceiling {
            self.master_half_step();
            took += 1;
            let now = self.cpu.pins().sync;
            if now && !was {
                break;
            }
            was = now;
        }
        took
    }

    /// Master half-steps until the PPU is on another scanline.
    pub fn step_scanline(&mut self) -> u64 {
        let line = self.board.borrow().ppu.position().line;
        let mut took = 0;
        // A line is 341 dots of eight master half-steps; twice that is a ceiling.
        while took < 2 * 341 * 8 {
            self.master_half_step();
            took += 1;
            if self.board.borrow().ppu.position().line != line {
                break;
            }
        }
        took
    }

    /// The front panel's reset button, pressed for `hold` master half-steps
    /// and released: the CPU's warm reset (the rung's measured freewheel),
    /// the PPU, the cartridge and every memory left as they were. The
    /// pad's latch count is NOT zeroed; a runner that counts from the
    /// release (as the bench's bridge does) takes the count here.
    pub fn reset_button(&mut self, hold: u64) {
        if let Some(l) = self.inputs.as_mut() {
            l.push(crate::record::RESET, 0, 0, hold as u32, self.master);
        }
        if let Some(t) = self.trace.as_mut() {
            t.input(2, 1, self.frames_done as u32);
        }
        self.res_n = false;
        self.run_master(hold);
        self.res_n = true;
    }

    /// The controllers, as the shell sets them.
    pub fn pad(&self, i: usize) -> nes_glue::controller::Buttons {
        self.board.borrow().pads[i].buttons
    }

    pub fn set_pad(&mut self, i: usize, b: nes_glue::controller::Buttons) {
        let was = self.board.borrow().pads[i].buttons;
        if let Some(l) = self.inputs.as_mut() {
            l.pad(i, b.as_byte(), self.master);
        }
        if was != b {
            if let Some(t) = self.trace.as_mut() {
                t.input(i as u8, b.as_byte(), self.frames_done as u32);
            }
        }
        self.board.borrow_mut().pads[i].buttons = b;
    }
}
