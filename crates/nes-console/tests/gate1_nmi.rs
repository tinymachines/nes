//! Gate 1, the CPU side: an NMI from the PPU landing around a BRK,
//! replayed through the console against the switch-level 6502.
//!
//! The console runs a cartridge whose program enables NMI at once, burns
//! a known number of cycles, and executes a BRK; the PPU's first vblank
//! asserts /NMI at a master half-step the program cannot move, so the
//! BRK is placed by the sled's length at a chosen half-cycle offset from
//! that edge (a scout run measures where the edge lands). The console
//! logs every CPU half-cycle's inputs and pin frame. Rung 0 (the
//! switch-level 6502, `v6502-sim`) then runs the same bytes with the
//! same NMI schedule, half-cycle for half-cycle, and the two pin traces
//! must agree through the BRK and the interrupt sequence: the vector
//! taken, the pushes, the timing. That is "the NMI-during-BRK case
//! replays through the console with the same half-cycle positions",
//! for eight offsets a cycle apart spanning the BRK's fetch, so the edge
//! falls in every cycle of the instruction and the ones before it.
//!
//! What is compared: address, R/W and sync on every half-cycle, and the
//! data byte except during a write's phi1 (where the two cores' bus
//! shows different junk and nothing is serviced: the 2A03 repository's
//! lockstep names the class). The stack page is compared with the two
//! cores' power-on stack pointers' difference removed, derived from the
//! first push each makes, the way that lockstep derives it. No number
//! here is typed from either core: the NMI's half-cycle is measured off
//! the console, the stack offset off the traces.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{Alignment, Console, CpuStep};
use v6502_pins::{Load, PinEngine, PinFrame};
use v6502_sim::pins::rung0;

const SLED: u16 = 0x8011;
const NMI_VEC: u16 = 0x8180;
const IRQ_VEC: u16 = 0x81c0;

/// SEI; CLD; LDA #$80; STA $2000; LDX #21; outer: LDY #0; inner: DEY;
/// BNE inner; DEX; BNE outer; then the sled, a BRK, NOPs, and a spin.
fn program(sled: &[u8]) -> Vec<u8> {
    let mut prg = vec![0u8; 0x8000];
    let mut p: Vec<u8> = vec![0x78, 0xd8, 0xa9, 0x80, 0x8d, 0x00, 0x20, 0xa2, 0x15, 0xa0, 0x00, 0x88, 0xd0, 0xfd, 0xca, 0xd0, 0xf8];
    assert_eq!(p.len(), (SLED - 0x8000) as usize);
    p.extend_from_slice(sled);
    p.extend([0x00, 0xea, 0xea, 0xea, 0xea, 0xea, 0xea]); // BRK (its padding byte is the first NOP), NOPs
    let here = 0x8000 + p.len() as u16;
    p.extend([0x4c, here as u8, (here >> 8) as u8]);
    prg[..p.len()].copy_from_slice(&p);
    // The handlers touch nothing but the stack (a RAM byte's power-on
    // fill is the console's and not rung 0's); their lengths tell them
    // apart, and the vector read says which was taken anyway.
    prg[(NMI_VEC - 0x8000) as usize..][..2].copy_from_slice(&[0xea, 0x40]); // NOP; RTI
    prg[(IRQ_VEC - 0x8000) as usize..][..3].copy_from_slice(&[0xea, 0xea, 0x40]); // NOP; NOP; RTI
    prg[0x7ffa..].copy_from_slice(&[NMI_VEC as u8, (NMI_VEC >> 8) as u8, 0x00, 0x80, IRQ_VEC as u8, (IRQ_VEC >> 8) as u8]);
    prg
}

/// A sled of `half_cycles` (even, at least four): NOPs, with one STA zp
/// (three cycles, no flags, no read of a RAM whose power-on fill the
/// two worlds do not share) where the remainder needs it.
fn sled(half_cycles: u64) -> Vec<u8> {
    assert!(half_cycles >= 4 && half_cycles.is_multiple_of(2), "sled of {half_cycles} half-cycles");
    let mut s = Vec::new();
    let mut left = half_cycles;
    if left % 4 == 2 {
        s.extend([0x85, 0x00]);
        left -= 6;
    }
    s.extend(std::iter::repeat_n(0xea, (left / 4) as usize));
    s
}

