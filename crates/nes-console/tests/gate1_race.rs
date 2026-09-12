//! Gate 1, the PPU side: the $2002 read race through the console, at the
//! half-step positions the console's own programs produce, held to the
//! table measured on the switch-level 2C02 with the console's access
//! shape (2c02, `race-shape-probe`; the fast rung holds the same table
//! in its `tests/race.rs`). This is the plumbing under test: that a
//! read the CPU starts at master half-step m reaches the PPU stamped
//! at the position within its dot that the alignment says, and comes
//! back with what the chip gives a read starting there.
//!
//! Against the first half-step of the dot in which the flag sets (vpos
//! 241, hpos 1): a read starting eight or more before misses (reads 0,
//! the flag sets, the NMI is taken), one starting one to seven before
//! suppresses (reads 0, the flag never sets, no NMI), one starting on
//! the dot or later consumes (reads 1, NMI taken: the CPU's phi1 sample
//! saw /INT low before the read cleared it). Against the clear's dot
//! (vpos 261, hpos 1): a read starting on it or later reads 0, one
//! before reads 1.
//!
//! A CPU read begins on a phi1, which recurs every twenty-four master
//! half-steps, so under one alignment every read starts on one residue
//! of twenty-four, and one of a dot's eight half-steps; the twenty-four
//! alignments (the CPU's first phi1 against the dot, cpu_phase 0 to 23
//! with ppu_phase 3) are run and the offsets seen must between them
//! cover every half-step of the window. A polling program walks the
//! set; a once-a-frame read a fixed delay after the NMI walks the
//! clear, where only the alignment can move it.

use nes_bus::cart::{Mirroring, Nrom};
use nes_bus::{DOTS_PER_LINE, LINES};
use nes_console::{Alignment, Console, CpuStep};
use v6502_pins::PinEngine;
use std::collections::BTreeSet;

const SUPPRESS_FROM: i64 = -7;

fn cart(prg: Vec<u8>) -> Nrom {
    Nrom::new(prg, vec![0; 0x2000], Mirroring::Vertical).unwrap()
}

fn vectors(prg: &mut [u8], nmi: u16) {
    prg[0x7ffa..].copy_from_slice(&[nmi as u8, (nmi >> 8) as u8, 0x00, 0x80, 0x00, 0x81]);
}

/// The master half-step at which the PPU steps dot (line, dot) of frame k,
/// rendering off (every frame the full 89,342 dots, the first beginning
/// at line 261 dot 0).
fn dot_master(a: Alignment, k: u64, line: usize, dot: usize) -> u64 {
    let index = if line == LINES - 1 { dot } else { DOTS_PER_LINE + line * DOTS_PER_LINE + dot } as u64;
    a.ppu_phase as u64 + 8 * (index + k * (LINES * DOTS_PER_LINE) as u64)
}

/// Runs the program for `frames`, keeping only the CPU half-cycles that
/// read $2002 or the NMI vector (a whole trace of a hundred frames would
/// be hundreds of megabytes).
fn trace(prg: Vec<u8>, a: Alignment, frames: usize) -> Vec<CpuStep> {
    let mut c = Console::new(Box::new(cart(prg)), None, a);
    let mut kept = Vec::new();
    let mut seen = 0;
    while c.frames.len() < frames {
        c.master_half_step();
        if c.cpu_half_cycles != seen {
            seen = c.cpu_half_cycles;
            let f = c.cpu.pins();
            if f.rw && !f.clk0 && (f.ab == 0x2002 || f.ab == 0xfffa) {
                kept.push(CpuStep { master: c.master - 1, nmi: false, irq: false, frame: f, core_rdy: f.rdy });
            }
        }
    }
    assert!(c.frames.iter().all(|f| f.parity != nes_bus::FrameParity::OddShort), "rendering is off: every frame full length");
    kept
}

/// The phi1 frames of $2002 reads: (master, bit 7).
fn reads_2002(t: &[CpuStep]) -> Vec<(u64, bool)> {
    t.iter().filter(|s| s.frame.ab == 0x2002).map(|s| (s.master, s.frame.db & 0x80 != 0)).collect()
}

