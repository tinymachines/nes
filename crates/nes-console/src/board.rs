//! The mainboard: what the CPU's bus and the PPU's bus reach, wired as
//! the glue says. `Board` is shared between the two chips (each holds
//! it behind a `Rc<RefCell>`), which is how one object can be the CPU's
//! `MicroBus` and the PPU's `VramBus` at once without either chip
//! knowing the other exists.
//!
//! Open bus is a value here: the last byte that crossed the CPU data
//! bus. A read nothing drives returns it; the controller buffers merge
//! their bits over it (nes-glue's rule).

use std::cell::RefCell;
use std::rc::Rc;

use nes_bus::cart::Cartridge;
use nes_glue::controller::{read_4016, read_4017, Controller};
use nes_glue::decode::cpu_decode;
use nes_glue::sram::Tmm2115;
use v2c02_fast::{Fast, Position, VramBus};
use v6502_micro::machine::MicroBus;

/// The cartridge and CIRAM, reached from both buses (PRG from the CPU's,
/// CHR and nametables from the PPU's), so they sit in their own cell.
pub struct Cart {
    pub cart: Box<dyn Cartridge>,
    /// U4, addressed by PPU A0..A9 and the cartridge's CIRAM A10.
    pub ciram: Tmm2115,
    /// CHR RAM boards: the cartridge's own CHR is ROM to nes-bus, so a
    /// board that declared none keeps its 8 KiB here and the PPU's
    /// writes land.
    pub chr_ram: Option<Vec<u8>>,
    /// The console's PPU dot counter as of the dot being stepped, set by
    /// `Console::master_half_step`. It is handed to the cartridge with
    /// every PPU bus access, because a counting board (MMC3) measures
    /// how long A12 has been low and nothing here else owns that clock.
    pub dot: u64,
}

/// The PPU's view of the cartridge and CIRAM.
pub struct PpuBus(pub Rc<RefCell<Cart>>);

impl VramBus for PpuBus {
    fn read(&mut self, a: u16) -> u8 {
        let mut c = self.0.borrow_mut();
        let a = a & 0x3fff;
        // Every access, whatever answers it: what a counting board
        // watches is the address line, and the nametable fetches are
        // half of the pattern it counts.
        let dot = c.dot;
        c.cart.ppu_bus(a, dot);
        if a < 0x2000 {
            if let Some(ram) = &c.chr_ram {
                return ram[a as usize];
            }
            return c.cart.chr_read(a).unwrap_or(0);
        }
        // $2000..$3EFF: CIRAM through the cartridge's two pins.
        let (a10, ce_n) = c.cart.ciram(a);
        if ce_n {
            return 0;
        }
        c.ciram.read((a & 0x03ff) | ((a10 as u16) << 10))
    }
    fn write(&mut self, a: u16, v: u8) {
        let mut c = self.0.borrow_mut();
        let a = a & 0x3fff;
        let dot = c.dot;
        c.cart.ppu_bus(a, dot);
        if a < 0x2000 {
            if let Some(ram) = c.chr_ram.as_mut() {
                ram[a as usize] = v;
            } else {
                c.cart.chr_write(a, v);
            }
            return;
        }
        let (a10, ce_n) = c.cart.ciram(a);
        if !ce_n {
            c.ciram.write((a & 0x03ff) | ((a10 as u16) << 10), v);
        }
    }
}

pub struct Board {
    /// U1, behind /RAM CS: 2 KiB mirrored through $0000..$1FFF.
    pub wram: Tmm2115,
    /// 8 KiB at $6000..$7FFF when fitted: not the NES-001's (it has
    /// none) but a cartridge's, which nes-bus's NROM does not model;
    /// blargg's test cartridges report through it, so the console offers
    /// it as the board a test cartridge brings, labelled.
    pub prg_ram: Option<Vec<u8>>,
    pub cart: Rc<RefCell<Cart>>,
    pub ppu: Fast,
    pub pads: [Controller; 2],
    pub open_bus: u8,
    /// CPU reads and writes seen, for the stamp and the tests.
    pub reads: u64,
    pub writes: u64,
    /// Print every PPU register write with the PPU's state, for probes.
    pub trace: bool,
    /// Set by the console before each CPU half-cycle: PPU half-steps from
    /// the start of the dot last stepped to this half-cycle's start, for
    /// the PPU's timed \$2002 read.
    pub half_steps_into_dot: u8,
    /// OUT0 as last written, so the strobe's edges are seen here.
    out0: bool,
    strobe_rose_at: Option<Position>,
    /// Where in the PPU's frame every latch fell: (the strobe's rise,
    /// its fall), indexed by latch (the bench's poll index). The frame a
    /// triggered capture hands back depends on where the poll sits
    /// against the vertical sync (`Console::picture_after_latch`), and
    /// the bench's `poll-line.py` measures the rise on the part.
    pub latch_positions: Vec<(Position, Position)>,
}

impl Board {
    /// The PPU drives its address bus from `v`, so a $2006 write and a
    /// $2007 access move the bus without any fetch happening: with
    /// rendering off that is the only way A12 moves, and it is how a
    /// game clocks a counting cartridge by hand. blargg's
    /// `1-clocking` #3 ("should decrement when A12 is toggled via
    /// PPUADDR") is the ROM that says so. During rendering the next
    /// fetch overrides this within a dot.
    fn ppu_address_moved(&mut self) {
        let a = self.ppu.v & 0x3fff;
        let mut c = self.cart.borrow_mut();
        let dot = c.dot;
        c.cart.ppu_bus(a, dot);
    }

