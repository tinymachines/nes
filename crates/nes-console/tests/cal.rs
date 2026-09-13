//! The calibration cartridge (nes-bench/docs/calibration-plan.md, C0):
//! every frame names itself, the screens step by Select and by the
//! timer, each screen's regions hold what the manifest says they hold,
//! and the pad's byte is echoed in the strip two frames after the blanking
//! that polled it.
//! MUTATE=1 reads the strip one tile to the right of where the manifest
//! puts it and must go red.

use nes_bus::cart::{Mirroring, Nrom};
use nes_bus::DotFrame;
use nes_console::cal::{self, read_strip_shifted, regions, Strip, What, BLACK, CONTENT_ROW, SCREENS, WHITE};
use nes_console::{Alignment, Console};
use nes_glue::controller::Buttons;

fn console() -> Console {
    let cart = Nrom::new(cal::program(), cal::chr(), Mirroring::Vertical).unwrap();
    Console::new(Box::new(cart), None, Alignment::default())
}

fn strip(f: &DotFrame) -> Result<Strip, String> {
    let dx = if std::env::var_os("MUTATE").is_some() { 8 } else { 0 };
    read_strip_shifted(f, dx, 0)
}

fn press(c: &mut Console, byte: u8, held: usize, released: usize) {
    c.set_pad(0, Buttons::from_byte(byte));
    c.run_frames(held);
    c.set_pad(0, Buttons::from_byte(0));
    c.run_frames(released);
}

fn at(f: &DotFrame, x: usize, y: usize) -> (u8, u8) {
    f.at(y, x + nes_bus::ACTIVE_FIRST_DOT)
}

#[test]
fn every_frame_names_itself_and_counts() {
    let mut c = console();
    c.run_frames(12);
    let first = (0..12).find(|&i| strip(&c.frames[i]).is_ok()).expect("no frame in the first twelve carries a readable strip");
    assert!(first <= 6, "the strip reads from frame {first}; the program should be drawing within six");
    let s0 = strip(&c.frames[first]).unwrap();
    assert_eq!((s0.screen, s0.variant, s0.hold, s0.pad), (0, 0, false, 0));
    for i in first..12 {
        let s = strip(&c.frames[i]).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        assert_eq!(s.frame, s0.frame + (i - first) as u16, "frame {i} counts on from frame {first}");
        assert_eq!(s.screen, 0);
    }
}

#[test]
fn select_steps_the_screens_and_each_holds_its_regions() {
    let mut c = console();
    c.run_frames(6);
    press(&mut c, 0x08, 2, 3); // Start: hold, so the timer never steps under us
    for s in 1..=SCREENS {
        press(&mut c, 0x04, 2, 4);
        let f = c.frames.last().unwrap();
        let st = strip(f).unwrap_or_else(|e| panic!("after {s} presses of Select: {e}"));
        assert_eq!(st.screen as usize, s % SCREENS, "Select steps to the next screen");
        assert!(st.hold, "Start holds");
        assert_eq!(st.variant, 0, "a fresh screen starts on variant 0");
        check_regions(f, s % SCREENS, 0);
    }
}

/// Every region of screen `s` on variant `v`, against the frame.
fn check_regions(f: &DotFrame, s: usize, v: usize) {
    let emphasis = if s == 1 { (v % 8) as u8 } else { 0 };
    for r in regions(s) {
        match &r.what {
            What::Flat { entries } => {
                let want = entries[v];
                for (x, y) in [(r.x + r.w / 2, r.y + r.h / 2), (r.x + 1, r.y + 1), (r.x + r.w - 2, r.y + r.h - 2)] {
                    let (got, e) = at(f, x, y);
                    let want = want.unwrap_or(if s == 6 { 0x00 } else { BLACK });
                    assert_eq!(got, want, "screen {s} variant {v} region {} at ({x},{y}): entry {got:02x}, the manifest says {want:02x}", r.name);
                    assert_eq!(e, emphasis, "screen {s} variant {v} region {}: emphasis", r.name);
                }
            }
            What::Pattern { kind, pitch, axis } => {
                let mut white = 0;
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        let (dx, dy) = (x - r.x, y - r.y);
                        let expect_white = match *axis {
                            "x" => (dx / pitch) % 2 == 0,
                            "y" => (dy / pitch) % 2 == 0,
                            _ => ((dx / pitch) + (dy / pitch)) % 2 == 0,
                        };
                        let (got, _) = at(f, x, y);
                        let want = if expect_white { WHITE } else { BLACK };
                        if s == 5 {
                            // the dot crawl draws its checkers in colour, not white
                            assert!(got == BLACK || !expect_white || got != BLACK, "screen 5 {kind}");
                            assert_eq!(got != BLACK, expect_white, "screen {s} {kind} at ({x},{y})");
                        } else {
                            assert_eq!(got, want, "screen {s} {kind} at ({x},{y}): {got:02x}");
                        }
                        white += expect_white as usize;
                    }
                }
                assert_eq!(white * 2, r.w * r.h, "screen {s} {kind}: half the dots are lit");
            }
        }
    }
    if s == 7 {
        // the border, the crosshair and a tick, in dots
        assert_eq!(at(f, 0, 0).0, WHITE, "top-left corner");
        assert_eq!(at(f, 100, 0).0, WHITE, "top border");
        assert_eq!(at(f, 255, 239).0, WHITE, "bottom-right corner");
        assert_eq!(at(f, 128, 100).0, WHITE, "the vertical of the crosshair at x = 128");
        assert_eq!(at(f, 50, 120).0, WHITE, "the horizontal of the crosshair at y = 120");
        assert_eq!(at(f, 129, 100).0, BLACK, "one dot right of the vertical");
        assert_eq!(at(f, 34, 5).0, WHITE, "a tick at tile column 4 is a full tile");
        assert_eq!(at(f, 50, 5).0, BLACK, "between ticks the border is one dot");
    }
    // the content rows below the strip carry nothing on screen 0
    if s == 0 {
        assert!((CONTENT_ROW * 8..240).all(|y| (0..256).all(|x| at(f, x, y).0 == BLACK)), "screen 0 shows the strip alone");
    }
}

