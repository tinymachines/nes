//! The model's subcarrier origin frame by frame, and which frames could
//! account for a difference against the part.
//!
//!   cargo run --release -p nes-console --example origin-walk -- rom.nes [frames]
//!
//! `split-score`'s measurement 6 names the origin on both sides at one
//! frame and prints the difference. It cannot say where the difference
//! came from, because it sees one frame. This walks the model's own
//! frames from power-on and prints the origin it carries into each, the
//! parity it left with, and the step that parity is worth, so the
//! accumulation can be read rather than inferred.
//!
//! The arithmetic this exists to serve: a frame steps the origin by
//! `samples_per_frame mod 12`, which is 4 for a full frame (`Even` or
//! `OddFull`) and 8 for a short one (`OddShort`, an odd frame with
//! rendering on, whose pre-render line drops a dot). So the origin
//! alternates between two of the three values while rendering stays on,
//! and every frame where the model and the part disagree about
//! rendering moves their origins 4 apart and leaves them there.
//!
//! **A difference of 4 is one such frame, 8 is two, 0 is three or
//! none**, which is why a sweep can read 0 in the middle of a range
//! that is diverging: the count wraps mod 3 and nothing about a single
//! record says which side of the wrap it is on.
//!
//! What to look at: `RENDERING` marks every frame where the model's
//! rendering state changed, which is where a disagreement can start. An
//! odd frame printed `OddFull` is one the model rendered nothing on; if
//! the part was rendering there, that frame is worth 4.
//!
//! SCRIPT, LATCH and KNOBS as the other probes read them. `TAIL=n`
//! prints the last n frames as well as the first, and `ALL=1` prints
//! every frame.

use nes_bus::FrameParity;
use nes_console::{ines, Console, Picture};
use ntsc_grid::{Geometry, Phase};

fn name(p: FrameParity) -> &'static str {
    match p {
        FrameParity::Even => "Even",
        FrameParity::OddFull => "OddFull",
        FrameParity::OddShort => "OddShort",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let mut c = Console::with_prg_ram(cart, chr_ram, nes_console::knobs::alignment_from_env(), true);
    nes_console::knobs::configure_from_env(&mut c);

    // The pad schedule, exactly as frame-motion and split-score read it,
    // so a run here is the same run the record was taken under.
    if let Ok(path) = std::env::var("SCRIPT") {
        let mut pad = 0u8;
        let mut schedule = Vec::new();
        for line in std::fs::read_to_string(&path).expect("SCRIPT file").lines() {
            let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
            match f.as_slice() {
                ["AT", n, b] => schedule.push((n.parse().expect("latch"), u8::from_str_radix(b, 16).expect("hex byte"))),
                ["SET", b] => pad = u8::from_str_radix(b, 16).expect("hex byte"),
                _ => {}
            }
        }
        c.board.borrow_mut().pads[0].schedule = schedule;
        c.set_pad(0, nes_glue::controller::Buttons::from_byte(pad));
    }

    let mut target = None;
    if let Ok(t) = std::env::var("LATCH") {
        let t: u64 = t.parse().expect("LATCH");
        let (f, pos) = c.run_to_picture_after_latch(t, frames).unwrap_or_else(|e| panic!("{e}"));
        println!("latch {t} fell at PPU line {} dot {}: the part's frame is the model's picture {f}", pos.line, pos.dot);
        target = Some(f);
    }
    if c.frames.len() < frames {
        c.run_frames(frames - c.frames.len());
    }

    let geo = Geometry::nes();
    let head: usize = std::env::var("HEAD").ok().and_then(|v| v.parse().ok()).unwrap_or(16);
    let tail: usize = std::env::var("TAIL").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let all = std::env::var("ALL").is_ok_and(|v| v == "1");

    println!("the model's origin frame by frame (12 samples to a subcarrier cycle; a full frame steps 4, a short one 8)");
    println!("  {:>6}  {:>8}  {:>4}  {:>6}  note", "frame", "parity", "step", "origin");

    // The origin Picture would carry: the same seed and the same chain,
    // recomputed here so this probe does not have to encode every frame
    // (which is what makes it cheap enough to run to a thousand).
    let mut origin = Phase::new(0);
    let mut shorts = 0usize;
    let mut rendering: Option<bool> = None;
    let mut changes = Vec::new();
    for (i, f) in c.frames.iter().enumerate() {
        let step = geo.frame_residue(f.parity);
        let short = f.parity == FrameParity::OddShort;
        if short {
            shorts += 1;
        }
        // An odd frame says whether the part was rendering: short if it
        // was, full if it was not. An even frame cannot say, so it does
        // not get a vote and the last odd frame's answer stands.
        let mut note = String::new();
        if f.parity != FrameParity::Even {
            let now = short;
            if rendering != Some(now) {
                if rendering.is_some() {
                    note = format!("RENDERING {} here", if now { "came on" } else { "went off" });
                    changes.push((i, now));
                } else {
                    note = format!("rendering {} at the first odd frame", if now { "on" } else { "off" });
                }
                rendering = Some(now);
            }
        }
        if Some(i) == target {
            note = if note.is_empty() { "<- F, the frame the record caught".into() } else { format!("{note}; <- F") };
        }
        let show = all || i < head || (tail > 0 && i + tail >= c.frames.len()) || !note.is_empty();
        if show {
            println!("  {:>6}  {:>8}  {:>4}  {:>6}  {}", i, name(f.parity), step, origin.get(), note);
        }
        origin = geo.next_origin(origin, f.parity);
    }

    println!();
    println!("{} frames, {shorts} of them short; the origin ends at {}", c.frames.len(), origin.get());
    println!("rendering changed {} times after the first odd frame: {}", changes.len(),
        if changes.is_empty() { "never".to_string() } else { changes.iter().map(|(i, on)| format!("{i} {}", if *on { "on" } else { "off" })).collect::<Vec<_>>().join(", ") });
    if let Some(f) = target {
        // What split-score's measurement 6 will read for this run, so
        // the two probes can be checked against each other by eye.
        let mut o = Phase::new(0);
        for fr in c.frames.iter().take(f) {
            o = geo.next_origin(o, fr.parity);
        }
        println!("the model's origin at F ({f}) is {}, which is what split-score's measurement 6 prints as the model's", o.get());
        println!("a part reading 4 more than that missed one short frame here, 8 more two, 0 three or none: the count wraps mod 3");
    }

    // The chain above is recomputed rather than encoded, which is the
    // only reason this probe can run to a thousand frames. That makes it
    // a second implementation of something `Picture` already does, so it
    // is held to `Picture` frame for frame rather than assumed equal.
    //
    // Bounded on purpose: encoding is the expensive half and the chain
    // is a fold, so a drift shows up at the first frame that differs and
    // never later. CHECK=n moves the bound, CHECK=0 skips it.
    let check: usize = std::env::var("CHECK").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let check = check.min(c.frames.len());
    let mut picture = Picture::decode_only();
    let mut mine = Phase::new(0);
    for (i, f) in c.frames.iter().take(check).enumerate() {
        let encoded = picture.encode(f);
        assert_eq!(encoded.phase_at_origin.get(), mine.get(), "frame {i}: this probe's origin chain has drifted from Picture's");
        mine = geo.next_origin(mine, f.parity);
    }
    if check > 0 {
        println!("the first {check} origins above are Picture's own chain, frame for frame");
    }
}
