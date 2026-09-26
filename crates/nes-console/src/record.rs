//! Recording a run, and the run's trace, for a debugger (the roof's flow
//! tools).
//!
//! Two things, because one is small and the other is not:
//!
//! - **The input log** (`InputLog`) is what a player did and what came
//!   out: every pad change and reset press, each at the master half-step
//!   it happened on, and a digest of every picture. A minute of play is a
//!   few hundred kilobytes, and it is the whole recording: the console is
//!   deterministic from power-on, so the log replays to the same run.
//! - **The trace** (`Trace`) is the run itself, one record per CPU cycle,
//!   about 1.8 million a second of play. It is never kept live; a replay
//!   of the log (`Replay`) produces it, and checks every picture against
//!   the log's digest on the way, so a trace that drifted from the run it
//!   claims to be refuses rather than being analysed.
//!
//! Both are flat little-endian byte records, because that is what crosses
//! the wasm boundary cheaply and what the roof's analysis crate reads.
//!
//! ## The input log: 16 bytes an event
//!
//! `[kind, port, value, 0, x u32, master u64]`, kinds:
//!
//! | kind | | |
//! |---|---|---|
//! | 1 `PAD` | port 0 or 1, value the pad byte (A, B, Select, Start, Up, Down, Left, Right from bit 0) | x 0 |
//! | 2 `RESET` | the front panel's button | x the hold, in master half-steps |
//! | 3 `FRAME` | a picture completed | x its digest (`frame_digest`) |
//! | 4 `END` | the recording stopped | x 0 |
//!
//! An event is applied when the console's `master` equals the event's:
//! before that master half-step runs. A `FRAME` is logged at the master
//! half-step its picture completed on.
//!
//! ## The trace: 8 bytes a record, the kind in the top two bits of byte 3
//!
//! | kind | bytes |
//! |---|---|
//! | 0 `CYCLE` | address lo, hi, data, flags, then the PRG offset + 1 as u32 (0: not the ROM) |
//! | 1 `REGS` | A, X, Y, 0x40, S, P, PPU line lo, hi |
//! | 2 `INPUT` | value, port (0, 1 the pads, 2 reset), 0, 0x80, then the frame as u32 |
//! | 3 `FRAME` | digest bits 0..24, 0xC0, then the frame's index as u32 |
//!
//! `CYCLE` is the CPU's bus on the phi2 half of a cycle, where the
//! address and data are valid: one per CPU cycle, the cycles a DMA holds
//! the core included (flag `HELD`). Its flags, from bit 0: read, SYNC
//! (an opcode fetch), /NMI asserted, /IRQ asserted, held (RDY low at the
//! pins: a sprite DMA or a sample fetch has the bus). The PRG offset is
//! the cartridge's own account of where a read lands (`Cartridge::
//! prg_offset`), so code on a banked board is keyed to its place in the
//! file and not its address; it is given for reads only.
//!
//! `REGS` follows each SYNC cycle and is the registers as they stand
//! entering that instruction: the previous instruction's results have
//! landed (the test holds this against every `LDA #imm` of a run).

pub const EVENT_BYTES: usize = 16;
pub const PAD: u8 = 1;
pub const RESET: u8 = 2;
pub const FRAME: u8 = 3;
pub const END: u8 = 4;

pub const RECORD_BYTES: usize = 8;
pub const KIND_CYCLE: u8 = 0x00;
pub const KIND_REGS: u8 = 0x40;
pub const KIND_INPUT: u8 = 0x80;
pub const KIND_FRAME: u8 = 0xc0;
pub const F_READ: u8 = 1;
pub const F_SYNC: u8 = 2;
pub const F_NMI: u8 = 4;
pub const F_IRQ: u8 = 8;
pub const F_HELD: u8 = 16;

/// FNV-1a over a picture's colour plane then its emphasis plane: what
/// the log keeps of each picture, and what a replay is held to.
pub fn frame_digest(f: &nes_bus::DotFrame) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in f.colour.iter().chain(f.emphasis.iter()) {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The input log, as a console appends to it.
#[derive(Default, Clone)]
pub struct InputLog {
    pub bytes: Vec<u8>,
    /// The pads as last logged, so an unchanged set is not an event.
    pads: [Option<u8>; 2],
}

impl InputLog {
    pub fn push(&mut self, kind: u8, port: u8, value: u8, x: u32, master: u64) {
        self.bytes.extend_from_slice(&[kind, port, value, 0]);
        self.bytes.extend_from_slice(&x.to_le_bytes());
        self.bytes.extend_from_slice(&master.to_le_bytes());
    }

    /// A pad set: logged only when it changes the pad.
    pub fn pad(&mut self, port: usize, value: u8, master: u64) {
        if self.pads[port] != Some(value) {
            self.pads[port] = Some(value);
            self.push(PAD, port as u8, value, 0, master);
        }
    }
}

/// One event of a log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: u8,
    pub port: u8,
    pub value: u8,
    pub x: u32,
    pub master: u64,
}

