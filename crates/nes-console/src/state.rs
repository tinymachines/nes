//! Saved states: the whole console stopped where a CPU cycle ends, as one
//! byte string, and started again there. What the roof's recorder takes
//! when a recording starts in the middle of a game, and what a replay
//! starts from.
//!
//! Each chip says what its own state is (the 2A03's `RungState`, the
//! 2C02's stepper, each cartridge board's `CartState`, the glue's parts),
//! and this adds the console's own: the board around the chips, the
//! cartridge's CIRAM and CHR RAM, the sound stage's filters and queues,
//! and the master clock's counters. Every part here is written out by
//! destructuring its struct with no `..`, so a field added to the Board,
//! the Cart, the Sound or the Console does not compile until it is either
//! saved or named as not state.
//!
//! Not saved, because the console is built the same way before a load:
//! the cartridge's ROM (the board's state never holds it), the chips'
//! measured tables, the sound stage's kernel and mixer tables, and the
//! instruments (the CPU trace, the record trace and input log, the PPU
//! write printer, the cartridge IRQ delay a probe sweeps). A state loads
//! into a console just powered on from the same ROM with the same
//! alignment; one it refuses leaves that console half-loaded, and the
//! caller drops it.

use serde::{Deserialize, Serialize};

use nes_bus::cart::CartState;
use nes_bus::DotFrame;
use nes_glue::controller::Controller;
use nes_glue::sram::Tmm2115;
use v2a03_micro::state::RungState;
use v2c02_fast::{Fast, Position};

use crate::board::{Board, Cart};
use crate::console::{Alignment, Console};
use crate::sound::Sound;