fn console_trace(prg: Vec<u8>, half_cycles: usize) -> Vec<CpuStep> {
    let cart = Nrom::new(prg, vec![0; 0x2000], Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.cpu_trace = Some(Vec::new());
    while c.cpu_trace.as_ref().unwrap().len() < half_cycles {
        c.master_half_step();
    }
    c.cpu_trace.take().unwrap()
}

/// The console's RAM as it powers on (the SRAM's fill, mirrored across
/// its window), so rung 0's unwritten bytes read what the console's do.
fn ram_image() -> Vec<u8> {
    let cart = Nrom::new(vec![0; 0x8000], vec![0; 0x2000], Mirroring::Vertical).unwrap();
    let c = Console::new(Box::new(cart), None, Alignment::default());
    let b = c.board.borrow();
    (0..0x2000u16).map(|a| b.wram.read(a & 0x7ff)).collect()
}

/// Rung 0 on the same bytes and the same RAM image, /NMI low from the
/// step the console saw it.
fn rung0_trace(prg: &[u8], nmi_low_from: usize, steps: usize) -> Vec<PinFrame> {
    let loads = [Load { org: 0x0000, bytes: ram_image() }, Load { org: 0x8000, bytes: prg.to_vec() }];
    let mut cpu = rung0(&loads, 0x8000);
    cpu.power_cycle();
    let mut out = Vec::with_capacity(steps);
    for k in 0..steps {
        cpu.set_inputs(true, true, k < nmi_low_from, true, false);
        cpu.half_step();
        out.push(cpu.pins());
    }
    out
}

fn first_fetch_at(frames: impl Iterator<Item = PinFrame>, addr: u16) -> usize {
    frames.enumerate().find(|(_, f)| f.sync && f.ab == addr).map(|(i, _)| i).expect("the fetch is in the trace")
}

#[test]
fn an_nmi_from_the_ppu_around_a_brk_replays_on_rung_0_at_every_offset() {
    // The scout: where the edge lands, in CPU half-cycles, against the
    // sled's first fetch. A long sled and no BRK in reach.
    let scout = console_trace(program(&sled(2000)), 60_000);
    let k_nmi = scout.iter().position(|s| s.nmi).expect("the PPU asserted NMI inside the run");
    let k_sled = first_fetch_at(scout.iter().map(|s| s.frame), SLED);
    assert!(k_nmi > k_sled + 4, "the NMI ({k_nmi}) must fall after the sled starts ({k_sled})");
    eprintln!("gate 1: /NMI falls at CPU half-cycle {k_nmi}, the sled's first fetch at {k_sled}");

    // The edge's parity against the cycle grid is the PPU's to choose:
    // offsets step by whole cycles from the one the grid allows.
    let parity = (k_nmi as i64 - k_sled as i64).rem_euclid(2);
    let mut checked = 0usize;
    for d in (-12i64 - parity..=2 - parity).step_by(2) {
        let len = (k_nmi as i64 - k_sled as i64 + d) as u64;
        let prg = program(&sled(len));
        let steps = k_nmi + 200;
        let con = console_trace(prg.clone(), steps);
        let k_brk = first_fetch_at(con.iter().map(|s| s.frame), SLED + sled(len).len() as u16);
        assert_eq!(k_brk as i64, k_nmi as i64 + d, "the BRK's fetch sits at the chosen offset");
        let k_nmi_here = con.iter().position(|s| s.nmi).unwrap();
        assert_eq!(k_nmi_here, k_nmi, "the sled's length cannot move the PPU's edge");

        // Align both traces on the reset vector's first fetch.
        let a_c = first_fetch_at(con.iter().map(|s| s.frame), 0x8000);
        let r0 = rung0_trace(&prg, 0, 64);
        let a_0 = first_fetch_at(r0.iter().copied(), 0x8000);
        // The console applies the low level before step k_nmi; rung 0's
        // step j corresponds to console step j - a_0 + a_c.
        let nmi_from_0 = k_nmi + a_0 - a_c;
        let r0 = rung0_trace(&prg, nmi_from_0, steps + a_0 - a_c + 1);

        // The stack offset, from the first push each makes (the BRK's).
        fn push(mut fs: impl Iterator<Item = PinFrame>) -> u8 {
            fs.find(|f| !f.rw && f.ab >> 8 == 1).map(|f| f.ab as u8).expect("a push")
        }
        let s_c = push(con.iter().map(|s| s.frame));
        let s_0 = push(r0.iter().copied());
        let offset = s_c.wrapping_sub(s_0);

        let from = k_brk - 30;
        let to = k_brk + 90;
        for k in from..to {
            let c = con[k].frame;
            let o = r0[k - a_c + a_0];
            let ab_c = if c.ab >> 8 == 1 { 0x0100 | (c.ab as u8).wrapping_sub(offset) as u16 } else { c.ab };
            let same = ab_c == o.ab && c.rw == o.rw && c.sync == o.sync && (c.db == o.db || (!c.rw && !c.clk0));
            assert!(
                same,
                "offset {d}: console and rung 0 disagree at half-cycle {k} ({} after the BRK fetch):\n  console {}\n  rung 0  {}",
                k as i64 - k_brk as i64,
                v6502_pins::line(&c),
                v6502_pins::line(&o)
            );
            checked += 1;
        }
        // And the interrupt was taken through the NMI vector on both.
        assert!(con[from..to].iter().any(|s| s.frame.ab == 0xfffa && s.frame.rw), "offset {d}: the console read the NMI vector");
        eprintln!("gate 1: BRK at NMI{d:+}: {} half-cycles agree", to - from);
    }
    eprintln!("gate 1: {checked} half-cycles compared over eight offsets, stack offset removed");
    assert!(checked >= 8 * 120);
}