/// Masters of the NMI vector's first read (one per NMI taken).
fn nmis(t: &[CpuStep]) -> Vec<u64> {
    t.iter().filter(|s| s.frame.ab == 0xfffa).map(|s| s.master).collect()
}

#[test]
fn a_polling_read_meets_the_set_where_the_chip_says() {
    // SEI; CLD; LDA #$80; STA $2000; LDX #0; loop: LDA $2002; STA $00;
    // INX; TXA; LSR; BCS skip; NOP; skip: JMP loop. Nineteen and twenty
    // cycles alternately, so the reads take both cycle parities, and the
    // set's drift against the CPU grid (sixteen half-steps a frame)
    // walks them across the dot. NMI: INC $10; RTI.
    let mut prg = vec![0u8; 0x8000];
    let p: Vec<u8> = vec![
        0x78, 0xd8, 0xa9, 0x80, 0x8d, 0x00, 0x20, 0xa2, 0x00,
        0xad, 0x02, 0x20, 0x85, 0x00, 0xe8, 0x8a, 0x4a, 0xb0, 0x01, 0xea, 0x4c, 0x09, 0x80,
    ];
    prg[..p.len()].copy_from_slice(&p);
    prg[0x0180..0x0183].copy_from_slice(&[0xe6, 0x10, 0x40]);
    prg[0x0100..0x0101].copy_from_slice(&[0x40]);
    vectors(&mut prg, 0x8180);

    let mut seen: BTreeSet<i64> = BTreeSet::new();
    let mut checked = 0;
    for cpu_phase in 0..24u8 {
        let a = Alignment { cpu_phase, ppu_phase: 3 };
        let frames = 40;
        let t = trace(prg.clone(), a, frames);
        let reads = reads_2002(&t);
        let nmi_at = nmis(&t);
        for k in 1..frames as u64 - 1 {
            let set = dot_master(a, k, 241, 1) as i64;
            let next_set = dot_master(a, k + 1, 241, 1) as i64;
            // The reads of this frame around the set, in order.
            let near: Vec<(i64, bool)> = reads.iter().map(|&(m, b)| (m as i64 - set, b)).filter(|(o, _)| *o > -3000 && *o < next_set - set - 3000).collect();
            let suppressed = near.iter().any(|(o, _)| (SUPPRESS_FROM..0).contains(o));
            let nmi_taken = nmi_at.iter().any(|&m| (m as i64) > set && (m as i64) < next_set);
            assert_eq!(!suppressed, nmi_taken, "cpu_phase {cpu_phase}, frame {k}: NMI taken iff no read started in the suppress window (offsets {:?})",
                near.iter().filter(|(o, _)| o.abs() < 40).collect::<Vec<_>>());
            for (i, &(o, bit7)) in near.iter().enumerate() {
                let first_after = o >= 0 && (i == 0 || near[i - 1].0 < 0);
                if o < 0 {
                    assert!(!bit7, "cpu_phase {cpu_phase}, frame {k}: a read starting {o} before the set read the flag set");
                } else if first_after {
                    assert_eq!(bit7, !suppressed, "cpu_phase {cpu_phase}, frame {k}: the first read after the set ({o}) reads the flag unless a read at {:?} suppressed it",
                        near.iter().find(|(x, _)| (SUPPRESS_FROM..0).contains(x)));
                }
                if o.abs() < 40 {
                    seen.insert(o.rem_euclid(8) + if o < 0 { -8 } else { 0 });
                    checked += 1;
                }
            }
        }
    }
    // Every half-step of the dot before the set, and of the set's dot,
    // was started on by some read under some alignment.
    let want: BTreeSet<i64> = (-8..8).collect();
    assert_eq!(seen, want, "the reads did not cover every half-step position around the set");
    eprintln!("gate 1 (race, set): {checked} reads within five dots of the set, every position covered");
}

