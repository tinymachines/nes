//! The write log: what a program wrote to the registers a reader of a
//! game asks about, read out of the console's trace (`record`) and kept
//! as text, a line a write. Four logs, each the file `storyboard` writes
//! under the same name, in the order the writes were made:
//!
//! | log | a line | |
//! |---|---|---|
//! | `vram` | `frame address count step` | a run of writes to the picture chip's memory through $2006/$2007: where it started (in hex, before the chip's mirroring), how many, and the step, 1 or 32 as $2000 set it |
//! | `scroll` | `frame line x y` | a pair of writes to $2005, with the picture line the second fell on (below 240 the picture was being drawn: a split; 240 and on, the blank, which sets the next picture's scroll) |
//! | `apu` | `frame register value` | a write to the sound chip ($4000 to $4013, $4015, $4017), the register as its low byte in hex |
//! | `mapper` | `frame address value` | a write into the cartridge's own range ($8000 and up, where a board keeps its registers), the address in hex |
//!
//! The frame is the caller's count, because the trace does not say which
//! side of a picture's end a cycle on the same master half-step fell.
//! `frame` is `storyboard`'s unit, the trace of one `run_frames(1)`; a
//! machine stopped part way through a picture uses `feed`, and `end_frame`
//! when the picture completes (nes-wasm's `Machine` does both).
//!
//! The log follows the program's view of the picture chip from the
//! writes it is fed (the two-write latch, the address, the step), so it
//! is on from power-on or it is approximate until the program next sets
//! them, which is every frame in practice.

use std::fmt::Write as _;

use crate::record::{F_HELD, F_READ, KIND_REGS, RECORD_BYTES};

/// One of the four logs, by the name its file has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Vram,
    Scroll,
    Apu,
    Mapper,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Vram, Kind::Scroll, Kind::Apu, Kind::Mapper];

    /// `vram`, `scroll`, `apu`, `mapper`: `storyboard` adds `.txt`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Vram => "vram",
            Kind::Scroll => "scroll",
            Kind::Apu => "apu",
            Kind::Mapper => "mapper",
        }
    }

    pub fn named(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

pub struct WriteLog {
    pub vram: String,
    pub scroll: String,
    pub apu: String,
    pub mapper: String,
    /// The first of a pair of $2005 writes, and the picture line as the
    /// last instruction's registers gave it.
    scroll_x: u8,
    line: u16,
    // The program's view of the picture chip's address: the two-write
    // latch, the address, the step. A run is consecutive $2007 writes.
    latch: bool,
    addr: u16,
    step: u16,
    run: Option<(u16, u16, u16)>,
    /// MUTATE_WRITELOG=1 drops every write of an odd value before it is
    /// looked at, and tests/writes.rs must go red.
    mutate: bool,
}

impl Default for WriteLog {
    fn default() -> Self {
        WriteLog {
            vram: String::new(),
            scroll: String::new(),
            apu: String::new(),
            mapper: String::new(),
            scroll_x: 0,
            line: 0,
            latch: false,
            addr: 0,
            step: 1,
            run: None,
            mutate: std::env::var_os("MUTATE_WRITELOG").is_some(),
        }
    }
}

impl WriteLog {
    /// The trace of frame `f`, whole: `feed`, then `end_frame`.
    pub fn frame(&mut self, f: u64, trace: &[u8]) {
        self.feed(f, trace);
        self.end_frame(f);
    }

    /// Frame `f` completed: a run of $2007 writes does not cross a frame.
    pub fn end_frame(&mut self, f: u64) {
        self.end_run(f);
    }

    fn end_run(&mut self, f: u64) {
        if let Some((a, n, st)) = self.run.take() {
            writeln!(self.vram, "{f} {a:04X} {n} {st}").unwrap();
        }
    }

    /// Some of the trace of frame `f`, in order (`record`'s 8-byte records).
    pub fn feed(&mut self, f: u64, trace: &[u8]) {
        for r in trace.chunks_exact(RECORD_BYTES) {
            if r[3] & 0xc0 == KIND_REGS {
                self.line = u16::from_le_bytes([r[6], r[7]]); // the registers' record carries the line
            }
            if r[3] & 0xc0 != 0 || r[3] & F_HELD != 0 {
                continue; // not a CPU cycle, or a cycle the DMA held
            }
            let ab = u16::from_le_bytes([r[0], r[1]]);
            let (read, db) = (r[3] & F_READ != 0, r[2]);
            if self.mutate && !read && db & 1 != 0 {
                continue;
            }
            if !read && ab >= 0x8000 {
                writeln!(self.mapper, "{f} {ab:04X} {db}").unwrap();
            }
            if !read && ((0x4000..=0x4013).contains(&ab) || ab == 0x4015 || ab == 0x4017) {
                writeln!(self.apu, "{f} {:02X} {db}", ab & 0xff).unwrap();
            }
            if !(0x2000..0x4000).contains(&ab) {
                continue;
            }
            match (ab & 7, read) {
                (2, true) => self.latch = false,
                (0, false) => self.step = if db & 4 != 0 { 32 } else { 1 },
                (5, false) => {
                    // $2005 shares the two-write latch with $2006.
                    if self.latch {
                        writeln!(self.scroll, "{f} {} {} {db}", self.line, self.scroll_x).unwrap();
                    } else {
                        self.scroll_x = db;
                    }
                    self.latch = !self.latch;
                }
                (6, false) => {
                    self.end_run(f);
                    self.addr = if self.latch { (self.addr & 0xff00) | db as u16 } else { (self.addr & 0x00ff) | ((db as u16 & 0x3f) << 8) };
                    self.latch = !self.latch;
                }
                (7, false) => {
                    self.run = match self.run {
                        Some((a, n, st)) if st == self.step => Some((a, n + 1, st)),
                        _ => {
                            self.end_run(f);
                            Some((self.addr, 1, self.step))
                        }
                    };
                    self.addr = self.addr.wrapping_add(self.step) & 0x3fff;
                }
                _ => {}
            }
        }
    }

    /// One log's text so far, which empties it.
    pub fn take(&mut self, kind: Kind) -> String {
        std::mem::take(match kind {
            Kind::Vram => &mut self.vram,
            Kind::Scroll => &mut self.scroll,
            Kind::Apu => &mut self.apu,
            Kind::Mapper => &mut self.mapper,
        })
    }
}
