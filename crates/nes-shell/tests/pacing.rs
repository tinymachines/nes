//! N8 step 2's headless gate: the paced loop on a synthetic clock. At
//! exactly the source period every tick advances one frame (no
//! duplicates, no drops); at half the period every other tick presents
//! the previous frame again; at twice the period every tick drops one.
//! The sound the loop banks per tick is the frame's worth at 48 kHz,
//! and the ring never holds more than a quarter second.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, program};
use nes_console::{Alignment, Console};
use nes_shell::run::Loop;

fn console() -> Console {
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    Console::new(Box::new(cart), None, Alignment::default())
}

/// The NES rendering-enabled pair period in nanoseconds, as ntsc-wasm
/// states it: two frames per 714,736 + 714,728 grid samples.
fn period_ns() -> f64 {
    (714_736.0 + 714_728.0) / 2.0 / (472_500_000.0 / 11.0) * 1e9
}

#[test]
fn the_loop_advances_by_whole_frames_as_the_clock_says() {
    let period = period_ns();
    for (label, dt, ticks, want_dup, want_drop) in [("at the period", period, 60.0, 0u64, 0u64), ("at half the period", period / 2.0, 60.0, 30, 0), ("at twice the period", period * 2.0, 20.0, 0, 20)] {
        let mut l = Loop::new(console());
        let mut produced = 0;
        for _ in 0..ticks as usize {
            if l.tick(dt as u64).is_some() {
                produced += 1;
            }
        }
        let s = l.stats();
        eprintln!("{label}: {} ticks, {} frames presented new, {} duplicated, {} dropped, {} frames run", s.presented, produced, s.duplicated, s.dropped, l.frames_run);
        assert_eq!(s.presented, ticks as u64);
        assert!((s.duplicated as i64 - want_dup as i64).abs() <= 1, "{label}: {} duplicated, expected about {want_dup}", s.duplicated);
        assert!((s.dropped as i64 - want_drop as i64).abs() <= 1, "{label}: {} dropped, expected about {want_drop}", s.dropped);
        assert_eq!(produced as u64, s.presented - s.duplicated, "a tick that advanced produced a frame");
    }
}

#[test]
fn the_ring_carries_a_frames_worth_of_sound_and_no_more_than_a_quarter_second() {
    let period = period_ns();
    let mut l = Loop::new(console());
    // A nanosecond over the period, so the first tick is due (the
    // period is not a whole number of nanoseconds).
    let dt = period as u64 + 1;
    l.tick(dt);
    let n1 = l.ring.lock().unwrap().samples.len();
    // A frame at 60.0988 Hz is 798.7 samples at 48 kHz.
    assert!((790..=810).contains(&n1), "one frame banked {n1} samples");
    // Nothing drains: after many frames the ring is capped.
    for _ in 0..60 {
        l.tick(dt);
    }
    let n = l.ring.lock().unwrap().samples.len();
    assert_eq!(n, 12_000, "the ring holds a quarter second at most, {n}");
    // And a drain past what it holds counts the underrun.
    let mut out = vec![0.0f32; 13_000];
    l.ring.lock().unwrap().pull(&mut out);
    assert_eq!(l.ring.lock().unwrap().underrun, 1_000);
    assert!(out[..12_000].iter().any(|v| *v != 0.0), "the drained sound is not silence");
}
