//! The front panel's reset button on the model (`Console::reset_button`):
//! /RESET held low stops the program (the test cartridge's poll stops
//! with it), and the release runs the warm reset through the vectors, so
//! the CPU reads $FFFC and $FFFD after the release and not before it in
//! the run, and the cartridge's init runs again (it clears the RAM flag
//! that turns its poll on, which a program that was never reset would
//! still hold), after which the poll comes back when asked. MUTATE_RESET=1 holds
//! the same time with the line left high, and the test must go red.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, program};
use nes_console::{Alignment, Console};

#[test]
fn the_button_stops_the_program_and_the_release_reads_the_reset_vector() {
    let mutate = std::env::var("MUTATE_RESET").is_ok_and(|v| v == "1");
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.run_frames(3);
    // The cartridge polls in its NMI handler once $0001 is set (as
    // latch_frame.rs sets it); a warm reset leaves RAM as it was.
    c.board.borrow_mut().wram.write(0x0001, 1);
    c.run_frames(10);
    c.cpu_trace = Some(Vec::new());
    let latches_before = c.board.borrow().pads[0].latches;
    assert!(latches_before > 0, "the test cartridge polls every frame");
    // Five frames' worth of master half-steps held.
    let hold = 5 * 89_342 * 8;
    if mutate {
        c.run_master(hold);
    } else {
        c.reset_button(hold);
    }
    let released_at = c.master;
    assert_eq!(
        c.board.borrow().pads[0].latches,
        latches_before,
        "no poll while /RESET is held (MUTATE_RESET=1 leaves it high: red)"
    );
    c.run_frames(4);
    assert_eq!(c.board.borrow().wram.read(0x0001), 0, "the init ran again and cleared the poll flag");
    c.board.borrow_mut().wram.write(0x0001, 1);
    c.run_frames(4);
    let trace = c.cpu_trace.take().unwrap();
    let vector_reads: Vec<u64> = trace.iter().filter(|s| s.frame.rw && (s.frame.ab == 0xfffc || s.frame.ab == 0xfffd)).map(|s| s.master).collect();
    assert!(!vector_reads.is_empty(), "the release reads the reset vector");
    assert!(vector_reads.iter().all(|&m| m >= released_at), "and only after the release");
    assert!(c.board.borrow().pads[0].latches > latches_before, "the program polls again");
}
