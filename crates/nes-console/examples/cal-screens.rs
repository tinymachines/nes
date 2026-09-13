//! The calibration cartridge's screens, one PPM each, for a look: the
//! console runs the cartridge, steps the screens with Select the way a
//! hand would, and writes each screen's last frame through the family's
//! own picture path (ntsc-crt's decode of the console's composite, no
//! CRT stages), plus the strip each frame reads.
//!
//!   cargo run --release -p nes-console --example cal-screens -- <out_dir>
//!
//! VARIANT=n parks the palette and bars screens on variant n by running
//! the timer there (60 or 120 frames a variant) before the frame is
//! taken; the default is variant 0.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::cal::{self, read_strip, SCREENS, SCREEN_NAMES, VARIANT_FRAMES};
use nes_console::{picture, Alignment, Console, Picture};
use nes_glue::controller::Buttons;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&out).unwrap();
    let variant: usize = std::env::var("VARIANT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let cart = Nrom::new(cal::program(), cal::chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.run_frames(6);
    // Start holds the screen so the timer never steps it under us.
    c.set_pad(0, Buttons::from_byte(0x08));
    c.run_frames(2);
    c.set_pad(0, Buttons::from_byte(0));
    c.run_frames(3);
    for s in 0..SCREENS {
        if s > 0 {
            c.set_pad(0, Buttons::from_byte(0x04));
            c.run_frames(2);
            c.set_pad(0, Buttons::from_byte(0));
            c.run_frames(4);
        }
        if s == 1 || s == 2 {
            c.run_frames(variant * VARIANT_FRAMES[s] as usize);
        }
        let mut p = Picture::decode_only();
        let n = c.frames.len();
        let mut shown = None;
        for f in &c.frames[n - 3..] {
            shown = Some(p.push(f));
        }
        let f = c.frames.last().unwrap();
        let strip = read_strip(f);
        let path = format!("{out}/cal-{s}-{}.ppm", SCREEN_NAMES[s]);
        std::fs::write(&path, picture::decoded_ppm(&shown.unwrap().decoded)).unwrap();
        match strip {
            Ok(st) => println!("{path}: screen {} variant {} hold {} frame {} pad {:02x}", st.screen, st.variant, st.hold, st.frame, st.pad),
            Err(e) => println!("{path}: the strip does not read: {e}"),
        }
    }
}