/// The byte form's first eight bytes, then a little-endian u32 version.
const MAGIC: &[u8; 8] = b"TMNESSTA";
/// Bumped whenever the layout below or any chip's state changes shape. A
/// state of another version is refused by name rather than read as
/// something it is not.
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct ConsoleState {
    cpu: RungState,
    /// The 2C02's stepper, as its own bytes (it is its own state; see
    /// v2c02-fast).
    ppu: Vec<u8>,
    board: BoardState,
    cart: CartPart,
    alignment: (u8, u8),
    master: u64,
    cpu_half_cycles: u64,
    dots: u64,
    frames: Vec<DotFrame>,
    sound: Option<SoundState>,
    res_n: bool,
    cart_irq_low_since: Option<u64>,
    frames_done: u64,
    last_frame_digest: Option<u32>,
    last_frame_master: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct BoardState {
    wram: Tmm2115,
    prg_ram: Option<Vec<u8>>,
    pads: [Controller; 2],
    open_bus: u8,
    reads: u64,
    writes: u64,
    half_steps_into_dot: u8,
    out0: bool,
    strobe_rose_at: Option<Position>,
    latch_positions: Vec<(Position, Position)>,
}

#[derive(Serialize, Deserialize)]
struct CartPart {
    board: CartState,
    ciram: Tmm2115,
    chr_ram: Option<Vec<u8>>,
    dot: u64,
}

#[derive(Serialize, Deserialize)]
struct SoundState {
    hp_x1: f64,
    hp_y1: f64,
    lp_y1: f64,
    pending: Vec<f32>,
    pending0: u64,
    samples: u64,
    next_out: u64,
    next_needed: u64,
    out: Vec<f32>,
}

impl Sound {
    fn state(&self) -> SoundState {
        // Every field, so one added is a decision; the tables are rebuilt
        // by `Sound::new`, the mixer with them.
        let Sound { mixer: _, ad1_lut: _, ad2_lut: _, hp_a: _, lp_b: _, hp_x1, hp_y1, lp_y1, pending, pending0, samples, next_out, next_needed, gain: _, kernel: _, half_width: _, out } = self;
        SoundState { hp_x1: *hp_x1, hp_y1: *hp_y1, lp_y1: *lp_y1, pending: pending.clone(), pending0: *pending0, samples: *samples, next_out: *next_out, next_needed: *next_needed, out: out.clone() }
    }

    fn load(&mut self, s: SoundState) {
        let SoundState { hp_x1, hp_y1, lp_y1, pending, pending0, samples, next_out, next_needed, out } = s;
        self.hp_x1 = hp_x1;
        self.hp_y1 = hp_y1;
        self.lp_y1 = lp_y1;
        self.pending = pending;
        self.pending0 = pending0;
        self.samples = samples;
        self.next_out = next_out;
        self.next_needed = next_needed;
        self.out = out;
    }
}

impl Board {
    fn state(&self) -> BoardState {
        // The cartridge and the PPU are saved on their own; `trace` is the
        // probe printer.
        let Board { wram, prg_ram, cart: _, ppu: _, pads, open_bus, reads, writes, trace: _, half_steps_into_dot, out0, strobe_rose_at, latch_positions } = self;
        BoardState {
            wram: wram.clone(),
            prg_ram: prg_ram.clone(),
            pads: pads.clone(),
            open_bus: *open_bus,
            reads: *reads,
            writes: *writes,
            half_steps_into_dot: *half_steps_into_dot,
            out0: *out0,
            strobe_rose_at: *strobe_rose_at,
            latch_positions: latch_positions.clone(),
        }
    }

    fn load(&mut self, s: BoardState) -> Result<(), String> {
        let BoardState { wram, prg_ram, pads, open_bus, reads, writes, half_steps_into_dot, out0, strobe_rose_at, latch_positions } = s;
        if prg_ram.as_ref().map(Vec::len) != self.prg_ram.as_ref().map(Vec::len) {
            return Err("the state's cartridge RAM is not the shape this console's is".into());
        }
        self.wram = wram;
        self.prg_ram = prg_ram;
        self.pads = pads;
        self.open_bus = open_bus;
        self.reads = reads;
        self.writes = writes;
        self.half_steps_into_dot = half_steps_into_dot;
        self.out0 = out0;
        self.strobe_rose_at = strobe_rose_at;
        self.latch_positions = latch_positions;
        Ok(())
    }
}

impl Cart {
    fn state(&self) -> Result<CartPart, String> {
        let Cart { cart, ciram, chr_ram, dot } = self;
        Ok(CartPart { board: cart.save_state().ok_or("this cartridge's board cannot save its state")?, ciram: ciram.clone(), chr_ram: chr_ram.clone(), dot: *dot })
    }

    fn load(&mut self, s: CartPart) -> Result<(), String> {
        let CartPart { board, ciram, chr_ram, dot } = s;
        if chr_ram.as_ref().map(Vec::len) != self.chr_ram.as_ref().map(Vec::len) {
            return Err("the state's CHR RAM is not the shape this console's is".into());
        }
        self.cart.load_state(&board)?;
        self.ciram = ciram;
        self.chr_ram = chr_ram;
        self.dot = dot;
        Ok(())
    }
}

impl Console {
    /// Whether a state can be taken here: where a CPU cycle ends (the
    /// 2A03's `at_cycle_end`). `run_to_cycle_end` gets there.
    pub fn at_cycle_end(&self) -> bool {
        self.cpu.at_cycle_end()
    }

    /// Step master half-steps until a CPU cycle ends: at most a cycle's
    /// twenty-four. Returns how many it took.
    pub fn run_to_cycle_end(&mut self) -> u64 {
        let mut n = 0;
        while !self.at_cycle_end() {
            self.master_half_step();
            n += 1;
        }
        n
    }

    /// The whole console as bytes; refused inside a CPU cycle.
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let Console {
            board,
            cpu,
            cpu_trace: _,
            alignment,
            master,
            cpu_half_cycles,
            dots,
            frames,
            sound,
            res_n,
            cart_irq_low_since,
            cart_irq_delay: _,
            trace: _,
            inputs: _,
            frames_done,
            last_frame_digest,
            last_frame_master,
        } = self;
        let b = board.borrow();
        let st = ConsoleState {
            cpu: cpu.save_state()?,
            ppu: postcard::to_allocvec(&b.ppu).map_err(|e| format!("the PPU's state: {e}"))?,
            board: b.state(),
            cart: b.cart.borrow().state()?,
            alignment: (alignment.cpu_phase, alignment.ppu_phase),
            master: *master,
            cpu_half_cycles: *cpu_half_cycles,
            dots: *dots,
            frames: frames.clone(),
            sound: sound.as_ref().map(Sound::state),
            res_n: *res_n,
            cart_irq_low_since: *cart_irq_low_since,
            frames_done: *frames_done,
            last_frame_digest: *last_frame_digest,
            last_frame_master: *last_frame_master,
        };
        let mut out = Vec::with_capacity(1 << 16);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        postcard::to_extend(&st, out).map_err(|e| format!("the console's state: {e}"))
    }

    /// The console as `bytes` left it. For a console just powered on from
    /// the same ROM; see the module's note on a refusal. The sound stage
    /// is loaded where both have one and left alone otherwise (a replay
    /// runs without sound, and sound never reaches a picture).
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() < 12 || &bytes[..8] != MAGIC {
            return Err("these bytes are not a saved console".into());
        }
        let v = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if v != VERSION {
            return Err(format!("a version {v} state, and this console reads version {VERSION}"));
        }
        let st: ConsoleState = postcard::from_bytes(&bytes[12..]).map_err(|e| format!("the saved console does not read: {e}"))?;
        let ConsoleState { cpu, ppu, board, cart, alignment, master, cpu_half_cycles, dots, frames, sound, res_n, cart_irq_low_since, frames_done, last_frame_digest, last_frame_master } = st;
        if alignment != (self.alignment.cpu_phase, self.alignment.ppu_phase) {
            return Err(format!("a state saved at alignment {alignment:?} into a console at {:?}", (self.alignment.cpu_phase, self.alignment.ppu_phase)));
        }
        let ppu: Fast = postcard::from_bytes(&ppu).map_err(|e| format!("the PPU's state does not read: {e}"))?;
        {
            let mut b = self.board.borrow_mut();
            b.cart.borrow_mut().load(cart)?;
            b.load(board)?;
            b.ppu.load_state(ppu)?;
        }
        self.cpu.load_state(&cpu)?;
        self.alignment = Alignment { cpu_phase: alignment.0, ppu_phase: alignment.1 };
        self.master = master;
        self.cpu_half_cycles = cpu_half_cycles;
        self.dots = dots;
        self.frames = frames;
        if let (Some(s), Some(here)) = (sound, self.sound.as_mut()) {
            here.load(s);
        }
        self.res_n = res_n;
        self.cart_irq_low_since = cart_irq_low_since;
        self.frames_done = frames_done;
        self.last_frame_digest = last_frame_digest;
        self.last_frame_master = last_frame_master;
        Ok(())
    }
}