#[test]
fn a_delayed_read_meets_the_clear_where_the_chip_says() {
    // NMI: INC $10; RTI. Main: SEI; CLD; the delays into $12 and $13;
    // LDA #$80; STA $2000; wait: LDA $10; CMP $11; BEQ wait; STA $11;
    // LDY #7; o: LDX #55; i: DEX; BNE i; DEY; BNE o (the coarse delay,
    // short of the clear); LDX $12; d: DEX; BNE d (five cycles a
    // count); LDA $13; CMP #1; BCS +0; CMP #2; BCS +0; CMP #3; BCS +0;
    // CMP #4; BCS +0 (a cycle a count, a taken branch each); LDA $2002;
    // STA $00; JMP wait. A scout with the delays at their least says
    // where the read lands; the counts then put it within half a cycle
    // of the clear, and the alignment says which half-step.
    let build = |x5: u8, x1: u8| {
        let mut prg = vec![0u8; 0x8000];
        let p: Vec<u8> = vec![
            0x78, 0xd8, 0xa9, x5, 0x85, 0x12, 0xa9, x1, 0x85, 0x13, 0xa9, 0x80, 0x8d, 0x00, 0x20,
            0xa5, 0x10, 0xc5, 0x11, 0xf0, 0xfa, 0x85, 0x11, // wait ($800f)
            0xa0, 0x07, 0xa2, 0x37, 0xca, 0xd0, 0xfd, 0x88, 0xd0, 0xf8, // coarse
            0xa6, 0x12, 0xca, 0xd0, 0xfd, // five a count
            0xa5, 0x13, 0xc9, 0x01, 0xb0, 0x00, 0xc9, 0x02, 0xb0, 0x00, 0xc9, 0x03, 0xb0, 0x00, 0xc9, 0x04, 0xb0, 0x00, // one a count
            0xad, 0x02, 0x20, 0x85, 0x00, 0x4c, 0x0f, 0x80,
        ];
        prg[..p.len()].copy_from_slice(&p);
        prg[0x0180..0x0183].copy_from_slice(&[0xe6, 0x10, 0x40]);
        prg[0x0100..0x0101].copy_from_slice(&[0x40]);
        vectors(&mut prg, 0x8180);
        prg
    };
    let mut seen: BTreeSet<i64> = BTreeSet::new();
    let mut checked = 0;
    for cpu_phase in 0..24u8 {
        let a = Alignment { cpu_phase, ppu_phase: 3 };
        // A scout with the fine delay at 1 finds where the read lands
        // against the clear; the fine delay then places it about a hundred
        // half-steps early and the drift does the rest over the frames.
        let t = trace(build(1, 0), a, 4);
        let clear2 = dot_master(a, 2, 261, 1) as i64;
        let scout = reads_2002(&t).iter().map(|&(m, _)| m as i64 - clear2).filter(|o| o.abs() < 100_000).min_by_key(|o| o.abs()).expect("a read near the clear");
        assert!(scout < 0, "cpu_phase {cpu_phase}: the coarse delay must leave the read short of the clear, not {scout:+}");
        // Whole cycles to add, so the read lands within half a cycle.
        let add = ((-scout + 12) / 24) as u8;
        let frames = 24;
        let t = trace(build(1 + add / 5, add % 5), a, frames);
        let reads = reads_2002(&t);
        for k in 2..frames as u64 - 1 {
            let clear = dot_master(a, k, 261, 1) as i64;
            let set = dot_master(a, k - 1, 241, 1) as i64;
            for &(m, bit7) in &reads {
                let o = m as i64 - clear;
                if (m as i64) <= set || o > 3000 {
                    continue;
                }
                // The flag is set from the previous set to this clear;
                // this read is the only one in between.
                assert_eq!(bit7, o < 0, "cpu_phase {cpu_phase}, frame {k}: a read starting at {o} against the clear");
                if o.abs() < 40 {
                    seen.insert(o.rem_euclid(8) + if o < 0 { -8 } else { 0 });
                    checked += 1;
                }
            }
        }
    }
    let want: BTreeSet<i64> = (-8..8).collect();
    assert_eq!(seen, want, "the reads did not cover every half-step position around the clear");
    eprintln!("gate 1 (race, clear): {checked} reads within five dots of the clear, twenty-four alignments, every position covered");
}
