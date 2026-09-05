//! The reset chain as authored: the hold after power good and after the
//! button, the button's own assertion, and the label on the number.

use nes_glue::reset::{ResetChain, HOLD_MASTER_CYCLES};

#[test]
fn power_on_holds_reset_for_the_labelled_span_then_releases() {
    let mut chain = ResetChain::power_on();
    assert!(!chain.reset_n(), "asserted at power good");
    chain.tick(HOLD_MASTER_CYCLES - 1);
    assert!(!chain.reset_n(), "still asserted one cycle short");
    chain.tick(1);
    assert!(chain.reset_n(), "released at the hold's end");
    chain.tick(1_000_000);
    assert!(chain.reset_n());
}

#[test]
fn the_button_asserts_at_once_and_the_hold_follows_its_release() {
    let mut chain = ResetChain::power_on();
    chain.tick(HOLD_MASTER_CYCLES);
    assert!(chain.reset_n());
    chain.button(true);
    assert!(!chain.reset_n(), "pressed: asserted");
    chain.tick(10 * HOLD_MASTER_CYCLES);
    assert!(!chain.reset_n(), "held down: stays asserted however long");
    chain.button(false);
    assert!(!chain.reset_n(), "released: the CIC's hold begins");
    chain.tick(HOLD_MASTER_CYCLES);
    assert!(chain.reset_n());
}

#[test]
fn the_hold_is_the_placeholder_it_says_it_is() {
    // About 50 ms of the 21.477272 MHz master: the label in reset.rs is
    // the claim, and this pins the number to it so a change comes with
    // a new label (or the scope capture that retires both).
    let ms = HOLD_MASTER_CYCLES as f64 / 21_477_272.0 * 1000.0;
    assert!((49.0..51.0).contains(&ms), "{ms} ms");
}
