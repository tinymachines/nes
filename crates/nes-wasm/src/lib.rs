//! The console behind a wasm-bindgen surface (N8 step 3): a ROM in,
//! frames (the 341 x 262 colour and emphasis planes the roof's NES
//! pipeline eats) and 48 kHz sound out, the pad in. Plain Rust with a
//! thin shell gated on the target, ntsc-wasm's shape, so the native
//! bench and the browser run the same code.

#![forbid(unsafe_code)]

use nes_console::record::{self, Played, Replay};
use nes_console::writes::{Kind, WriteLog};
use nes_console::{ines, Alignment, Console, Sound};
use nes_glue::controller::Buttons;

pub struct Machine {
    console: Console,
    battery: bool,
    /// Trace bytes dropped off the front of the history so far: the
    /// history's end is this plus what it holds, a place that survives
    /// the trimming (`history_end`, `history_cut`).
    history_base: u64,
    /// Whether the history is asked for: the trace is also on, and not
    /// kept, while only the write log reads it.
    history: bool,
    /// The write log while it is on (`set_write_log`), and how much of
    /// the trace it has read.
    writes: Option<WriteLog>,
    writes_at: usize,
    /// MUTATE_WRITELOG_FEED=1, read as the log is turned on: frames run
    /// together are fed as one, every write counted to the last, and a
    /// stop's trace is not cut at a picture's end. Both of the write
    /// log's comparisons with `storyboard` must go red.
    writes_mutant: bool,
    /// The image it was powered on from: a saved state loads into a
    /// console powered on from it again, and names it by digest.
    rom: Vec<u8>,
}

/// The history kept: a megabyte of trace, some thirty thousand
/// instructions; it is trimmed back to this once it holds twice as much.
const HISTORY_KEEP: usize = 1 << 20;

/// FNV-1a over a ROM image: what a saved state names its cartridge by,
/// so a state is refused on any other image.
fn rom_digest(rom: &[u8]) -> u64 {
    rom.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// A saved state as the page keeps it: the ROM's digest, then the
/// console's own bytes (`nes_console::state`).
fn state_with_rom(rom: &[u8], console: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + console.len());
    out.extend_from_slice(&rom_digest(rom).to_le_bytes());
    out.extend_from_slice(console);
    out
}

/// A console powered on from `rom` and loaded from `state`, which must
/// name that ROM; with the sound stage the page plays through, or
/// without it, as a replay runs.
fn console_at(rom: &[u8], state: &[u8], sound: bool) -> Result<Console, String> {
    if state.len() < 8 || u64::from_le_bytes(state[..8].try_into().unwrap()) != rom_digest(rom) {
        return Err("this saved state was made on another cartridge image".into());
    }
    let (mut console, _) = power_on(rom)?;
    if sound {
        console.sound = Some(Sound::default());
    }
    console.load_state(&state[8..])?;
    Ok(console)
}

/// The console a ROM powers on into, and whether its header says a
/// battery keeps its RAM: the one construction the page's console and a
/// replay share, so a replay is the machine that was played.
fn power_on(rom: &[u8]) -> Result<(Console, bool), String> {
    let r = ines::parse(rom).map_err(|e| format!("{e:?}"))?;
    let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = r.cart().map_err(|e| format!("{e:?}"))?;
    Ok((Console::with_prg_ram(cart, chr_ram, Alignment::default(), true), r.battery))
}

impl Machine {
    pub fn new(rom: &[u8]) -> Result<Machine, String> {
        let (mut console, battery) = power_on(rom)?;
        console.sound = Some(Sound::default());
        Ok(Machine { console, battery, history_base: 0, history: false, writes: None, writes_at: 0, writes_mutant: false, rom: rom.to_vec() })
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
        if self.writes.is_some() {
            // A frame at a time, as `storyboard` runs: the log's frame
            // is the trace of one.
            let each = if self.writes_mutant { n.max(1) } else { 1 };
            let mut left = n;
            while left > 0 {
                let k = each.min(left);
                self.console.run_frames(k);
                self.feed_writes(true);
                left -= k;
            }
        } else {
            self.console.run_frames(n);
        }
        self.trim_history();
        if self.console.frames.len() > 1 {
            let last = self.console.frames.pop().unwrap();
            self.console.frames.clear();
            self.console.frames.push(last);
        }
    }