    pub fn new(cart: Box<dyn Cartridge>, chr_ram: Option<Vec<u8>>, prg_ram: bool) -> Rc<RefCell<Board>> {
        // A board that banks its own CHR RAM keeps it; the console's
        // copy exists for the CHR ROM boards a test cartridge is built
        // on, and two copies would be one fact in two places.
        let chr_ram = if cart.owns_chr_ram() { None } else { chr_ram };
        let cart = Rc::new(RefCell::new(Cart { cart, ciram: Tmm2115::new(), chr_ram, dot: 0 }));
        let ppu = Fast::on_bus(Box::new(PpuBus(cart.clone())));
        let trace = std::env::var_os("TRACE_PPU").is_some();
        Rc::new(RefCell::new(Board { wram: Tmm2115::new(), prg_ram: prg_ram.then(|| vec![0u8; 0x2000]), cart, ppu, pads: [Controller::default(), Controller::default()], open_bus: 0, reads: 0, writes: 0, trace, half_steps_into_dot: 0, out0: false, strobe_rose_at: None, latch_positions: Vec::new() }))
    }

    fn read(&mut self, a: u16) -> u8 {
        self.reads += 1;
        if self.trace && std::env::var_os("TRACE_BUS").is_some() {
            eprintln!("bus read ${a:04x}");
        }
        let d = cpu_decode(a, true);
        let v = if !d.ram_cs_n {
            self.wram.read(a)
        } else if !d.ppu_cs_n {
            let v = self.ppu.read_timed((a & 7) as u8, self.half_steps_into_dot);
            self.ppu_address_moved();
            v
        } else if a == 0x4016 {
            let line = self.pads[0].read();
            read_4016(line, true, true, self.open_bus)
        } else if a == 0x4017 {
            let line = self.pads[1].read();
            read_4017(line, [true; 4], self.open_bus)
        } else if a < 0x4020 {
            // The 2A03's own registers: write-only here, the Rung answers
            // $4015 before the bus sees it.
            self.open_bus
        } else if (0x6000..0x8000).contains(&a) && self.prg_ram.is_some() {
            self.prg_ram.as_ref().unwrap()[(a - 0x6000) as usize]
        } else {
            self.cart.borrow_mut().cart.cpu_read(a).unwrap_or(self.open_bus)
        };
        self.open_bus = v;
        v
    }

    /// The core's look at an operand byte or a zero-page pointer: RAM and
    /// the cartridge only, no side effect, no open-bus update; a register
    /// answers with the open bus rather than being read. Public because it
    /// is also what a debugger's look at the bus should be: the model's
    /// own definition of a read that changes nothing (`Console::peek`).
    pub fn peek(&mut self, a: u16) -> u8 {
        let d = cpu_decode(a, true);
        if !d.ram_cs_n {
            self.wram.read(a)
        } else if (0x6000..0x8000).contains(&a) && self.prg_ram.is_some() {
            self.prg_ram.as_ref().unwrap()[(a - 0x6000) as usize]
        } else if a >= 0x4020 {
            self.cart.borrow_mut().cart.cpu_read(a).unwrap_or(self.open_bus)
        } else {
            self.open_bus
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        self.writes += 1;
        self.open_bus = v;
        let d = cpu_decode(a, true);
        if !d.ram_cs_n {
            self.wram.write(a, v);
        } else if !d.ppu_cs_n {
            if self.trace {
                eprintln!("ppu write ${:04x} <- {v:02x}  (v={:04x} t={:04x} w={} ctrl={:02x} mask={:02x} pos={:?})", a, self.ppu.v, self.ppu.t, self.ppu.w as u8, self.ppu.ctrl, self.ppu.mask, self.ppu.position());
            }
            self.ppu.write((a & 7) as u8, v);
            self.ppu_address_moved();
        } else if a == 0x4016 {
            // OUT0 is the controller strobe.
            let out0 = v & 1 != 0;
            for p in &mut self.pads {
                p.strobe(out0);
            }
            let pos = self.ppu.position();
            if out0 && !self.out0 {
                self.strobe_rose_at = Some(pos);
            }
            if !out0 && self.out0 {
                self.latch_positions.push((self.strobe_rose_at.take().unwrap_or(pos), pos));
            }
            self.out0 = out0;
        } else if a < 0x4020 {
            // The 2A03's registers: the Rung takes them from its own frames.
        } else if let (true, Some(ram)) = ((0x6000..0x8000).contains(&a), self.prg_ram.as_mut()) {
            ram[(a - 0x6000) as usize] = v;
        } else {
            // With the dot: MMC1's serial port has to tell an RMW's two
            // writes from two separate ones, and the dot is the only
            // clock the edge carries.
            let mut c = self.cart.borrow_mut();
            let dot = c.dot;
            c.cart.cpu_write_at(a, v, dot);
        }
    }
}

/// The CPU's view of the board.
pub struct CpuBus(pub Rc<RefCell<Board>>);

impl MicroBus for CpuBus {
    fn read(&mut self, a: u16) -> u8 {
        self.0.borrow_mut().read(a)
    }
    fn write(&mut self, a: u16, v: u8) {
        self.0.borrow_mut().write(a, v)
    }
    fn peek(&mut self, a: u16) -> u8 {
        self.0.borrow_mut().peek(a)
    }
}
