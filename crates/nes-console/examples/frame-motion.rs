//! How much of the picture moves, frame by frame, under a bench
//! script's inputs: the probe that picks the latch for a capture that
//! has to tell one frame from its neighbour.
//!
//!   cargo run --release -p nes-console --example frame-motion -- rom.nes [frames]
//!
//! SCRIPT=<bench script> gives the console the same SET and AT lines the
//! bridge gets (capture-score reads them the same way, and the latch
//! numbering is the same: from power-on, the script's RESET skipped).
//! For every frame it prints how many of the 256 x 240 active dots
//! differ from the frame before, and the rows they fall between.
//!
//! Why it exists (2026-09-19): Duck Hunt's field scored against the
//! model's frames F-1 and F+1 equally well and against F worse
//! (nes-bench open-items). A still picture cannot tell an odd
//! neighbour from the other one, so the capture has to be taken where
//! something moves. This says where that is without a capture and
//! without a look, and its number is the one a frame-against-frame
//! correlation has to work with.
//!
//! LATCH=<n> also prints which picture that latch lands on, by the
//! console's own rule, so a frame index here is a latch for the bench.
//! SHOW=<dir> writes each frame in a MARK=a-b range decoded, for a look.

use nes_bus::{ACTIVE_DOTS, ACTIVE_ROWS};
use nes_console::{ines, Console, Picture};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let mut c = Console::with_prg_ram(cart, chr_ram, nes_console::knobs::alignment_from_env(), true);
    nes_console::knobs::configure_from_env(&mut c);
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
    if let Ok(t) = std::env::var("LATCH") {
        let t: u64 = t.parse().expect("LATCH");
        let (target, pos) = c.run_to_picture_after_latch(t, frames).unwrap_or_else(|e| panic!("{e}"));
        println!("latch {t} fell at PPU line {} dot {}: the part's frame is picture {target}", pos.line, pos.dot);
    }
    if c.frames.len() < frames {
        c.run_frames(frames - c.frames.len());
    }

    let mark: Option<(usize, usize)> = std::env::var("MARK").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    let total = ACTIVE_DOTS * ACTIVE_ROWS;
    println!("frame  changed  of {total}  rows");
    for i in 1..c.frames.len() {
        let a = c.frames[i - 1].active_entries();
        let b = c.frames[i].active_entries();
        let mut changed = 0usize;
        let (mut first, mut last) = (usize::MAX, 0usize);
        for row in 0..ACTIVE_ROWS {
            let mut any = false;
            for dot in 0..ACTIVE_DOTS {
                if a[row * ACTIVE_DOTS + dot] != b[row * ACTIVE_DOTS + dot] {
                    changed += 1;
                    any = true;
                }
            }
            if any {
                first = first.min(row);
                last = row;
            }
        }
        let rows = if first == usize::MAX { "none".to_string() } else { format!("{first}..{last}") };
        println!("{i:5}  {changed:7}  {:6.3}%  {rows}", 100.0 * changed as f64 / total as f64);
    }
    if let (Ok(dir), Some((a, b))) = (std::env::var("SHOW"), mark) {
        std::fs::create_dir_all(&dir).expect("SHOW dir");
        let mut p = Picture::decode_only();
        for (i, f) in c.frames.iter().enumerate() {
            let shown = p.push(f);
            if i >= a && i <= b {
                let path = format!("{dir}/frame-{i:04}.ppm");
                std::fs::write(&path, nes_console::picture::decoded_ppm(&shown.decoded)).expect("SHOW");
                println!("wrote {path}");
            }
        }
    }
}
