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
use v2c02_fast::{Fast, VramBus};
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
}

/// The PPU's view of the cartridge and CIRAM.
pub struct PpuBus(pub Rc<RefCell<Cart>>);

impl VramBus for PpuBus {
    fn read(&mut self, a: u16) -> u8 {
        let mut c = self.0.borrow_mut();
        let a = a & 0x3fff;
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
    pub cart: Rc<RefCell<Cart>>,
    pub ppu: Fast,
    pub pads: [Controller; 2],
    pub open_bus: u8,
    /// CPU reads and writes seen, for the stamp and the tests.
    pub reads: u64,
    pub writes: u64,
}

impl Board {
    pub fn new(cart: Box<dyn Cartridge>, chr_ram: Option<Vec<u8>>) -> Rc<RefCell<Board>> {
        let cart = Rc::new(RefCell::new(Cart { cart, ciram: Tmm2115::new(), chr_ram }));
        let ppu = Fast::on_bus(Box::new(PpuBus(cart.clone())));
        Rc::new(RefCell::new(Board { wram: Tmm2115::new(), cart, ppu, pads: [Controller::default(); 2], open_bus: 0, reads: 0, writes: 0 }))
    }

    fn read(&mut self, a: u16) -> u8 {
        self.reads += 1;
        let d = cpu_decode(a, true);
        let v = if !d.ram_cs_n {
            self.wram.read(a)
        } else if !d.ppu_cs_n {
            self.ppu.read((a & 7) as u8)
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
        } else {
            self.cart.borrow_mut().cart.cpu_read(a).unwrap_or(self.open_bus)
        };
        self.open_bus = v;
        v
    }

    fn write(&mut self, a: u16, v: u8) {
        self.writes += 1;
        self.open_bus = v;
        let d = cpu_decode(a, true);
        if !d.ram_cs_n {
            self.wram.write(a, v);
        } else if !d.ppu_cs_n {
            self.ppu.write((a & 7) as u8, v);
        } else if a == 0x4016 {
            // OUT0 is the controller strobe.
            for p in &mut self.pads {
                p.strobe(v & 1 != 0);
            }
        } else if a < 0x4020 {
            // The 2A03's registers: the Rung takes them from its own frames.
        } else {
            self.cart.borrow_mut().cart.cpu_write(a, v);
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
}
