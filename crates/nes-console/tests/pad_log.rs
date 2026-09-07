//! The controller port's poll log, the model's side of the bench's B0
//! (tinymachines/nes-bench): one entry per latch with the byte the
//! register loaded and the reads the poll took. On the polling
//! cartridge every poll is eight reads and carries the byte set; with
//! its DMC loop on, some polls take nine, which is the die's double
//! clock (2a03's joy-clock-probe, held by its tests/joypad.rs) reaching
//! the console's board: the count and the latches it lands on are the
//! model's prediction for the part. MUTATE_HELD=1 (rung 3 keeps a held
//! read's first byte and asks once) must go red on the nines.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, pad_program};
use nes_console::{Alignment, Console};
use nes_glue::controller::Buttons;

fn polls(dmc: bool, pad: u8, frames: usize) -> Vec<(u8, u32)> {
    polls_scripted(dmc, pad, frames, Vec::new())
}

fn polls_scripted(dmc: bool, pad: u8, frames: usize, schedule: Vec<(u64, u8)>) -> Vec<(u8, u32)> {
    let cart = Nrom::new(pad_program(dmc), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.board.borrow_mut().pads[0].log_polls = true;
    c.board.borrow_mut().pads[0].schedule = schedule;
    c.set_pad(0, Buttons::from_byte(pad));
    c.run_frames(frames);
    let b = c.board.borrow();
    let p = &b.pads[0].polls;
    // Latch i closes poll i-1: the byte it loaded, the reads after it.
    (1..p.len()).map(|i| (p[i - 1].0, p[i].1)).collect()
}

#[test]
fn every_poll_is_eight_reads_of_the_byte_set() {
    let p = polls(false, 0xa5, 120);
    assert!(p.len() >= 100, "one poll a frame: {}", p.len());
    assert!(p.iter().all(|&(b, c)| b == 0xa5 && c == 8), "every poll eight reads of $a5: {:?}", p.iter().filter(|&&(b, c)| b != 0xa5 || c != 8).take(4).collect::<Vec<_>>());
}

#[test]
fn a_dmc_loop_makes_some_polls_nine_reads_and_none_other_than_eight_or_nine() {
    let p = polls(true, 0xa5, 600);
    let nines: Vec<usize> = p.iter().enumerate().filter(|(_, &(_, c))| c == 9).map(|(i, _)| i).collect();
    let other: Vec<(usize, u32)> = p.iter().enumerate().filter(|(_, &(_, c))| c != 8 && c != 9).map(|(i, &(_, c))| (i, c)).collect();
    eprintln!("{} polls, {} of nine reads at latches {:?}", p.len(), nines.len(), nines);
    assert!(other.is_empty(), "a poll is eight reads or nine: {other:?}");
    assert!(nines.len() >= 10, "the DMC at its fastest rate lands on the poll's reads often over 600 frames (MUTATE_HELD asks once and finds none): {}", nines.len());
    // The model's prediction for the part, recorded so a change is seen.
    assert_eq!((p.len(), nines.len()), (596, 21), "polls and nines over 600 frames: {} and {}", p.len(), nines.len());
}

#[test]
fn a_scheduled_byte_holds_from_exactly_its_latch() {
    // The bench script's AT: from latch n on, the register holds hh. On
    // the bridge the byte is written after latch n-1; here it is applied
    // at the strobe's rise before latch n. Either way poll n-1 shows the
    // old byte and poll n the new, which is what compare-logs.py holds
    // the two logs to.
    let p = polls_scripted(false, 0x00, 120, vec![(30, 0x08), (33, 0x00), (50, 0x81)]);
    let bytes: Vec<u8> = p.iter().map(|&(b, _)| b).collect();
    assert_eq!(&bytes[28..35], &[0x00, 0x00, 0x08, 0x08, 0x08, 0x00, 0x00], "08 from latch 30, 00 from 33: {:?}", &bytes[28..35]);
    assert_eq!(&bytes[49..52], &[0x00, 0x81, 0x81], "81 from latch 50");
    assert!(bytes[52..].iter().all(|&b| b == 0x81), "and held after");
}
