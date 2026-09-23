//! The console behind a wasm-bindgen surface (N8 step 3): a ROM in,
//! frames (the 341 x 262 colour and emphasis planes the roof's NES
//! pipeline eats) and 48 kHz sound out, the pad in. Plain Rust with a
//! thin shell gated on the target, ntsc-wasm's shape, so the native
//! bench and the browser run the same code.

#![forbid(unsafe_code)]

use nes_console::{ines, Alignment, Console, Sound};
use nes_glue::controller::Buttons;

pub struct Machine {
    console: Console,
    battery: bool,
}

impl Machine {
    pub fn new(rom: &[u8]) -> Result<Machine, String> {
        let r = ines::parse(rom).map_err(|e| format!("{e:?}"))?;
        let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
        let cart = r.cart().map_err(|e| format!("{e:?}"))?;
        let mut console = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
        console.sound = Some(Sound::default());
        Ok(Machine { console, battery: r.battery })
    }

    /// Whether the header says the cartridge has a battery behind its
    /// RAM: the one case where `battery_ram` is a save worth keeping.
    pub fn has_battery(&self) -> bool {
        self.battery
    }

    /// The cartridge RAM as it stands (`Console::battery_ram`); empty
    /// where the board fits none.
    pub fn battery_ram(&self) -> Vec<u8> {
        self.console.battery_ram().unwrap_or_default()
    }

    /// A saved cartridge RAM back in, before the game runs.
    pub fn set_battery_ram(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.console.set_battery_ram(bytes)
    }

    /// Run `n` frames; the newest is what `colour`, `emphasis` and
    /// `parity` return afterwards.
    pub fn run_frames(&mut self, n: usize) {
        self.console.frames.clear();
        self.console.run_frames(n);
        if self.console.frames.len() > 1 {
            let last = self.console.frames.pop().unwrap();
            self.console.frames.clear();
            self.console.frames.push(last);
        }
    }

    pub fn colour(&self) -> Vec<u8> {
        self.console.frames.last().map(|f| f.colour.clone()).unwrap_or_default()
    }

    pub fn emphasis(&self) -> Vec<u8> {
        self.console.frames.last().map(|f| f.emphasis.clone()).unwrap_or_default()
    }

    /// 0 Even, 1 OddFull, 2 OddShort: what ntsc-wasm's push_frame takes.
    pub fn parity(&self) -> u8 {
        match self.console.frames.last().map(|f| f.parity) {
            Some(nes_bus::FrameParity::OddFull) => 1,
            Some(nes_bus::FrameParity::OddShort) => 2,
            _ => 0,
        }
    }

    /// Drain the 48 kHz sound produced so far, in the table's units
    /// times the stage gain (the shell's listening level is 0.25 of
    /// full scale per unit).
    pub fn sound(&mut self) -> Vec<f32> {
        self.console.sound.as_mut().map(|s| s.out.drain(..).collect()).unwrap_or_default()
    }

    /// The pad as a byte in the register's order: A, B, Select, Start,
    /// Up, Down, Left, Right from bit 0.
    pub fn set_pad(&mut self, bits: u8) {
        let b = Buttons {
            a: bits & 1 != 0,
            b: bits & 2 != 0,
            select: bits & 4 != 0,
            start: bits & 8 != 0,
            up: bits & 16 != 0,
            down: bits & 32 != 0,
            left: bits & 64 != 0,
            right: bits & 128 != 0,
        };
        self.console.set_pad(0, b);
    }

    pub fn cpu_half_cycles(&self) -> u64 {
        self.console.cpu_half_cycles
    }

    // ------------------------------------------------------------------
    // Control, for the roof's workbench: the machine moved by its own
    // units, and the front panel's two buttons. Every step keeps the
    // newest completed frame where `colour` finds it, as run_frames does.
    // ------------------------------------------------------------------

    fn keep_newest_frame(&mut self) {
        if self.console.frames.len() > 1 {
            let last = self.console.frames.pop().unwrap();
            self.console.frames.clear();
            self.console.frames.push(last);
        }
    }

    /// The front panel's reset button, held for a frame's worth of master
    /// half-steps and released: the CPU restarts at its vector; the PPU,
    /// the cartridge and every memory keep what they hold.
    pub fn reset(&mut self) {
        self.console.reset_button(89_342 * 8);
        self.keep_newest_frame();
    }

    /// `n` CPU half-cycles. Returns the master half-steps taken.
    pub fn step_half_cycles(&mut self, n: u32) -> u32 {
        let took = self.console.step_cpu_half_cycles(n as u64) as u32;
        self.keep_newest_frame();
        took
    }

