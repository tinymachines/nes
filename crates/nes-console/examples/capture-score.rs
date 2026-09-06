//! N6 step 2: the capture path, machine half. Run a ROM in the console,
//! encode its frames the way the picture does, and score named flat
//! regions of a capture against the console's own synthesis through the
//! identical decoder (Rung C): luma, hue and saturation per region.
//!
//!   capture-score <rom.nes> [frames]                      synthetic
//!   capture-score <rom.nes> [frames] <capture.u8> <rate>   real
//!
//! Synthetic: the console's last three frames through ntsc-crt's
//! capture-card model at the scope's rate (125 MHz, 5 ppm off, 2 mV of
//! noise), recovered as the real path is (`auto_level_nes`, then
//! `recover_nes`). Real: the file the M4 tools read (u8 samples at the
//! declared rate), the same recovery. Regions are found in the console's
//! own frame: the largest flat rectangle of every distinct (colour,
//! emphasis) at least 12 dots wide and 6 rows tall, scored one dot in
//! from each edge.
//!
//! Tolerances (docs/n6-plan.md, stated before measuring): synthetic,
//! luma within 0.01, hue within 1.0 degree where the synthesis has a
//! hue (saturation above 0.02; a grey has none), saturation within 5
//! percent of the synthesis or 0.005 absolute, whichever is larger; a
//! miss on any region exits 1. Real: recorded, not held; a miss beyond
//! the synthetic tolerance is named.

use nes_bus::{DotFrame, ACTIVE_DOTS, ACTIVE_ROWS};
use nes_console::{ines, Alignment, Console, Picture};
use ntsc_grid::CompositeFrame;
use ntsc_source_cap::ingest::{auto_level_nes, read_capture};
use ntsc_source_cap::{capture_model, recover_nes, Capture};

const WIDTH: usize = 2048;
const SAMPLES_PER_DOT: usize = WIDTH / ACTIVE_DOTS;
const SCOPE_RATE: f64 = 125_000_000.0;

#[derive(Clone, Copy, Debug)]
struct Region {
    colour: u8,
    emphasis: u8,
    row0: usize,
    row1: usize,
    x0: usize,
    x1: usize,
}

/// The largest flat rectangle per distinct (colour, emphasis): runs of
/// one value at least 8 dots long on each row, merged over consecutive
/// rows where the runs overlap by 8 dots or more (the extent kept is
/// the intersection: a bar's edges may step from row to row, as
/// full_palette's do), at least 6 rows and 12 dots in the end.
fn regions(f: &DotFrame) -> Vec<Region> {
    let mut best: std::collections::BTreeMap<(u8, u8), Region> = Default::default();
    let mut open: Vec<Region> = Vec::new();
    for row in 0..ACTIVE_ROWS {
        let mut runs = Vec::new();
        let mut x = 0;
        while x < ACTIVE_DOTS {
            let (c, e) = f.at(row, x + 1);
            let mut x1 = x + 1;
            while x1 < ACTIVE_DOTS && f.at(row, x1 + 1) == (c, e) {
                x1 += 1;
            }
            if x1 - x >= 8 {
                runs.push(Region { colour: c, emphasis: e, row0: row, row1: row + 1, x0: x, x1 });
            }
            x = x1;
        }
        let mut next = Vec::new();
        let mut used = vec![false; open.len()];
        for r in runs {
            let hit = open.iter().enumerate().find(|(i, o)| {
                !used[*i] && (o.colour, o.emphasis) == (r.colour, r.emphasis) && o.x1.min(r.x1) >= o.x0.max(r.x0) + 8
            });
            if let Some((i, o)) = hit {
                used[i] = true;
                next.push(Region { row1: row + 1, x0: o.x0.max(r.x0), x1: o.x1.min(r.x1), ..*o });
            } else {
                next.push(r);
            }
        }
        for (i, o) in open.iter().enumerate() {
            if !used[i] {
                consider(&mut best, *o);
            }
        }
        open = next;
    }
    for o in &open {
        consider(&mut best, *o);
    }
    best.into_values().collect()
}

fn consider(best: &mut std::collections::BTreeMap<(u8, u8), Region>, r: Region) {
    if r.row1 - r.row0 < 6 || r.x1 - r.x0 < 12 {
        return;
    }
    let area = |r: &Region| (r.row1 - r.row0) * (r.x1 - r.x0);
    let e = best.entry((r.colour, r.emphasis)).or_insert(r);
    if area(&r) > area(e) {
        *e = r;
    }
}

struct Score {
    y: f64,
    sat: f64,
    hue: f64,
}

fn score(dec: &ntsc_decode::Decoder, frame: &CompositeFrame, r: &Region) -> Score {
    // The comb wants both neighbours: row 0 is not decoded.
    let row0 = r.row0.max(1);
    let yuv = dec.decode_yuv(frame, row0, r.row1 - row0, WIDTH);
    let (s0, s1) = ((r.x0 + 1) * SAMPLES_PER_DOT, (r.x1 - 1) * SAMPLES_PER_DOT);
    let (mut my, mut mu, mut mv, mut n) = (0.0f64, 0.0, 0.0, 0.0);
    for row in 0..r.row1 - row0 {
        for x in s0..s1 {
            let i = row * WIDTH + x;
            my += yuv.y[i] as f64;
            mu += yuv.u[i] as f64;
            mv += yuv.v[i] as f64;
            n += 1.0;
        }
    }
    let (y, u, v) = (my / n, mu / n, mv / n);
    Score { y, sat: (u * u + v * v).sqrt(), hue: v.atan2(u).to_degrees() }
}

