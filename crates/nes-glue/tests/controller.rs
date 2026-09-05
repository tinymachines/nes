//! U9 and U10 against the SN74LS368A datasheet (inverting, three-state)
//! and the controller's 4021 against its documented order, with open
//! bus on every bit the buffers do not drive.

use nes_glue::controller::{read_4016, read_4017, Buttons, Controller};

#[test]
fn the_eight_buttons_come_out_in_order_then_ones() {
    let mut pad = Controller::default();
    pad.buttons = Buttons { a: true, start: true, left: true, ..Buttons::default() };
    pad.strobe(true);
    pad.strobe(false);
    // Port levels: low = pressed.
    let levels: Vec<bool> = (0..10).map(|_| pad.read()).collect();
    let pressed: Vec<bool> = levels.iter().map(|l| !l).collect();
    assert_eq!(pressed, [true, false, false, true, false, false, true, false, true, true], "A B Select Start Up Down Left Right, then the line low (D0 reads 1) for every later read");
}

#[test]
fn a_held_strobe_keeps_reloading_so_every_read_is_a() {
    let mut pad = Controller::default();
    pad.buttons = Buttons { a: true, ..Buttons::default() };
    pad.strobe(true);
    assert!((0..5).all(|_| !pad.read()), "with OUT0 high every read shows A");
    // The buttons at the fall are the ones latched, not the ones at the rise.
    pad.buttons = Buttons { b: true, ..Buttons::default() };
    pad.strobe(false);
    assert!(pad.read(), "A released");
    assert!(!pad.read(), "B pressed, second out");
}

#[test]
fn the_buffers_invert_the_driven_bits_and_leave_the_rest_to_the_bus() {
    // A pressed button pulls the port line low; U9 inverts it to a 1 on
    // D0. The expansion lines idle high, so D3 and D4 read 0.
    assert_eq!(read_4016(false, true, true, 0x40), 0x41);
    assert_eq!(read_4016(true, true, true, 0x40), 0x40);
    // Open bus carries through on D1, D2, D5..D7 exactly.
    assert_eq!(read_4016(true, true, true, 0xe6), 0xe6);
    assert_eq!(read_4016(false, false, false, 0x00), 0x19);
    // U10: D0 from port 2, D1..D4 from the expansion port; D5..D7 open bus.
    assert_eq!(read_4017(false, [true; 4], 0x40), 0x41);
    assert_eq!(read_4017(true, [true, true, false, true], 0x40), 0x48, "a Zapper's light sense on D3");
    assert_eq!(read_4017(true, [true; 4], 0xe0), 0xe0);
}
