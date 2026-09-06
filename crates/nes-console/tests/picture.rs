//! N6 step 1: the picture. Two things must hold. A console frame through
//! `Picture` is what the standalone PPU rung's frame from the same world
//! is through the same picture, every decoded sample (the seam adds
//! nothing, now past the encoder). And the subcarrier phase the picture
//! carries across the console's real parity sequence is what ntsc-grid's
//! arithmetic gives that sequence: the odd frame's short line moves the
//! phase, so the console must pass its parity, not just its dots.
//! `MUTATE=1` pushes every frame as Even and the second must go red.
//!
//! No die data or goldens are read here.

use nes_bus::cart::{Mirroring, Nrom};
use nes_bus::{DotFrame, FrameParity};
use nes_console::testrom::{chr, program};
use nes_console::{Alignment, Console, Picture};
use ntsc_grid::Geometry;
use v2c02_fast::Fast;

fn console() -> Console {
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    Console::new(Box::new(cart), None, Alignment::default())
}

#[test]
fn a_console_frame_through_the_picture_is_the_rungs_own_through_it() {
    let mut c = console();
    c.run_frames(6);
    let ours = c.frames.last().unwrap().clone();
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
    // The rung's frames alternate parity with rendering on; take the
    // one with the console frame's parity so the two are the same
    // frame in every field.
    let mut theirs = alone.frame();
    if theirs.parity != ours.parity {
        theirs = alone.frame();
    }
    assert_eq!(theirs.parity, ours.parity, "the standalone rung reaches the console frame's parity");
    let a = Picture::decode_only().push(&ours).decoded;
    let b = Picture::decode_only().push(&theirs).decoded;
    assert_eq!((a.width, a.height), (2048, 240));
    let differ = a.data.iter().zip(&b.data).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{differ} of {} decoded components differ between the console's frame and the rung's own through the same picture", a.data.len());
    // And the picture saw the program's painting: the rows it painted
    // colour $30 (rows 32..63) decode brighter than the backdrop rows.
    let luma = |row: usize| -> f32 {
        let o = row * a.width;
        (o..o + a.width).map(|i| a.data[i * 3..i * 3 + 3].iter().sum::<f32>()).sum::<f32>() / a.width as f32
    };
    assert!(luma(40) > luma(10) + 0.5, "the painted rows decode brighter than the backdrop: {} vs {}", luma(40), luma(10));
    // The CRT stages run on it and put something on the screen.
    let shown = Picture::new().push(&ours);
    let d = shown.displayed.unwrap();
    assert_eq!((d.width, d.height), (768, 720));
    assert!(d.data.iter().any(|&v| v > 0.0), "the displayed frame is not black");
    eprintln!("console frame {:?} through Rung C: {} components equal to the rung's own; displayed {}x{}", ours.parity, a.data.len(), d.width, d.height);
}

#[test]
fn the_phase_carried_across_the_consoles_parity_sequence_is_the_grids() {
    let mutate = std::env::var("MUTATE").is_ok_and(|v| v == "1");
    let mut c = console();
    c.run_frames(12);
    let parities: Vec<FrameParity> = c.frames.iter().map(|f| f.parity).collect();
    let short = parities.iter().filter(|&&p| p == FrameParity::OddShort).count();
    assert!(short >= 3, "the sequence carries short frames, which are what move the phase: {parities:?}");
    let mut picture = Picture::decode_only();
    for f in &c.frames {
        if mutate {
            let mut e = DotFrame { parity: FrameParity::Even, colour: f.colour.clone(), emphasis: f.emphasis.clone() };
            // An Even frame is a full frame: the short frame's last line
            // must be filled out to push it as one.
            e.colour.resize(nes_bus::LINES * nes_bus::DOTS_PER_LINE, 0x0f);
            e.emphasis.resize(nes_bus::LINES * nes_bus::DOTS_PER_LINE, 0);
            picture.push(&e);
        } else {
            picture.push(f);
        }
    }
    let grid = Geometry::nes();
    let expected = parities.iter().fold(ntsc_grid::Phase::new(0), |o, &p| grid.next_origin(o, p));
    assert_eq!(picture.origin(), expected, "the phase after {parities:?}: the picture carries {}, the grid's arithmetic gives {}", picture.origin().get(), expected.get());
    eprintln!("12 frames ({short} short): phase {} after the sequence, as the grid's arithmetic gives", expected.get());
}