    /// To the next instruction's opcode fetch. Returns the master
    /// half-steps taken; a core that never fetches is stopped at the ceiling.
    pub fn step_instruction(&mut self) -> u32 {
        let took = self.console.step_instruction(4 * 89_342 * 8) as u32;
        self.keep_newest_frame();
        took
    }

    /// To the next scanline.
    pub fn step_scanline(&mut self) -> u32 {
        let took = self.console.step_scanline() as u32;
        self.keep_newest_frame();
        took
    }

    /// Whether a frame completed since the last time the planes were
    /// taken: the page paints only then.
    pub fn has_frame(&self) -> bool {
        !self.console.frames.is_empty()
    }

    /// Controller 2, the same byte as `set_pad`.
    pub fn set_pad2(&mut self, bits: u8) {
        let b = Buttons {
            a: bits & 1 != 0,
            b: bits & 2 != 0,
            select: bits & 4 != 0,
            start: bits & 8 != 0,
            up: bits & 16 != 0,
            down: bits & 32 != 0,
            left: bits & 64 != 0,
            right: bits & 128 != 0,
        };
        self.console.set_pad(1, b);
    }

    // ------------------------------------------------------------------
    // Reads, for the roof's workbench: the machine as it stands, with no
    // side effect (`Console`'s reads say what each is and is not). Flat
    // byte vectors, because that is what crosses the boundary cheaply.
    // ------------------------------------------------------------------

    /// A, X, Y, S, P, PC low, PC high, then the last opcode fetch's
    /// address low and high and the opcode: ten bytes.
    pub fn cpu_state(&self) -> Vec<u8> {
        let (a, x, y, s, p, pc) = self.console.cpu_registers();
        let (fpc, op) = self.console.last_fetch();
        vec![a, x, y, s, p, pc as u8, (pc >> 8) as u8, fpc as u8, (fpc >> 8) as u8, op]
    }

    /// `len` bytes of the CPU bus from `at`, wrapping at $FFFF, with no
    /// side effect: registers answer with the open bus.
    pub fn peek(&self, at: u16, len: u16) -> Vec<u8> {
        (0..len).map(|i| self.console.peek(at.wrapping_add(i))).collect()
    }

    /// The PPU: line low, line high, dot low, dot high, ctrl, mask,
    /// v low, v high, t low, t high, fine x, w, OAM address, vblank,
    /// sprite 0 hit (1 if this frame), its line low, high, dot low, high,
    /// sprite overflow: twenty bytes.
    pub fn ppu_state(&self) -> Vec<u8> {
        let s = self.console.ppu_status();
        let (hit, hl, hd) = match s.spr0_hit {
            Some((l, d)) => (1u8, l as u16, d as u16),
            None => (0, 0, 0),
        };
        vec![
            s.line as u8, (s.line >> 8) as u8, s.dot as u8, (s.dot >> 8) as u8,
            s.ctrl, s.mask, s.v as u8, (s.v >> 8) as u8, s.t as u8, (s.t >> 8) as u8,
            s.fine_x, s.w as u8, s.oamaddr, s.vbl as u8,
            hit, hl as u8, (hl >> 8) as u8, hd as u8, (hd >> 8) as u8, s.spr_overflow as u8,
        ]
    }

    /// Palette RAM, 32 bytes.
    pub fn palette(&self) -> Vec<u8> {
        self.console.ppu_palette().to_vec()
    }

    /// OAM, 256 bytes.
    pub fn oam(&self) -> Vec<u8> {
        self.console.ppu_oam().to_vec()
    }

    /// The nametable RAM, 2 KiB in the chip's order.
    pub fn ciram(&self) -> Vec<u8> {
        self.console.ciram()
    }