#[test]
fn the_timer_steps_the_screens_and_the_variants_unattended() {
    let mut c = console();
    c.run_frames(250);
    let st = strip(c.frames.last().unwrap()).unwrap();
    assert_eq!((st.screen, st.variant), (1, 0), "screen 0 lasts 240 frames, then the palette screen");
    check_regions(c.frames.last().unwrap(), 1, 0);
    c.run_frames(60);
    let st = strip(c.frames.last().unwrap()).unwrap();
    assert_eq!((st.screen, st.variant), (1, 1), "sixty frames on, the palette screen is on its second variant");
    check_regions(c.frames.last().unwrap(), 1, 1);
}

#[test]
fn the_pad_byte_is_echoed_the_frame_after_the_poll_and_paints_the_field() {
    let mut c = console();
    c.run_frames(6);
    press(&mut c, 0x08, 2, 3);
    for _ in 0..6 {
        press(&mut c, 0x04, 2, 4);
    }
    let f = c.frames.last().unwrap();
    assert_eq!(strip(f).unwrap().screen, 6);
    let field = regions(6).into_iter().next().unwrap();
    let centre = |f: &DotFrame| at(f, field.x + field.w / 2, field.y + field.h / 2).0;
    assert_eq!(centre(f), 0x00, "no button: the field is grey");
    let n = c.frames.len();
    c.set_pad(0, Buttons::from_byte(0xb1)); // A, Up, Down, Right: no Select, no Start
    c.run_frames(4);
    let echoed: Vec<(u8, u8)> = (n..n + 4).map(|i| (strip(&c.frames[i]).unwrap().pad, centre(&c.frames[i]))).collect();
    let first = echoed.iter().position(|&(p, _)| p == 0xb1).expect("the byte is echoed within four frames");
    // The byte is applied at the next strobe, in the blanking that ends
    // frame n; the main loop builds it into the strip during n+1; the
    // blanking that ends n+1 writes it; frame n+2 shows it.
    assert_eq!(first, 2, "the strip echoes the byte two frames after the blanking that polled it: {echoed:?}");
    assert!(echoed[first..].iter().all(|&(p, col)| p == 0xb1 && col == 0x2a), "the echo holds and the field is green the same frame: {echoed:?}");
    assert!(echoed[..first].iter().all(|&(_, col)| col == 0x00), "the field is grey until the echo: {echoed:?}");
}

#[test]
fn the_manifest_and_the_program_are_one_generator() {
    let m = cal::manifest();
    for name in cal::SCREEN_NAMES {
        assert!(m.contains(&format!("\"name\": \"{name}\"")), "the manifest names screen {name}");
    }
    assert_eq!(m.matches("\"pattern\"").count(), 12, "twelve pattern regions across the gratings and the dot crawl");
    assert_eq!(m.matches("\"entries\"").count(), 8 + 40 + 8 + 1, "the flat regions of the palette, bars, edges and pad screens");
    let p = cal::program();
    assert_eq!(p.len(), 0x8000);
    let reset = u16::from_le_bytes([p[0x7ffc], p[0x7ffd]]);
    assert_eq!(reset, 0x8000, "reset at the start of PRG");
}
