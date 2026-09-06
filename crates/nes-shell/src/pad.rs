//! Controller 1 from a gamepad, beside the keyboard.
//!
//! gilrs delivers the pad as events; this drains them into a `Buttons`
//! each display tick, and the window ORs it with the keyboard's before
//! the console thread reads it. The layout is positional against the
//! NES pad: the east and north face buttons are A, the south and west
//! are B (so a SNES-style thumb lands on B and A where the NES puts
//! them, and the lower pair alone works too), Start and Select by
//! name, the D-pad by name, and the left stick past `STICK_THRESHOLD`
//! on either axis. No pad, or no way to enumerate one (a session
//! without udev), is not an error: the keyboard still plays, and the
//! reason prints once.

use nes_glue::controller::Buttons;

/// The left stick counts as a direction beyond this deflection, in
/// gilrs's unit range; the same edge both ways.
pub const STICK_THRESHOLD: f32 = 0.5;

pub struct Pad {
    gilrs: Option<gilrs::Gilrs>,
    /// The pad's buttons as of the last poll.
    pub buttons: Buttons,
    /// The left stick as of the last axis event, x then y.
    pub stick: (f32, f32),
    /// Distinct pads that have sent an event, for the exit line.
    pub seen: std::collections::BTreeSet<usize>,
}

impl Pad {
    /// Opens the enumerator; without one the pad is empty and stays so.
    pub fn open() -> Pad {
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => {
                let n = g.gamepads().count();
                if n > 0 {
                    for (_, gp) in g.gamepads() {
                        eprintln!("nes-shell: gamepad: {} ({:?})", gp.name(), gp.power_info());
                    }
                }
                Some(g)
            }
            Err(e) => {
                eprintln!("nes-shell: no gamepad enumerator ({e}); the keyboard is controller 1");
                None
            }
        };
        Pad { gilrs, buttons: Buttons::default(), stick: (0.0, 0.0), seen: Default::default() }
    }

    /// Drains the pending events and returns the pad's buttons.
    pub fn poll(&mut self) -> Buttons {
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = g.next_event() {
                self.seen.insert(ev.id.into());
                match ev.event {
                    gilrs::EventType::ButtonPressed(b, _) => apply_button(&mut self.buttons, b, true),
                    gilrs::EventType::ButtonReleased(b, _) => apply_button(&mut self.buttons, b, false),
                    gilrs::EventType::AxisChanged(a, v, _) => apply_axis(&mut self.buttons, &mut self.stick, a, v),
                    gilrs::EventType::Disconnected => self.buttons = Buttons::default(),
                    _ => {}
                }
            }
        }
        self.buttons
    }
}

/// One face or shoulder button of the pad onto the NES pad, or nothing.
pub fn apply_button(b: &mut Buttons, button: gilrs::Button, down: bool) {
    use gilrs::Button::*;
    match button {
        East | North => b.a = down,
        South | West => b.b = down,
        Start => b.start = down,
        Select => b.select = down,
        DPadUp => b.up = down,
        DPadDown => b.down = down,
        DPadLeft => b.left = down,
        DPadRight => b.right = down,
        _ => {}
    }
}

/// The left stick onto the D-pad: each axis past the threshold sets its
/// direction and clears the opposite one; inside it clears both. The
/// D-pad's own buttons set the same bits, so a stick at rest does not
/// undo a held D-pad: the stick only writes when it has moved.
pub fn apply_axis(b: &mut Buttons, stick: &mut (f32, f32), axis: gilrs::Axis, v: f32) {
    let (was, now) = match axis {
        gilrs::Axis::LeftStickX => (stick.0, &mut stick.0),
        gilrs::Axis::LeftStickY => (stick.1, &mut stick.1),
        _ => return,
    };
    *now = v;
    let (neg, pos) = match axis {
        gilrs::Axis::LeftStickX => (&mut b.left, &mut b.right),
        _ => (&mut b.down, &mut b.up), // gilrs: +y is up
    };
    let dir = |x: f32| (x >= STICK_THRESHOLD) as i8 - (x <= -STICK_THRESHOLD) as i8;
    if dir(was) != dir(v) {
        *neg = dir(v) < 0;
        *pos = dir(v) > 0;
    }
}

/// The keyboard's buttons and the pad's, either held counting.
pub fn merge(k: Buttons, p: Buttons) -> Buttons {
    Buttons {
        a: k.a | p.a,
        b: k.b | p.b,
        select: k.select | p.select,
        start: k.start | p.start,
        up: k.up | p.up,
        down: k.down | p.down,
        left: k.left | p.left,
        right: k.right | p.right,
    }
}