    /// The console's CHR-RAM, 8 KiB on a board that keeps it here; empty
    /// where the cartridge carries its own CHR (the file has the tiles).
    pub fn chr_ram(&self) -> Vec<u8> {
        self.console.chr_ram().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::Machine;
    use nes_console::testrom;

    fn ines() -> Vec<u8> {
        let mut rom = vec![0x4e, 0x45, 0x53, 0x1a, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        rom.extend(testrom::program());
        rom.extend(testrom::chr());
        rom
    }

    #[test]
    fn the_reads_cross_the_boundary_as_the_documented_bytes() {
        let mut m = Machine::new(&ines()).expect("the plumbing cartridge loads");
        m.run_frames(2);
        let cpu = m.cpu_state();
        assert_eq!(cpu.len(), 10);
        let pc = cpu[5] as u16 | ((cpu[6] as u16) << 8);
        assert!(pc >= 0x8000, "the PC is in the cartridge: {pc:#06x}");
        let fpc = cpu[7] as u16 | ((cpu[8] as u16) << 8);
        assert!(fpc >= 0x8000);
        assert_eq!(m.peek(0xfffc, 2).len(), 2);
        assert_eq!(m.peek(0xffff, 3).len(), 3, "wraps rather than fails");
        let ppu = m.ppu_state();
        assert_eq!(ppu.len(), 20);
        let line = ppu[0] as u16 | ((ppu[1] as u16) << 8);
        let dot = ppu[2] as u16 | ((ppu[3] as u16) << 8);
        assert!(line < 262 && dot < 341);
        assert_eq!(m.palette().len(), 32);
        assert_eq!(m.oam().len(), 256);
        assert_eq!(m.ciram().len(), 0x800);
        assert!(m.chr_ram().is_empty(), "CHR-ROM in the file: nothing here");
        let before = m.cpu_half_cycles();
        let _ = (m.cpu_state(), m.peek(0, 256), m.ppu_state(), m.palette(), m.oam(), m.ciram());
        assert_eq!(m.cpu_half_cycles(), before, "reads take no time");
    }

    #[test]
    fn the_steps_move_the_machine_by_their_units() {
        let mut m = Machine::new(&ines()).expect("the plumbing cartridge loads");
        m.run_frames(1);
        let h = m.cpu_half_cycles();
        // From wherever the frame left the phase: two half-cycles is at most
        // twenty-four master half-steps, and at least thirteen.
        let took = m.step_half_cycles(2);
        assert!((13..=24).contains(&took), "{took}");
        assert_eq!(m.cpu_half_cycles(), h + 2);
        let ppu = m.ppu_state();
        let line = ppu[0] as u16 | ((ppu[1] as u16) << 8);
        m.step_scanline();
        let ppu = m.ppu_state();
        assert_eq!(ppu[0] as u16 | ((ppu[1] as u16) << 8), (line + 1) % 262);
        let took = m.step_instruction();
        assert!(took > 0 && took < 4 * 89_342 * 8);
        m.set_pad2(0x81);
        m.reset();
        assert!(m.cpu_half_cycles() > h);
    }
}

#[cfg(target_arch = "wasm32")]
mod bridge {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct Nes {
        m: super::Machine,
    }

    #[wasm_bindgen]
    impl Nes {
        #[wasm_bindgen(constructor)]
        pub fn new(rom: &[u8]) -> Result<Nes, JsValue> {
            super::Machine::new(rom).map(|m| Nes { m }).map_err(|e| JsValue::from_str(&e))
        }

        pub fn run_frames(&mut self, n: u32) {
            self.m.run_frames(n as usize);
        }

        pub fn colour(&self) -> Vec<u8> {
            self.m.colour()
        }

        pub fn emphasis(&self) -> Vec<u8> {
            self.m.emphasis()
        }

        pub fn parity(&self) -> u8 {
            self.m.parity()
        }

        pub fn sound(&mut self) -> Vec<f32> {
            self.m.sound()
        }

        pub fn set_pad(&mut self, bits: u8) {
            self.m.set_pad(bits);
        }

        pub fn cpu_half_cycles(&self) -> f64 {
            self.m.cpu_half_cycles() as f64
        }

        pub fn has_battery(&self) -> bool {
            self.m.has_battery()
        }

        pub fn battery_ram(&self) -> Vec<u8> {
            self.m.battery_ram()
        }

        pub fn set_battery_ram(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
            self.m.set_battery_ram(bytes).map_err(|e| JsValue::from_str(&e))
        }

        pub fn reset(&mut self) {
            self.m.reset();
        }

        pub fn step_half_cycles(&mut self, n: u32) -> u32 {
            self.m.step_half_cycles(n)
        }

        pub fn step_instruction(&mut self) -> u32 {
            self.m.step_instruction()
        }

        pub fn step_scanline(&mut self) -> u32 {
            self.m.step_scanline()
        }

        pub fn has_frame(&self) -> bool {
            self.m.has_frame()
        }

        pub fn set_pad2(&mut self, bits: u8) {
            self.m.set_pad2(bits);
        }

        pub fn cpu_state(&self) -> Vec<u8> {
            self.m.cpu_state()
        }

        pub fn peek(&self, at: u16, len: u16) -> Vec<u8> {
            self.m.peek(at, len)
        }

        pub fn ppu_state(&self) -> Vec<u8> {
            self.m.ppu_state()
        }

        pub fn palette(&self) -> Vec<u8> {
            self.m.palette()
        }

        pub fn oam(&self) -> Vec<u8> {
            self.m.oam()
        }

        pub fn ciram(&self) -> Vec<u8> {
            self.m.ciram()
        }

        pub fn chr_ram(&self) -> Vec<u8> {
            self.m.chr_ram()
        }
    }
}
