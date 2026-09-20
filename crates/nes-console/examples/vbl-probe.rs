//! The vertical blank flag's own timeline, on the console.
//!
//!   cargo run --release -p nes-console --example vbl-probe -- [rom.nes] [frames]
//!
//! Steps the console a master half-step at a time and prints every edge
//! of the PPU's vblank flag: where it fell in the PPU's frame, and how
//! many CPU cycles since the last rise. The part sets it at (241, 1) and
//! clears it at (261, 1), and a rise comes 29780 or 29781 CPU cycles
//! after the last one, whichever the frame's parity and the rendering
//! bits make it.
//!
//! With no ROM the probe runs its own cartridge: turn rendering off,
//! enable nothing, and spin. That is the state blargg's `01-vbl_basics`
//! measures in and the state three of the games on this desk wait in,
//! and it is the one where the flag is the only thing moving.
//!
//! REPORT=1 keeps the last forty $2002 reads in a ring and dumps them
//! when a blargg ROM's $6000 window stops saying "running": the read it
//! decided on is the last in the dump, with the PC that made it. The
//! trigger is the window and not a write to $6000, because `set_test`
//! writes there too and would fire on every test's first line.
//!
//! RENDER=1 turns rendering on in that cartridge instead, so the two
//! can be compared without changing anything else. READS=1 prints every
//! CPU read of $2002 beside the edges, with the byte it returned and
//! where in the frame it landed: the flag being right and the read
//! being wrong are different faults and this tells them apart.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{Alignment, Console};
use v6502_pins::PinEngine;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = args.get(1).filter(|a| a.ends_with(".nes"));
    let frames: usize = args.iter().skip(1).find_map(|s| s.parse().ok()).unwrap_or(4);
    let (cart, chr_ram): (Box<dyn nes_bus::cart::Cartridge>, Option<Vec<u8>>) = match rom {
        Some(path) => {
            let bytes = std::fs::read(path).expect("rom");
            let r = nes_console::ines::parse(&bytes).expect("iNES");
            println!("{path}: mapper {}", r.mapper);
            let chr = r.chr_ram.then(|| vec![0u8; 0x2000]);
            (r.cart().expect("a board this console has"), chr)
        }
        None => {
            // LDA #mask; STA $2001; LDA #$00; STA $2000; JMP here.
            let mask = if std::env::var_os("RENDER").is_some() { 0x18u8 } else { 0x00 };
            let mut prg = vec![0xeau8; 0x8000];
            let code: &[u8] = &[0xa9, mask, 0x8d, 0x01, 0x20, 0xa9, 0x00, 0x8d, 0x00, 0x20, 0x4c, 0x0a, 0x80];
            prg[..code.len()].copy_from_slice(code);
            prg[0x7ffc..0x7ffe].copy_from_slice(&[0x00, 0x80]);
            println!("the probe's own cartridge: $2001 = {mask:02x}");
            (Box::new(Nrom::new(prg, vec![0u8; 0x2000], Mirroring::Vertical).expect("NROM")), None)
        }
    };
    let mut c = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
    let mut last = c.board.borrow().ppu.vbl;
    let mut last_rise: Option<u64> = None;
    let mut edges = 0;
    let reads = std::env::var_os("READS").is_some();
    let near_only = std::env::var_os("NEAR").is_some();
    let report = std::env::var_os("REPORT").is_some();
    let mut ring: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut pc = 0u16;
    let mut dumped = false;
    let mut started = false;
    let mut last_ring_pc: Option<u16> = None;
    let mut shown_reads = 0;
    let mut last_read: Option<u64> = None;
    for _ in 0..frames {
        let target = c.frames.len() + 1;
        while c.frames.len() < target {
            c.master_half_step();
            let f0 = c.cpu.pins();
            if f0.sync && f0.clk0 {
                pc = f0.ab;
            }
            if report && !dumped {
                // The magic goes in before the running byte does, so a
                // window that has never said $80 has not started yet:
                // triggering on it dumps the shell's own region check
                // and calls it the test.
                let done = {
                    let b = c.board.borrow();
                    b.prg_ram.as_ref().and_then(|r| {
                        if r[1..4] == [0xde, 0xb0, 0x61] && r[0] == 0x80 {
                            started = true;
                        }
                        (started && r[1..4] == [0xde, 0xb0, 0x61] && r[0] != 0x80).then_some(r[0])
                    })
                };
                if let Some(code) = done {
                    dumped = true;
                    let text: String = {
                        let b = c.board.borrow();
                        let r = b.prg_ram.as_ref().unwrap();
                        r[4..].iter().take_while(|&&x| x != 0).map(|&x| x as char).collect()
                    };
                    println!("the ROM reported {code:02x} at CPU half-cycle {}: {:?}", c.cpu_half_cycles, text.trim());
                    println!("the last {} reads of $2002:", ring.len());
                    for r in &ring {
                        println!("  {r}");
                    }
                }
            }
            if reads && shown_reads < 60 {
                let f = c.cpu.pins();
                if f.rw && f.clk0 && f.ab & 0x2007 == 0x2002 && f.ab < 0x4000 && last_read != Some(c.cpu_half_cycles) {
                    last_read = Some(c.cpu_half_cycles);
                    let (v, pos, hs) = {
                        let b = c.board.borrow();
                        (b.ppu.vbl, b.ppu.position(), b.half_steps_into_dot)
                    };
                    // The polling loops are hundreds of identical reads;
                    // what is worth printing is a read that found
                    // something, and every read in the blanking lines.
                    // NEAR=1: only the reads that land within a few
                    // dots of the set, which is where the race is.
                    let near = (pos.line == 241 && pos.dot < 12) || (pos.line == 240 && pos.dot > 328);
                    if report {
                        // A poll loop is thousands of reads from one PC
                        // and would push everything else out of the
                        // ring; each run of them is kept as one entry,
                        // the LAST of the run, which is the read that
                        // ended the wait.
                        let line = format!("read ${:04x} -> {:02x} at line {:3} dot {:3} +{hs} (CPU half-cycle {}) from PC {pc:04x}", f.ab, f.db, pos.line, pos.dot, c.cpu_half_cycles);
                        if last_ring_pc == Some(pc) {
                            ring.pop_back();
                        }
                        last_ring_pc = Some(pc);
                        ring.push_back(line);
                        if ring.len() > 40 {
                            ring.pop_front();
                        }
                    }
                    if (f.db & 0x80 != 0 && !near_only) || (near_only && near) || std::env::var_os("ALLREADS").is_some() {
                        println!("  read ${:04x} -> {:02x} at line {:3} dot {:3} +{hs} half-steps (CPU half-cycle {}), flag now {}", f.ab, f.db, pos.line, pos.dot, c.cpu_half_cycles, v as u8);
                        shown_reads += 1;
                    }
                }
            }
            let (now, pos) = {
                let b = c.board.borrow();
                (b.ppu.vbl, b.ppu.position())
            };
            if now == last {
                continue;
            }
            last = now;
            edges += 1;
            if edges > 40 {
                continue;
            }
            let hc = c.cpu_half_cycles;
            if now {
                let since = last_rise.map(|p| format!(", {} CPU cycles since the last rise", (hc - p) / 2)).unwrap_or_default();
                println!("rise at line {:3} dot {:3} (CPU half-cycle {hc}){since}", pos.line, pos.dot);
                last_rise = Some(hc);
            } else {
                println!("fall at line {:3} dot {:3} (CPU half-cycle {hc})", pos.line, pos.dot);
            }
        }
    }
    println!("{edges} edges over {frames} frames; the flag is {}", if last { "set" } else { "clear" });
}