/// A log's events, refused whole if it is not one.
pub fn events(bytes: &[u8]) -> Result<Vec<Event>, String> {
    if !bytes.len().is_multiple_of(EVENT_BYTES) {
        return Err(format!("an input log is 16-byte events; this one is {} bytes", bytes.len()));
    }
    let mut out = Vec::with_capacity(bytes.len() / EVENT_BYTES);
    let mut last = 0u64;
    for (i, e) in bytes.chunks_exact(EVENT_BYTES).enumerate() {
        let ev = Event {
            kind: e[0],
            port: e[1],
            value: e[2],
            x: u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
            master: u64::from_le_bytes(e[8..16].try_into().unwrap()),
        };
        if !(PAD..=END).contains(&ev.kind) {
            return Err(format!("event {i} is of kind {}, which no log has", ev.kind));
        }
        if ev.master < last {
            return Err(format!("event {i} is at master half-step {} but the one before it was at {last}", ev.master));
        }
        last = ev.master;
        out.push(ev);
    }
    Ok(out)
}

/// The trace, as a console appends to it.
#[derive(Default)]
pub struct Trace {
    pub bytes: Vec<u8>,
}

impl Trace {
    #[inline]
    pub fn cycle(&mut self, ab: u16, db: u8, flags: u8, prg: Option<usize>) {
        let p = prg.map_or(0u32, |o| o as u32 + 1);
        let [a0, a1] = ab.to_le_bytes();
        let [p0, p1, p2, p3] = p.to_le_bytes();
        self.bytes.extend_from_slice(&[a0, a1, db, KIND_CYCLE | flags, p0, p1, p2, p3]);
    }

    #[inline]
    pub fn regs(&mut self, (a, x, y, s, p): (u8, u8, u8, u8, u8), line: u16) {
        let [l0, l1] = line.to_le_bytes();
        self.bytes.extend_from_slice(&[a, x, y, KIND_REGS, s, p, l0, l1]);
    }

    pub fn input(&mut self, port: u8, value: u8, frame: u32) {
        let [f0, f1, f2, f3] = frame.to_le_bytes();
        self.bytes.extend_from_slice(&[value, port, 0, KIND_INPUT, f0, f1, f2, f3]);
    }

    pub fn frame(&mut self, digest: u32, index: u32) {
        let [d0, d1, d2, _] = digest.to_le_bytes();
        let [f0, f1, f2, f3] = index.to_le_bytes();
        self.bytes.extend_from_slice(&[d0, d1, d2, KIND_FRAME, f0, f1, f2, f3]);
    }
}

/// A log played back into a console that has just powered on (and had
/// its battery RAM, if the recording started with one, put back). Each
/// `run` goes on for some pictures; the console's `trace`, if it has one,
/// fills as it goes.
pub struct Replay {
    events: Vec<Event>,
    next: usize,
    frames_checked: u64,
}

/// Where a replay stands after a `run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Played {
    /// The pictures asked for were played and each matched the log.
    More,
    /// The log's END was reached.
    Ended,
}

impl Replay {
    pub fn new(log: &[u8]) -> Result<Replay, String> {
        let events = events(log)?;
        if events.last().map(|e| e.kind) != Some(END) {
            return Err("the input log has no END: the recording was not stopped, or the file was cut short".into());
        }
        Ok(Replay { events, next: 0, frames_checked: 0 })
    }

    /// The pictures the log records.
    pub fn frames(&self) -> u64 {
        self.events.iter().filter(|e| e.kind == FRAME).count() as u64
    }

    /// Pictures checked against the log so far.
    pub fn frames_checked(&self) -> u64 {
        self.frames_checked
    }

    /// Play until `pictures` more have completed or the log ends.
    pub fn run(&mut self, c: &mut crate::Console, pictures: u64) -> Result<Played, String> {
        let target = self.frames_checked + pictures;
        while self.frames_checked < target {
            let Some(&e) = self.events.get(self.next) else {
                return Ok(Played::Ended);
            };
            if c.master > e.master {
                return Err(format!("the replay passed master half-step {} without reaching the log's event there (kind {})", e.master, e.kind));
            }
            if c.master < e.master {
                c.master_half_step();
                continue;
            }
            // c.master == e.master: this event is due.
            self.next += 1;
            match e.kind {
                PAD => c.set_pad(e.port as usize, nes_glue::controller::Buttons::from_byte(e.value)),
                RESET => c.reset_button(e.x as u64),
                FRAME => {
                    let got = c.last_frame_digest.ok_or_else(|| format!("the log has a picture at master half-step {} and the replay completed none there", e.master))?;
                    if c.last_frame_master != Some(e.master) || got != e.x {
                        return Err(format!(
                            "the replay left the recording at picture {}: the log has digest {:08x} at master half-step {}, the replay {:08x} at {:?}",
                            self.frames_checked, e.x, e.master, got, c.last_frame_master
                        ));
                    }
                    self.frames_checked += 1;
                }
                _ => return Ok(Played::Ended),
            }
        }
        Ok(Played::More)
    }
}