fn hue_delta(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let real = args.get(3).map(|p| (p.clone(), args.get(4).and_then(|s| s.parse::<f64>().ok()).expect("<capture> <rate>")));
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.nrom().expect("NROM");
    let mut c = Console::with_prg_ram(Box::new(cart), chr_ram, Alignment::default(), true);
    c.run_frames(frames);

    // The synthesis: every frame encoded in order, the phase carried.
    let mut picture = Picture::decode_only();
    let encoded: Vec<CompositeFrame> = c.frames.iter().map(|f| picture.encode(f)).collect();
    let last = c.frames.last().unwrap();
    let synth = encoded.last().unwrap();
    let regions = regions(last);
    println!("{} frames of {}; {} flat regions of distinct (colour, emphasis) in the last frame ({:?})", frames, args[1], regions.len(), last.parity);

    let (cap, what): (Capture, String) = match &real {
        Some((path, rate)) => {
            let raw = read_capture(std::path::Path::new(path), "u8", Some(*rate));
            (auto_level_nes(&raw).0, format!("real capture {path} at {rate} Hz"))
        }
        None => {
            // The model's knobs, for finding which stage a miss belongs
            // to: SYNTH_RATE (Hz), SYNTH_PPM, SYNTH_DC (volts), SYNTH_NOISE
            // (volts), SYNTH_LEVEL=0 skips the re-referencing.
            let knob = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
            let (ppm, dc, noise) = (knob("SYNTH_PPM", 5.0), knob("SYNTH_DC", 0.020), knob("SYNTH_NOISE", 0.002));
            let rate = knob("SYNTH_RATE", SCOPE_RATE);
            let level = std::env::var("SYNTH_LEVEL").map(|v| v != "0").unwrap_or(true);
            let tail: Vec<&CompositeFrame> = encoded.iter().rev().take(3).rev().collect();
            let model = capture_model(&tail, rate, ppm, dc as f32, noise as f32, 6);
            let cap = if level { auto_level_nes(&model).0 } else { model };
            (cap, format!("synthetic capture of the last {} frames at {rate} Hz, {ppm:+} ppm, {} mV DC, {} mV noise, re-referenced: {level}", tail.len(), dc * 1000.0, noise * 1000.0))
        }
    };
    let rec = recover_nes(&cap);
    println!("{what}: recovered {:+.1} ppm, worst burst residual {:.3}, anchor line {}", rec.rate_error_ppm, rec.worst_burst_residual, rec.anchor_line);

    let dec = picture.decoder();
    let (tol_y, tol_hue, tol_sat_rel, tol_sat_abs, hue_floor): (f64, f64, f64, f64, f64) = (0.01, 1.0, 0.05, 0.005, 0.02);
    let mut misses = 0;
    println!("{:<5} {:<3} {:<9} {:<9} | {:<8} {:<8} {:<8} | {:<8} {:<8} {:<8} | {:<8} {:<8} {:<8}", "col", "emp", "rows", "x", "Y syn", "sat syn", "hue syn", "Y cap", "sat cap", "hue cap", "dY", "dsat", "dhue");
    for r in &regions {
        let s = score(dec, synth, r);
        let k = score(dec, &rec.frame, r);
        let dy = k.y - s.y;
        let dsat = k.sat - s.sat;
        let dhue = if s.sat > hue_floor { hue_delta(k.hue, s.hue) } else { 0.0 };
        let sat_tol = tol_sat_abs.max(tol_sat_rel * s.sat);
        let miss = dy.abs() > tol_y || dhue.abs() > tol_hue || dsat.abs() > sat_tol;
        if miss {
            misses += 1;
        }
        let hue_s = if s.sat > hue_floor { format!("{:+.1}", s.hue) } else { "grey".into() };
        let hue_k = if s.sat > hue_floor { format!("{:+.1}", k.hue) } else { "grey".into() };
        println!(
            "${:02x}   {}   {:>3}..{:<3}  {:>3}..{:<3}  | {:<+8.4} {:<8.4} {:<8} | {:<+8.4} {:<8.4} {:<8} | {:<+8.4} {:<+8.4} {:<+8.1}{}",
            r.colour, r.emphasis, r.row0, r.row1, r.x0, r.x1, s.y, s.sat, hue_s, k.y, k.sat, hue_k, dy, dsat, dhue, if miss { "  MISS" } else { "" }
        );
    }
    let held = real.is_none();
    println!(
        "{} of {} regions within luma {tol_y}, hue {tol_hue} deg, saturation {}% (or {tol_sat_abs}); {}",
        regions.len() - misses,
        regions.len(),
        tol_sat_rel * 100.0,
        if held { "the synthetic roundtrip is held to that" } else { "the real capture is recorded, not held" }
    );
    if held && misses > 0 {
        std::process::exit(1);
    }
}
