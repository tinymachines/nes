//! The gamepad's mapping onto the NES pad, without a pad: the layout
//! table, the stick's threshold both ways, the D-pad and stick sharing
//! bits, and the OR with the keyboard. `Pad::open` is exercised for
//! not failing on a box with no pad (or no enumerator).

use gilrs::{Axis, Button};
use nes_glue::controller::Buttons;
use nes_shell::pad::{apply_axis, apply_button, merge, Pad, STICK_THRESHOLD};

fn pressed(button: Button) -> Buttons {
    let mut b = Buttons::default();
    apply_button(&mut b, button, true);
    b
}

#[test]
fn the_face_buttons_land_positionally_and_the_rest_by_name() {
    let one = |f: fn(&mut Buttons)| {
        let mut b = Buttons::default();
        f(&mut b);
        b
    };
    assert_eq!(pressed(Button::East), one(|b| b.a = true), "east is A");
    assert_eq!(pressed(Button::North), one(|b| b.a = true), "north is A");
    assert_eq!(pressed(Button::South), one(|b| b.b = true), "south is B");
    assert_eq!(pressed(Button::West), one(|b| b.b = true), "west is B");
    assert_eq!(pressed(Button::Start), one(|b| b.start = true));
    assert_eq!(pressed(Button::Select), one(|b| b.select = true));
    assert_eq!(pressed(Button::DPadUp), one(|b| b.up = true));
    assert_eq!(pressed(Button::DPadDown), one(|b| b.down = true));
    assert_eq!(pressed(Button::DPadLeft), one(|b| b.left = true));
    assert_eq!(pressed(Button::DPadRight), one(|b| b.right = true));
    for other in [Button::LeftTrigger, Button::RightTrigger, Button::LeftThumb, Button::Mode, Button::C, Button::Z, Button::Unknown] {
        assert_eq!(pressed(other), Buttons::default(), "{other:?} maps to nothing");
    }
    // A release clears only its own bit.
    let mut b = Buttons { a: true, b: true, ..Default::default() };
    apply_button(&mut b, Button::East, false);
    assert_eq!(b, Buttons { b: true, ..Default::default() });
}

#[test]
fn the_left_stick_is_a_direction_past_the_threshold_and_not_inside_it() {
    let mut b = Buttons::default();
    let mut stick = (0.0, 0.0);
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, STICK_THRESHOLD - 0.01);
    assert_eq!(b, Buttons::default(), "just inside is nothing");
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, STICK_THRESHOLD);
    assert_eq!(b, Buttons { right: true, ..Default::default() }, "the threshold itself counts");
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, -1.0);
    assert_eq!(b, Buttons { left: true, ..Default::default() }, "the other way clears the first");
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, 0.0);
    assert_eq!(b, Buttons::default(), "centre clears both");
    // gilrs's y axis is positive upward.
    apply_axis(&mut b, &mut stick, Axis::LeftStickY, 1.0);
    assert_eq!(b, Buttons { up: true, ..Default::default() });
    apply_axis(&mut b, &mut stick, Axis::LeftStickY, -1.0);
    assert_eq!(b, Buttons { down: true, ..Default::default() });
    // The right stick and the triggers are not directions.
    apply_axis(&mut b, &mut stick, Axis::RightStickX, 1.0);
    apply_axis(&mut b, &mut stick, Axis::LeftZ, 1.0);
    assert_eq!(b, Buttons { down: true, ..Default::default() });
}

#[test]
fn a_held_dpad_survives_a_stick_that_has_not_moved() {
    let mut b = Buttons::default();
    let mut stick = (0.0, 0.0);
    apply_button(&mut b, Button::DPadLeft, true);
    // Noise inside the threshold: no change of direction, no write.
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, 0.1);
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, -0.2);
    assert_eq!(b, Buttons { left: true, ..Default::default() });
    // The stick going right overrides; back to centre clears, since the
    // stick cannot know the D-pad is still down (one set of bits).
    apply_axis(&mut b, &mut stick, Axis::LeftStickX, 1.0);
    assert_eq!(b, Buttons { right: true, ..Default::default() });
}

#[test]
fn the_keyboard_and_the_pad_are_ored() {
    let k = Buttons { a: true, up: true, ..Default::default() };
    let p = Buttons { b: true, up: true, start: true, ..Default::default() };
    assert_eq!(merge(k, p), Buttons { a: true, b: true, up: true, start: true, ..Default::default() });
    assert_eq!(merge(k, Buttons::default()), k);
    assert_eq!(merge(Buttons::default(), p), p);
}

#[test]
fn opening_without_a_pad_is_not_an_error_and_polls_empty() {
    let mut pad = Pad::open();
    assert_eq!(pad.poll(), Buttons::default());
    assert_eq!(pad.seen.len(), 0, "no pad sent an event on this box");
}
