//! The picture a triggered capture hands back, placed from where the
//! latch fell against the vertical sync's onset (`picture_after_latch`,
//! `Console::run_to_picture_after_latch`). The rule is pinned on both
//! sides of the onset and at the frame's two ends; then the test
//! cartridge, which polls in its NMI handler at the top of the blank,
//! is run to a latch and the board's recorded position and the picture
//! chosen are checked against each other and against the old rule (the
//! next picture), which for a poll before the onset is the same
//! picture. MUTATE=1 moves the onset a dot late and the pinned edge
//! must go red.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, program};
use nes_console::{after_vsync_onset, picture_after_latch, Alignment, Console, VSYNC_ONSET};
use nes_glue::controller::Buttons;
use v2c02_fast::Position;

fn p(line: usize, dot: usize) -> Position {
    Position { line, dot }
}

#[test]
fn the_onset_is_row_244_dot_280_and_the_pre_render_line_comes_first() {
    let mutate = std::env::var("MUTATE").is_ok_and(|v| v == "1") as usize;
    let onset = p(VSYNC_ONSET.line, VSYNC_ONSET.dot + mutate);
    assert!(!after_vsync_onset(p(onset.line, onset.dot - 1)), "the dot before the onset is before it");
    assert!(after_vsync_onset(onset), "the onset itself is after (MUTATE=1 moves it a dot late: red)");
    assert!(!after_vsync_onset(p(0, 0)));
    assert!(!after_vsync_onset(p(241, 1)), "the blank's first dot, where most games poll");
    assert!(!after_vsync_onset(p(243, 340)));
    assert!(after_vsync_onset(p(251, 210)), "Super Mario Bros.'s poll");
    assert!(after_vsync_onset(p(260, 340)));
    assert!(!after_vsync_onset(p(261, 0)), "the pre-render line begins the PPU's frame");
    assert_eq!(picture_after_latch(10, p(241, 1)), 11);
    assert_eq!(picture_after_latch(10, p(251, 210)), 12);
}

#[test]
fn the_test_cartridge_polls_before_the_onset_and_gets_the_next_picture() {
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.run_frames(3);
    c.board.borrow_mut().wram.write(0x0001, 1);
    c.set_pad(0, Buttons { a: true, ..Default::default() });
    let (fell_in, pos) = c.run_to_latch(4, 60).unwrap();
    assert_eq!(c.board.borrow().latch_positions.len(), c.board.borrow().pads[0].latches as usize, "one recorded position per latch");
    assert!((241..244).contains(&pos.line), "the handler polls at the top of the blank, before the onset: {pos:?}");
    assert!(!after_vsync_onset(pos));
    let mut d = Console::new(Box::new(Nrom::new(program(), chr(), Mirroring::Vertical).unwrap()), None, Alignment::default());
    d.run_frames(3);
    d.board.borrow_mut().wram.write(0x0001, 1);
    d.set_pad(0, Buttons { a: true, ..Default::default() });
    let (target, pos2) = d.run_to_picture_after_latch(4, 60).unwrap();
    assert_eq!(pos2, pos);
    assert_eq!(target, fell_in + 1, "a poll before the onset: the next picture");
    assert_eq!(d.frames.len(), target + 1, "the console ran exactly to it");
}
