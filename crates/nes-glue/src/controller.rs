//! U9 and U10, two 74LS368 hex inverting three-state buffers, the
//! controller read path, and the standard controller on the other end
//! of it.
//!
//! AUTHORED from the SN74LS368A datasheet (an enabled output is the
//! inverted input; a disabled output floats) and the NES-001 wiring:
//!
//! - U9, enabled by the 2A03's /OE1 (a read of $4016): drives D0 with
//!   controller port 1's data line inverted, and D3 and D4 with two
//!   expansion port lines; D1, D2 and D5..D7 are not driven.
//! - U10, enabled by /OE2 (a read of $4017): drives D0 with port 2's
//!   data line inverted and D1..D4 with expansion lines (a Zapper's
//!   light sense and trigger arrive on D3 and D4).
//!
//! The data line at the port is pulled up and a pressed button pulls the
//! shift register's output LOW, so the inversion is what makes a pressed
//! button read as 1. The bits the buffers do not drive keep whatever the
//! bus held, which is the console's open bus: this module takes that
//! value in and merges around the driven bits, never inventing it.
//!
//! The standard controller is a 4021 shift register: while OUT0 is high
//! it loads the eight buttons continuously, its fall latches them as
//! they stand at that moment, and each read after it clocks one bit out
//! in the order A, B, Select, Start, Up, Down, Left, Right. Past the
//! eighth the line the port shows sits LOW, so the CPU reads 1 on D0
//! (nesdev's "official controllers return 1 after the eighth read"; the
//! register's serial input is wired to give that level). The diode
//! arrays on the port lines are protection and have no behaviour here.

/// The eight buttons in the shift register's order, bit 0 first out.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Buttons {
    pub a: bool,
    pub b: bool,
    pub select: bool,
    pub start: bool,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl Buttons {
    fn as_byte(self) -> u8 {
        (self.a as u8)
            | (self.b as u8) << 1
            | (self.select as u8) << 2
            | (self.start as u8) << 3
            | (self.up as u8) << 4
            | (self.down as u8) << 5
            | (self.left as u8) << 6
            | (self.right as u8) << 7
    }
}

/// A standard controller: the 4021, from the port's side.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Controller {
    pub buttons: Buttons,
    /// The register, bit 0 next out; ones shift in from the top.
    shift: u8,
    strobe: bool,
}

impl Controller {
    /// OUT0 as the console drives it: high keeps the register loading
    /// (so every read returns A), and the fall latches the buttons as
    /// they stand then.
    pub fn strobe(&mut self, out0: bool) {
        if out0 || self.strobe {
            self.shift = self.buttons.as_byte();
        }
        self.strobe = out0;
    }

    /// The data line's LEVEL at the port for one read, and the shift the
    /// read's clock causes: low when the button now at the head of the
    /// register is pressed, and low (a 1 on D0) for every read past the
    /// eighth.
    pub fn read(&mut self) -> bool {
        if self.strobe {
            self.shift = self.buttons.as_byte();
        }
        let pressed = self.shift & 1 != 0;
        if !self.strobe {
            self.shift = (self.shift >> 1) | 0x80;
        }
        !pressed
    }
}

/// U9's view of a $4016 read: the three driven bits merged over the
/// open bus. `port_data` and the two expansion lines are pin levels.
pub fn read_4016(port_data: bool, exp_d3: bool, exp_d4: bool, open_bus: u8) -> u8 {
    let driven = (!port_data as u8) | (!exp_d3 as u8) << 3 | (!exp_d4 as u8) << 4;
    (open_bus & !0b0001_1001) | driven
}

/// U10's view of a $4017 read: D0 from port 2, D1..D4 from the
/// expansion port, inverted; D5..D7 open bus.
pub fn read_4017(port_data: bool, exp_d1_d4: [bool; 4], open_bus: u8) -> u8 {
    let mut driven = (!port_data) as u8;
    for (i, &e) in exp_d1_d4.iter().enumerate() {
        driven |= (!e as u8) << (i + 1);
    }
    (open_bus & !0b0001_1111) | driven
}
