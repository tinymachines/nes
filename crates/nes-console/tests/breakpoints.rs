//! Breakpoints (`Console::run_frames_until`): the console stops as the CPU
//! begins fetching the opcode at a chosen address, inside that fetch, and
//! runs on past it when asked. The test cartridge's NMI handler runs once
//! a frame, so a breakpoint on its first instruction stops once a frame,
//! a picture apart; an address the program never runs never stops it; and
//! a run broken into stops makes exactly the pictures a straight run
//! makes, so looking does not move the machine. MUTATE_BREAK=1 counts a
//! fetch already under way and must go red.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{testrom, Alignment, Console};

fn console() -> Console {
    let cart = Nrom::new(testrom::program(), testrom::chr(), Mirroring::Vertical).expect("NROM");
    Console::new(Box::new(cart), None, Alignment::default())
}

fn nmi_vector(c: &Console) -> u16 {
    c.peek(0xfffa) as u16 | (c.peek(0xfffb) as u16) << 8
}

#[test]
fn a_breakpoint_on_the_nmi_handler_stops_once_a_frame_inside_its_fetch() {
    let mut c = console();
    c.run_frames(2);
    let nmi = nmi_vector(&c);
    let mut stops = Vec::new();
    for _ in 0..6 {
        let hit = c.run_frames_until(3, &[nmi]).expect("the handler runs every frame");
        assert_eq!(hit, nmi);
        assert_eq!(c.cpu_registers().5, nmi, "stopped in the fetch of the instruction at the breakpoint: the PC the code panel lights");
        assert!(v6502_pins::PinEngine::pins(&c.cpu).sync, "inside the fetch");
        stops.push(c.frames_done);
    }
    // One stop a picture: running on goes past the stop it stood at.
    assert!(stops.windows(2).all(|w| w[1] == w[0] + 1), "{stops:?}");
}

#[test]
fn an_address_never_run_never_stops_the_console() {
    let mut c = console();
    let before = c.frames.len();
    assert_eq!(c.run_frames_until(4, &[0x0123]), None);
    assert_eq!(c.frames.len() - before, 4);
}

#[test]
fn a_run_broken_into_stops_makes_the_same_pictures() {
    let mut a = console();
    let mut b = console();
    a.run_frames(2);
    b.run_frames(2);
    let nmi = nmi_vector(&a);
    a.run_frames(8);
    let mut n = 0;
    while n < 8 {
        let before = b.frames.len();
        b.run_frames_until(8 - n, &[nmi, nmi.wrapping_add(3)]);
        n += b.frames.len() - before;
    }
    assert_eq!(a.frames.len(), b.frames.len());
    for (x, y) in a.frames.iter().zip(&b.frames) {
        assert!(x.colour == y.colour && x.emphasis == y.emphasis);
    }
    assert_eq!(a.master, b.master, "the same clock when the last picture completed");
}
