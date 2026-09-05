//! N5 step 1: the plumbing. A cartridge built in the test carries a
//! program that paints a picture through the PPU's registers and counts
//! NMIs in RAM; the console runs it on both rungs through the board.
//! What must hold: the frames come out one per PPU frame, the picture is
//! the one a standalone PPU rung produces from the same VRAM and register
//! state (the console adds nothing and loses nothing between the two
//! chips), the NMI counter in RAM advances once per frame, and the
//! controller path returns a pressed button through OUT0 and $4016.
//!
//! No die data or goldens are read here: the two rungs' own gates hold
//! them to their chips; this holds the wiring.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{Alignment, Console};
use nes_glue::controller::Buttons;
use v2c02_fast::Fast;

use nes_console::testrom::{chr, program};

fn console() -> Console {
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    Console::new(Box::new(cart), None, Alignment::default())
}

#[test]
fn frames_come_out_one_per_ppu_frame_and_the_nmi_counts_them() {
    let mut c = console();
    c.run_frames(6);
    assert_eq!(c.frames.len(), 6);
    // 89,342 dots a frame, eight master half-steps each.
    let dots_per_frame = (nes_bus::LINES * nes_bus::DOTS_PER_LINE) as u64;
    assert_eq!(c.dots, 6 * dots_per_frame, "one frame is one full traversal of the table");
    assert_eq!(c.master, (c.dots - 1) * 8 + c.alignment.ppu_phase as u64 + 1, "the master counter is eight per dot (master {} dots {})", c.master, c.dots);
    let nmis = c.board.borrow().wram.read(0x0000);
    // NMI on from the program's setup (a few thousand cycles in), so
    // every frame after the first has one; the exact count is measured
    // and must be within one of the frames seen.
    assert!((4..=6).contains(&nmis), "NMIs counted in RAM: {nmis} over 6 frames");
    eprintln!("6 frames, {} master half-steps, {} CPU half-cycles, {nmis} NMIs counted by the program", c.master, c.cpu_half_cycles);
}

#[test]
fn the_picture_is_the_ppu_rungs_own_from_the_same_state() {
    let mut c = console();
    c.run_frames(6);
    let ours = c.frames.last().unwrap().clone();
    // The standalone rung on the same CHR and CIRAM, with the same
    // register state, must draw the same picture: the console adds
    // nothing between the chips.
    let (ctrl, mask, t) = {
        let b = c.board.borrow();
        (b.ppu.ctrl, b.ppu.mask, b.ppu.t)
    };
    let mut alone = Fast::on_bus(Box::new(nes_console::board::PpuBus(c.board.borrow().cart.clone())));
    for (i, colour) in [0x0fu8, 0x30, 0x16, 0x2a].iter().enumerate() {
        alone.write(6, 0x3f);
        alone.write(6, i as u8);
        alone.write(7, *colour);
    }
    alone.write(0, ctrl);
    alone.write(1, mask);
    alone.write(6, (t >> 8) as u8 & 0x3f);
    alone.write(6, t as u8);
    alone.write(5, 0);
    alone.write(5, 0);
    // t's coarse position comes from the last $2006 pair the program
    // wrote; the frame starts from the pre-render line's copy of t.
    let theirs = alone.frame();
    let mut differ = 0;
    for row in 0..nes_bus::ACTIVE_ROWS {
        for d in 1..=nes_bus::ACTIVE_DOTS {
            if ours.at(row, d) != theirs.at(row, d) {
                differ += 1;
            }
        }
    }
    assert_eq!(differ, 0, "{differ} dots differ between the console's frame and the rung's own");
    // And the picture is what the program painted: rows 32..63 (tiles
    // 128..255 of the nametable: rows 4..7) carry colour 1, the rest 0.
    let c1 = ours.at(40, 100).0;
    let c0 = ours.at(10, 100).0;
    let palette = c.board.borrow().ppu.palette;
    let ciram: Vec<u8> = {
        let b = c.board.borrow();
        let cart = b.cart.borrow();
        [0x000u16, 0x07f, 0x080, 0x0ff, 0x100, 0x2bf, 0x2c0, 0x3bf, 0x3c0, 0x3c8, 0x3cf, 0x3ff].iter().map(|&a| cart.ciram.read(a)).collect()
    };
    let (writes, reads) = { let b = c.board.borrow(); (b.writes, b.reads) };
    assert_eq!(c1, 0x30, "tile 1's colour where the program wrote tile 1 (palette {palette:02x?}; ciram samples {ciram:02x?}; {reads} reads {writes} writes; ppu ctrl {ctrl:02x} mask {mask:02x} t {t:04x})");
    assert_eq!(c0, 0x0f, "the backdrop elsewhere");
}

#[test]
fn a_pressed_button_reaches_the_program_through_out0_and_4016() {
    let mut c = console();
    c.run_frames(3);
    c.set_pad(0, Buttons { a: true, ..Buttons::default() });
    c.board.borrow_mut().wram.write(0x0001, 1);
    c.run_frames(2);
    let read = c.board.borrow().wram.read(0x0002);
    assert_eq!(read & 1, 1, "A pressed reads as D0 = 1 through U9 (\\$02 = {read:02x})");
    assert_eq!(read & 0xe0, 0x40, "D5..D7 are the open bus, the $40 of the address just read");
}
