//! The power-on reset chain: the front panel's reset button (RST_PB),
//! the CIC lockout chip's reset output, and the /RESET line both chips
//! and the cartridge see.
//!
//! AUTHORED, and the one part of this crate that is a placeholder by its
//! own admission. On the NES-001 the button and the CIC both drive the
//! reset line: pressing the button asserts it at once, and the CIC holds
//! it asserted for a while after power comes up and after the button is
//! released while its handshake with the cartridge's CIC runs (and keeps
//! pulsing it, the famous blink, when that handshake fails). The hold's
//! length is not known here to a master cycle; the console sketch's
//! section 5 names the capture that replaces it (CPU_RST, RST_PB, CIC_RST
//! and PWR_LED at power-on, 1 MSa/s, a long record), and until then
//! `HOLD_MASTER_CYCLES` is a number with this label on it.

/// How long the CIC holds /RESET low after power good or the button's
/// release, in master clock cycles (21.477272 MHz). AUTHORED PLACEHOLDER:
/// about 50 ms, a round figure inside the range the lockout handshake
/// takes; the scope capture named above replaces it.
pub const HOLD_MASTER_CYCLES: u64 = 1_073_864;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ResetChain {
    button: bool,
    /// Master cycles of hold remaining once the button is up.
    hold: u64,
}

impl Default for ResetChain {
    fn default() -> ResetChain {
        ResetChain::power_on()
    }
}

impl ResetChain {
    /// Power comes up: the hold starts, the button is up.
    pub fn power_on() -> ResetChain {
        ResetChain { button: false, hold: HOLD_MASTER_CYCLES }
    }

    /// RST_PB as a level: pressed = true.
    pub fn button(&mut self, pressed: bool) {
        if self.button && !pressed {
            self.hold = HOLD_MASTER_CYCLES;
        }
        self.button = pressed;
    }

    /// Advance the master clock by `cycles`.
    pub fn tick(&mut self, cycles: u64) {
        if !self.button {
            self.hold = self.hold.saturating_sub(cycles);
        }
    }

    /// /RESET's LEVEL: low (false) while asserted.
    pub fn reset_n(&self) -> bool {
        !(self.button || self.hold > 0)
    }
}