    /// `run_frames` with breakpoints: stops as the CPU begins fetching the
    /// opcode at one of `at` and answers that address, or -1 when the
    /// frames ran out first (`Console::run_frames_until`). The newest
    /// picture is kept as `run_frames` keeps it; a stop before any
    /// completed keeps the one before (`frames_done` says how many did).
    pub fn run_frames_until(&mut self, n: usize, at: &[u16]) -> i32 {
        let hit = if self.writes.is_some() {
            let mut hit = None;
            for _ in 0..n {
                hit = self.console.run_frames_until(1, at);
                self.feed_writes(hit.is_none());
                if hit.is_some() {
                    break;
                }
            }
            hit
        } else {
            self.console.run_frames_until(n, at)
        };
        self.keep_newest_frame();
        hit.map_or(-1, |a| a as i32)
    }

    /// Pictures completed since power-on.
    pub fn frames_done(&self) -> u64 {
        self.console.frames_done
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

    // ------------------------------------------------------------------
    // History, for a debugger: the console's own trace (`record`'s 8-byte
    // records: every CPU cycle's bus, the registers at each instruction)
    // while it is on, the newest HISTORY_KEEP bytes of it kept. The page
    // reads the tail to show the instructions just run, and cuts it back
    // to where it was when it steps back to a saved moment.
    // ------------------------------------------------------------------

    /// Tracing costs time, so the history is off until asked for.
    pub fn set_history(&mut self, on: bool) {
        if on && !self.history {
            self.feed_writes(false);
            self.console.trace.get_or_insert_with(Default::default);
            self.history_base = 0;
        } else if !on {
            if self.writes.is_none() {
                self.console.trace = None;
            }
            self.history_base = 0;
        }
        self.history = on;
    }

    /// The newest `max` bytes of the history (a whole number of records).
    pub fn history(&self, max: usize) -> Vec<u8> {
        let Some(t) = self.console.trace.as_ref().filter(|_| self.history) else { return Vec::new() };
        let n = max.min(t.bytes.len()) & !7;
        t.bytes[t.bytes.len() - n..].to_vec()
    }

    /// Where the history ends: every byte it has ever held, counted.
    pub fn history_end(&self) -> u64 {
        self.history_base + self.console.trace.as_ref().filter(|_| self.history).map_or(0, |t| t.bytes.len() as u64)
    }

    /// Back to where the history ended at `end` (`history_end` then): what
    /// was traced after it did not happen once the machine is put back.
    pub fn history_cut(&mut self, end: u64) {
        let base = self.history_base;
        if let Some(t) = self.console.trace.as_mut() {
            if end <= base {
                t.bytes.clear();
                self.history_base = end;
            } else if ((end - base) as usize) < t.bytes.len() {
                t.bytes.truncate((end - base) as usize);
            }
            self.writes_at = self.writes_at.min(t.bytes.len());
        }
    }

    fn trim_history(&mut self) {
        if let Some(t) = self.console.trace.as_mut() {
            if t.bytes.len() > 2 * HISTORY_KEEP {
                let cut = (t.bytes.len() - HISTORY_KEEP) & !7;
                t.bytes.drain(..cut);
                self.history_base += cut as u64;
                self.writes_at = self.writes_at.saturating_sub(cut);
            }
        }
    }

    // ------------------------------------------------------------------
    // The write log, for the roof's lessons (`nes_console::writes`): what
    // the program wrote to the picture chip's memory, the scroll, the
    // sound chip and the cartridge's own range, a line a write, as
    // `storyboard` writes them to vram.txt, scroll.txt, apu.txt and
    // mapper.txt. Read out of the console's trace as the machine runs.
    // ------------------------------------------------------------------

    /// The write log reads the trace, which costs time, so it is off
    /// until asked for. Turned on at power-on it is `storyboard`'s log
    /// line for line (a frame is the count of pictures completed before
    /// it); turned on later it starts from there, and a saved state
    /// loaded does not put the log's own view of the picture chip back
    /// (`writes`). Turning it off drops what was not taken. It grows
    /// until it is taken.
    pub fn set_write_log(&mut self, on: bool) {
        if on && self.writes.is_none() {
            let t = self.console.trace.get_or_insert_with(Default::default);
            self.writes_at = t.bytes.len();
            self.writes = Some(WriteLog::default());
            self.writes_mutant = std::env::var_os("MUTATE_WRITELOG_FEED").is_some();
        } else if !on {
            self.writes = None;
            if !self.history {
                self.console.trace = None;
            }
        }
    }

    pub fn write_log(&self) -> bool {
        self.writes.is_some()
    }

    /// One log's lines so far, which empties it: `kind` is `vram`,
    /// `scroll`, `apu` or `mapper`, the file `storyboard` writes them to.
    /// Empty while the log is off. A run of $2007 writes is a line once
    /// it has ended, so one under way at a stop inside a frame comes with
    /// a later take.
    pub fn take_writes(&mut self, kind: &str) -> Result<String, String> {
        let kind = Kind::named(kind).ok_or_else(|| format!("no write log is called {kind:?}: vram, scroll, apu or mapper"))?;
        self.feed_writes(false);
        Ok(self.writes.as_mut().map(|w| w.take(kind)).unwrap_or_default())
    }

    /// The trace since the last look, into the write log. `ended`: the
    /// console has just run exactly to a picture's end, and what is new
    /// is that frame's, whole or the rest of it. Otherwise (a step, a
    /// breakpoint) what is new is cut at each picture's own record: a
    /// CPU cycle on the very master half-step a picture ended on is then
    /// counted to the next frame, where a run counts it to that one.
    fn feed_writes(&mut self, ended: bool) {
        let (Some(w), Some(t)) = (self.writes.as_mut(), self.console.trace.as_mut()) else { return };
        let new = &t.bytes[self.writes_at..];
        if ended {
            w.frame(self.console.frames_done - 1, new);
        } else {
            let mut from = 0;
            for (i, r) in new.chunks_exact(record::RECORD_BYTES).enumerate() {
                if r[3] & 0xc0 == record::KIND_FRAME && !self.writes_mutant {
                    let at = (i + 1) * record::RECORD_BYTES;
                    w.frame(u32::from_le_bytes([r[4], r[5], r[6], r[7]]) as u64, &new[from..at]);
                    from = at;
                }
            }
            w.feed(self.console.frames_done, &new[from..]);
        }
        if self.history {
            self.writes_at = t.bytes.len();
        } else {
            t.bytes.clear();
            self.writes_at = 0;
        }
    }

    fn keep_newest_frame(&mut self) {
        self.feed_writes(false);
        self.trim_history();
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

    // ------------------------------------------------------------------
    // Recording, for the roof's flow tools (`nes_console::record`): the
    // inputs and a digest of every picture, from power-on. The trace is
    // not kept here; a `Replayer` makes it from the log.
    // ------------------------------------------------------------------

    /// Start logging. Refused once the console has run: a recording
    /// replays from power-on, so it starts there (the page loads the
    /// cartridge again, puts the save back, then records).
    pub fn record_start(&mut self) -> Result<(), String> {
        if self.console.master != 0 {
            return Err("a recording starts at power-on: load the cartridge again, then record".into());
        }
        self.console.inputs = Some(Default::default());
        Ok(())
    }

    pub fn recording(&self) -> bool {
        self.console.inputs.is_some()
    }

    /// The whole machine as bytes, taken where the CPU's current cycle
    /// ends (a few master half-steps on at most: the machine runs there).
    pub fn save_state(&mut self) -> Result<Vec<u8>, String> {
        self.console.run_to_cycle_end();
        self.feed_writes(false);
        Ok(state_with_rom(&self.rom, &self.console.save_state()?))
    }

    /// The machine as `state` left it. Refused, and the machine left as
    /// it was, for another cartridge's state, a state this build cannot
    /// read, or while a recording runs (a recording is one run from where
    /// it started).
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), String> {
        if self.console.inputs.is_some() {
            return Err("stop the recording before loading a saved state".into());
        }
        let mut console = console_at(&self.rom, state, true)?;
        // The history is the page's instrument, not the machine's state:
        // it stays, and the page cuts it back to the moment (`history_cut`).
        console.trace = self.console.trace.take();
        self.console = console;
        Ok(())
    }

    /// Start logging from here rather than from power-on: the machine
    /// runs to the end of the CPU's cycle, and the state it stands in
    /// there is what a replay starts from, returned for the page to keep
    /// with the log.
    pub fn record_start_here(&mut self) -> Result<Vec<u8>, String> {
        if self.console.inputs.is_some() {
            return Err("a recording is already running".into());
        }
        let state = self.save_state()?;
        self.console.inputs = Some(Default::default());
        Ok(state)
    }

    /// Stop logging: the log, ended here. Empty if nothing was recording.
    pub fn record_stop(&mut self) -> Vec<u8> {
        let m = self.console.master;
        match self.console.inputs.take() {
            Some(mut l) => {
                l.push(record::END, 0, 0, 0, m);
                l.bytes
            }
            None => Vec::new(),
        }
    }

    /// The log so far, ended here, while the recording carries on: what
    /// the page keeps as it goes, so a recording outlives a page that is
    /// closed without stopping it. Empty if nothing is recording.
    pub fn record_so_far(&self) -> Vec<u8> {
        match &self.console.inputs {
            Some(l) => {
                let mut l = l.clone();
                l.push(record::END, 0, 0, 0, self.console.master);
                l.bytes
            }
            None => Vec::new(),
        }
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

    /// The pattern memory as the picture chip sees it now, 8 KiB through
    /// the board's banks as they stand (`Console::chr`); empty for a
    /// board that cannot say.
    pub fn chr(&self) -> Vec<u8> {
        self.console.chr().unwrap_or_default()
    }
}

/// A recording played back into a console that has just powered on, with
/// the trace on: the run itself, a chunk at a time, every picture held
/// to the log's digest (a replay that parts from the log stops and says
/// where). No sound: nothing the CPU does depends on it.
pub struct Replayer {
    console: Console,
    replay: Replay,
}

impl Replayer {
    /// `battery` is the cartridge RAM the recording started with (empty
    /// for none), put back before the first step, as the page does.
    pub fn new(rom: &[u8], battery: &[u8], log: &[u8]) -> Result<Replayer, String> {
        let replay = Replay::new(log)?;
        let (mut console, _) = power_on(rom)?;
        if !battery.is_empty() {
            console.set_battery_ram(battery)?;
        }
        console.trace = Some(Default::default());
        Ok(Replayer { console, replay })
    }

    /// A replay of a recording that started from a saved state
    /// (`Machine::record_start_here`) rather than from power-on.
    pub fn from_state(rom: &[u8], state: &[u8], log: &[u8]) -> Result<Replayer, String> {
        let replay = Replay::new(log)?;
        let mut console = console_at(rom, state, false)?;
        console.trace = Some(Default::default());
        Ok(Replayer { console, replay })
    }

    /// Play `pictures` more (or to the log's end); true when it ended.
    pub fn run(&mut self, pictures: u32) -> Result<bool, String> {
        let played = self.replay.run(&mut self.console, pictures as u64)?;
        self.console.frames.clear();
        Ok(played == Played::Ended)
    }

    /// The trace written since the last take (`record`'s 8-byte records).
    pub fn take_trace(&mut self) -> Vec<u8> {
        self.console.trace.as_mut().map(|t| std::mem::take(&mut t.bytes)).unwrap_or_default()
    }

    /// The pictures the log records, and how many the replay has matched.
    pub fn frames(&self) -> u32 {
        self.replay.frames() as u32
    }

    pub fn frames_checked(&self) -> u32 {
        self.replay.frames_checked() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::Machine;
    use nes_console::testrom;
    use nes_console::writes::{Kind, WriteLog};

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
    fn a_recording_made_on_the_page_replays_with_its_trace() {
        let mut m = Machine::new(&ines()).expect("the plumbing cartridge loads");
        m.run_frames(1);
        assert!(m.record_start().is_err(), "a recording starts at power-on");
        let mut m = Machine::new(&ines()).unwrap();
        m.record_start().unwrap();
        assert!(m.recording());
        for pad in [0u8, 8, 8, 0, 0x80] {
            m.set_pad(pad);
            m.run_frames(4);
        }
        m.reset();
        m.run_frames(3);
        let log = m.record_stop();
        assert!(!m.recording());
        assert!(m.record_stop().is_empty());
        let mut r = super::Replayer::new(&ines(), &[], &log).expect("the log replays");
        assert_eq!(r.frames(), 5 * 4 + 1 + 3, "every picture, the reset's hold included");
        let mut trace = 0;
        while !r.run(5).expect("each picture matches") {
            trace += r.take_trace().len();
        }
        trace += r.take_trace().len();
        assert_eq!(r.frames_checked(), r.frames());
        assert!(trace > 24 * 29_000 * 8, "{trace} bytes of trace");
    }

    #[test]
    fn a_recording_so_far_replays_to_where_it_was_taken_and_the_recording_carries_on() {
        let mut m = Machine::new(&ines()).unwrap();
        assert!(m.record_so_far().is_empty(), "nothing recording, nothing so far");
        m.record_start().unwrap();
        for pad in [0u8, 8, 0x80] {
            m.set_pad(pad);
            m.run_frames(4);
        }
        let early = m.record_so_far();
        assert!(m.recording(), "taking a copy does not stop it");
        m.set_pad(4);
        m.run_frames(5);
        let log = m.record_stop();
        // The copy is the log as it stood, with its own END.
        assert_eq!(&log[..early.len() - 16], &early[..early.len() - 16]);
        assert!(log.len() > early.len());
        let mut r = super::Replayer::new(&ines(), &[], &early).expect("the copy replays");
        assert_eq!(r.frames(), 12);
        while !r.run(5).expect("each picture matches") {}
        assert_eq!(r.frames_checked(), 12);
        let mut r = super::Replayer::new(&ines(), &[], &log).expect("the whole log replays");
        assert_eq!(r.frames(), 17);
        while !r.run(5).expect("each picture matches") {}
        assert_eq!(r.frames_checked(), 17);
    }

    #[test]
    fn a_recording_started_mid_game_replays_from_its_state() {
        let mut m = Machine::new(&ines()).unwrap();
        m.set_pad(0x80);
        m.run_frames(7);
        m.step_half_cycles(5);
        let state = m.record_start_here().expect("a recording starts anywhere");
        assert!(m.record_start_here().is_err(), "one at a time");
        for pad in [0u8, 8, 0x81] {
            m.set_pad(pad);
            m.run_frames(4);
        }
        m.reset();
        m.run_frames(2);
        let log = m.record_stop();
        let mut r = super::Replayer::from_state(&ines(), &state, &log).expect("the state and log replay");
        assert_eq!(r.frames(), 3 * 4 + 1 + 2, "every picture after the start, the reset's hold included");
        let mut trace = 0;
        while !r.run(5).expect("each picture matches") {
            trace += r.take_trace().len();
        }
        assert_eq!(r.frames_checked(), r.frames());
        assert!(trace > 0);
    }

    #[test]
    fn a_saved_state_loads_into_the_same_game_and_is_refused_by_another() {
        let mut a = Machine::new(&ines()).unwrap();
        a.set_pad(0x81);
        a.run_frames(3);
        a.step_half_cycles(7);
        let state = a.save_state().unwrap();
        let mut b = Machine::new(&ines()).unwrap();
        b.load_state(&state).expect("the same cartridge");
        for _ in 0..5 {
            a.run_frames(1);
            b.run_frames(1);
            assert_eq!(a.colour(), b.colour());
            assert_eq!(a.sound(), b.sound());
        }
        let mut other = ines();
        other[0x10 + 0x100] ^= 0xff;
        let mut c = Machine::new(&other).unwrap();
        let before = c.cpu_half_cycles();
        assert!(c.load_state(&state).unwrap_err().contains("another cartridge"));
        assert_eq!(c.cpu_half_cycles(), before, "a refused load leaves the machine as it was");
        let mut garbled = state.clone();
        garbled[8] ^= 1;
        assert!(b.load_state(&garbled).is_err());
    }

    #[test]
    fn a_breakpoint_stops_the_page_s_run_and_keeps_a_picture_to_show() {
        let mut m = Machine::new(&ines()).unwrap();
        m.run_frames(2);
        let nmi = m.peek(0xfffa, 2);
        let nmi = nmi[0] as u16 | (nmi[1] as u16) << 8;
        let before = m.frames_done();
        let hit = m.run_frames_until(5, &[nmi]);
        assert_eq!(hit, nmi as i32);
        assert!(m.frames_done() - before < 5);
        assert!(!m.colour().is_empty(), "a picture to show at the stop");
        assert_eq!(m.run_frames_until(3, &[0x0123]), -1);
    }

    #[test]
    fn the_history_holds_the_instructions_just_run_and_cuts_back_to_a_moment() {
        let mut m = Machine::new(&ines()).unwrap();
        assert!(m.history(1 << 16).is_empty(), "off until asked for");
        m.set_history(true);
        m.run_frames(1);
        let end = m.history_end();
        let state = m.save_state().unwrap();
        let at_save = m.history_end();
        assert!(at_save >= end);
        for _ in 0..20 {
            m.step_instruction();
        }
        // The last register record follows the fetch of the instruction
        // the machine last began: its address is the fetch cycle's.
        let h = m.history(4096);
        assert_eq!(h.len() % 8, 0);
        let recs: Vec<&[u8]> = h.chunks(8).collect();
        let regs = recs.iter().rposition(|r| r[3] >> 6 == 1).expect("a register record");
        let fetch = recs[..regs].iter().rposition(|r| r[3] >> 6 == 0 && r[3] & 2 != 0).expect("its fetch");
        let pc = u16::from_le_bytes([recs[fetch][0], recs[fetch][1]]);
        assert_eq!(pc, m.console.last_fetch().0);
        // Back to the moment: the history ends where it ended then.
        m.load_state(&state).unwrap();
        m.history_cut(at_save);
        assert_eq!(m.history_end(), at_save);
        // A long run is trimmed, and the end still counts every byte.
        let before = m.history_end();
        m.run_frames(12);
        assert!(m.history(usize::MAX).len() <= 2 * super::HISTORY_KEEP);
        assert!(m.history_end() > before + super::HISTORY_KEEP as u64);
        m.set_history(false);
        assert!(m.history(4096).is_empty());
    }

    /// `storyboard`'s run of `rom`, the four logs in `Kind::ALL`'s order:
    /// the console powered on, its trace on, a frame at a time.
    fn storyboard(rom: &[u8], frames: u64) -> [String; 4] {
        let (mut console, _) = super::power_on(rom).unwrap();
        console.trace = Some(Default::default());
        let mut log = WriteLog::default();
        for f in 0..frames {
            console.run_frames(1);
            let tr = console.trace.as_mut().unwrap();
            log.frame(f, &tr.bytes);
            tr.bytes.clear();
            console.frames.clear();
        }
        Kind::ALL.map(|k| log.take(k))
    }

    fn take_all(m: &mut Machine, into: &mut [String; 4]) {
        for (k, text) in Kind::ALL.into_iter().zip(into.iter_mut()) {
            text.push_str(&m.take_writes(k.name()).unwrap());
        }
    }

    /// The page's run of `rom`: the same frames in uneven runs, the logs
    /// taken at uneven moments, the history turned on and off under it.
    fn paged(rom: &[u8], frames: usize) -> [String; 4] {
        let mut m = Machine::new(rom).unwrap();
        m.set_write_log(true);
        let mut got: [String; 4] = Default::default();
        let (mut done, mut i) = (0, 0);
        while done < frames {
            let n = [1, 3, 7, 2, 16, 1, 5][i % 7].min(frames - done);
            if i == 9 {
                m.set_history(true);
            }
            if i == 30 {
                m.set_history(false);
            }
            m.run_frames(n);
            done += n;
            if i % 3 == 0 {
                take_all(&mut m, &mut got);
            }
            i += 1;
        }
        take_all(&mut m, &mut got);
        got
    }

    #[test]
    fn the_write_log_is_storyboard_s_however_the_page_runs_the_frames() {
        // The write log's cartridge first: every log has a line a frame.
        let want = storyboard(&testrom::writes_rom(), 300);
        for (k, text) in Kind::ALL.into_iter().zip(&want) {
            assert!(text.lines().count() > 250, "{}: {} lines is not a log worth comparing", k.name(), text.lines().count());
        }
        let got = paged(&testrom::writes_rom(), 300);
        for ((k, got), want) in Kind::ALL.into_iter().zip(&got).zip(&want) {
            assert!(got == want, "{}: the page's log parts from storyboard's at line {}", k.name(), got.lines().zip(want.lines()).take_while(|(a, b)| a == b).count());
        }
        // The test cartridge: its setup's few lines, and no mapper line
        // at all (an NROM), so of it the mapper log is not compared.
        let want = storyboard(&ines(), 300);
        assert_eq!(want.each_ref().map(|t| t.lines().count()), [3, 1, 1, 0]);
        assert_eq!(paged(&ines(), 300), want);
    }

    #[test]
    fn the_write_log_is_off_until_asked_for_and_stops_when_told() {
        let mut m = Machine::new(&testrom::writes_rom()).unwrap();
        assert!(!m.write_log());
        m.run_frames(5);
        assert!(m.console.trace.is_none(), "off: the console traces nothing");
        assert_eq!(m.take_writes("apu").unwrap(), "");
        assert!(m.take_writes("ppu").is_err(), "a log that does not exist is refused by name");
        // On from here: the frames are the pictures completed, 5 and on.
        m.set_write_log(true);
        m.run_frames(3);
        let apu = m.take_writes("apu").unwrap();
        assert!(apu.lines().count() >= 8, "{apu}");
        assert!(apu.lines().all(|l| matches!(l.split(' ').next(), Some("5" | "6" | "7"))), "{apu}");
        assert_eq!(m.take_writes("apu").unwrap(), "", "taken once");
        assert!(m.history(4096).is_empty(), "the log's trace is not the history");
        assert_eq!(m.history_end(), 0);
        m.set_write_log(false);
        assert!(m.console.trace.is_none(), "off again: the trace goes with it");
        m.run_frames(2);
        assert_eq!(m.take_writes("mapper").unwrap(), "");
        // The history alone keeps its trace when the log goes.
        m.set_history(true);
        m.set_write_log(true);
        m.set_write_log(false);
        assert!(m.console.trace.is_some());
    }

    #[test]
    fn the_write_log_follows_a_machine_that_stops_inside_its_frames() {
        // Breakpoints on the NMI handler, single instructions, a step of
        // most of a frame (which crosses a picture's end inside the
        // handler, writes on both sides of it), scanlines and a saved
        // state between the runs: the same 300 pictures, and on this
        // cartridge the same lines (no write of its falls on the master
        // half-step a picture ends on, where a stop and a run would
        // count it to different frames).
        let want = storyboard(&testrom::writes_rom(), 300);
        let mut m = Machine::new(&testrom::writes_rom()).unwrap();
        m.set_write_log(true);
        let mut got: [String; 4] = Default::default();
        let nmi = m.peek(0xfffa, 2);
        let nmi = nmi[0] as u16 | (nmi[1] as u16) << 8;
        let mut i = 0;
        while m.frames_done() < 300 {
            match i % 5 {
                0 => {
                    m.run_frames_until(2, &[nmi]);
                }
                1 => {
                    for _ in 0..5 {
                        m.step_instruction();
                    }
                }
                2 => {
                    m.step_half_cycles(50_000);
                    m.step_scanline();
                    m.save_state().unwrap();
                }
                3 => take_all(&mut m, &mut got),
                _ => {
                    // To the picture's end exactly, or the last run would overshoot.
                    m.run_frames_until(1, &[]);
                }
            }
            i += 1;
        }
        take_all(&mut m, &mut got);
        // What was traced after the 300th picture is frame 300's.
        let got = got.map(|t| t.lines().filter(|l| !l.starts_with("300 ")).map(|l| format!("{l}\n")).collect::<String>());
        for ((k, got), want) in Kind::ALL.into_iter().zip(&got).zip(&want) {
            assert!(got == want, "{}: parts from storyboard's at line {}", k.name(), got.lines().zip(want.lines()).take_while(|(a, b)| a == b).count());
        }
    }

    /// The same comparison on any cartridge, by hand:
    /// NES_WRITES_ROM=rom.nes cargo test --release -p nes-wasm -- --ignored
    #[test]
    #[ignore]
    fn the_write_log_is_storyboard_s_on_a_cartridge_named() {
        let rom = std::fs::read(std::env::var("NES_WRITES_ROM").expect("NES_WRITES_ROM")).expect("the ROM");
        let frames = std::env::var("NES_WRITES_FRAMES").map_or(600, |f| f.parse().expect("NES_WRITES_FRAMES"));
        let want = storyboard(&rom, frames as u64);
        let got = paged(&rom, frames);
        for ((k, got), want) in Kind::ALL.into_iter().zip(&got).zip(&want) {
            eprintln!("{}: {} lines", k.name(), want.lines().count());
            assert!(got == want, "{}: the page's log parts from storyboard's at line {}", k.name(), got.lines().zip(want.lines()).take_while(|(a, b)| a == b).count());
        }
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

        pub fn run_frames_until(&mut self, n: u32, at: &[u16]) -> i32 {
            self.m.run_frames_until(n as usize, at)
        }

        pub fn frames_done(&self) -> f64 {
            self.m.frames_done() as f64
        }

        pub fn set_history(&mut self, on: bool) {
            self.m.set_history(on)
        }

        pub fn history(&self, max: u32) -> Vec<u8> {
            self.m.history(max as usize)
        }

        pub fn history_end(&self) -> f64 {
            self.m.history_end() as f64
        }

        pub fn history_cut(&mut self, end: f64) {
            self.m.history_cut(end as u64)
        }

        pub fn set_write_log(&mut self, on: bool) {
            self.m.set_write_log(on)
        }

        pub fn write_log(&self) -> bool {
            self.m.write_log()
        }

        /// `kind`: `vram`, `scroll`, `apu` or `mapper`.
        pub fn take_writes(&mut self, kind: &str) -> Result<String, JsValue> {
            self.m.take_writes(kind).map_err(|e| JsValue::from_str(&e))
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

        pub fn chr(&self) -> Vec<u8> {
            self.m.chr()
        }

        pub fn record_start(&mut self) -> Result<(), JsValue> {
            self.m.record_start().map_err(|e| JsValue::from_str(&e))
        }

        pub fn recording(&self) -> bool {
            self.m.recording()
        }

        pub fn record_stop(&mut self) -> Vec<u8> {
            self.m.record_stop()
        }

        pub fn save_state(&mut self) -> Result<Vec<u8>, JsValue> {
            self.m.save_state().map_err(|e| JsValue::from_str(&e))
        }

        pub fn load_state(&mut self, state: &[u8]) -> Result<(), JsValue> {
            self.m.load_state(state).map_err(|e| JsValue::from_str(&e))
        }

        pub fn record_start_here(&mut self) -> Result<Vec<u8>, JsValue> {
            self.m.record_start_here().map_err(|e| JsValue::from_str(&e))
        }

        pub fn record_so_far(&self) -> Vec<u8> {
            self.m.record_so_far()
        }
    }

    #[wasm_bindgen]
    pub struct NesReplay {
        r: super::Replayer,
    }

    #[wasm_bindgen]
    impl NesReplay {
        #[wasm_bindgen(constructor)]
        pub fn new(rom: &[u8], battery: &[u8], log: &[u8]) -> Result<NesReplay, JsValue> {
            super::Replayer::new(rom, battery, log).map(|r| NesReplay { r }).map_err(|e| JsValue::from_str(&e))
        }

        /// A recording that started from a saved state.
        pub fn from_state(rom: &[u8], state: &[u8], log: &[u8]) -> Result<NesReplay, JsValue> {
            super::Replayer::from_state(rom, state, log).map(|r| NesReplay { r }).map_err(|e| JsValue::from_str(&e))
        }

        pub fn run(&mut self, pictures: u32) -> Result<bool, JsValue> {
            self.r.run(pictures).map_err(|e| JsValue::from_str(&e))
        }

        pub fn take_trace(&mut self) -> Vec<u8> {
            self.r.take_trace()
        }

        pub fn frames(&self) -> u32 {
            self.r.frames()
        }

        pub fn frames_checked(&self) -> u32 {
            self.r.frames_checked()
        }
    }
}
