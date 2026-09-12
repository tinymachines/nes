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
}

impl Machine {
    pub fn new(rom: &[u8]) -> Result<Machine, String> {
        let r = ines::parse(rom).map_err(|e| format!("{e:?}"))?;
        let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
        let cart = r.cart().map_err(|e| format!("{e:?}"))?;
        let mut console = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
        console.sound = Some(Sound::default());
        Ok(Machine { console })
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
    }
}
