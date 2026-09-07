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
//! emphasis) at least 6 rows tall and wide enough to be scored a
//! settling distance in from each edge, that distance derived from the
//! decoder's own chroma filter (`margin_dots`).
//!
//! Both sides carry the capture's band limit: the synthesis goes through
//! the card model's front end (`ntsc_source_cap::front_end`, its
//! anti-alias lowpass alone) before the decoder, since 2026-09-06's
//! procedure decision (docs/n6-report.md): a decoder fed the raw
//! square-wave synthesis sees harmonics a card never passes, and the
//! first run's hue and saturation misses were that. SYNTH_RAW=1 scores
//! against the raw synthesis instead, the way the first run did.
//!
//! The bench's B1 (nes-bench/docs/bench-plan.md) scores a triggered
//! capture against the model's frame at the same poll: SCRIPT=<bench
//! script> gives the console the same SET and AT lines the bridge got,
//! LATCH=<n> runs the console to the first frame that completes after
//! latch n (a game polls in vblank, before the vertical sync, so the
//! picture after that sync is the first one drawn from the input at
//! latch n), and TRIGGER_SAMPLE=<i> slices the real record from the
//! trigger's sample on, so the recovery's first full frame is the same
//! frame on the part; the recovery needs two full frames after the
//! slice, so the head places the trigger early in the record. Without
//! a real record, SYNTH_TRIGGER=1 synthesises six frames, scores the
//! fourth, and slices from inside the third, as a trigger placed there
//! would, holding the roundtrip to the tolerances: the tool's own green
//! run before any capture exists. MUTATE_TRIGGER=1 slices one frame
//! late and must be red across the bars cartridge's luma-row step.
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
use ntsc_source_cap::{capture_model, front_end, recover_nes, Capture};

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
fn regions(f: &DotFrame, margin: usize) -> Vec<Region> {
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
                consider(&mut best, *o, margin);
            }
        }
        open = next;
    }
    for o in &open {
        consider(&mut best, *o, margin);
    }
    best.into_values().collect()
}

/// How far inside a region's edge the decoder's chroma has settled, in
/// dots, from the decoder's own filter: half the decimated lowpass's
/// span plus the two decimated samples the interpolation reaches, rounded
/// up. DERIVED from the instance, not chosen: the first run scored one
/// dot in from the edges of sixteen-dot bars and read the filter's
/// transitions as a chroma residual (docs/n6-report.md).
fn margin_dots(dec: &ntsc_decode::Decoder) -> usize {
    (dec.uv_taps.len() * dec.uv_decimation / 2 + 2 * dec.uv_decimation).div_ceil(SAMPLES_PER_DOT)
}

fn consider(best: &mut std::collections::BTreeMap<(u8, u8), Region>, r: Region, margin: usize) {
    if r.row1 - r.row0 < 6 || r.x1 - r.x0 < 2 * margin + 10 {
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
    let margin = margin_dots(dec);
    let (s0, s1) = ((r.x0 + margin) * SAMPLES_PER_DOT, (r.x1 - margin) * SAMPLES_PER_DOT);
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
    // The bench script's SET and AT lines, and the latch that picks the
    // frame; the frame count is a ceiling when LATCH is given.
    let latch: Option<u64> = std::env::var("LATCH").ok().and_then(|v| v.parse().ok());
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
    let synth_trigger = std::env::var("SYNTH_TRIGGER").is_ok_and(|v| v == "1");
    let frames = match latch {
        Some(t) => {
            let mut ran = 0usize;
            while c.board.borrow().pads[0].latches <= t && ran < frames {
                c.run_frames(1);
                ran += 1;
            }
            assert!(c.board.borrow().pads[0].latches > t, "latch {t} was not reached in {frames} frames (the game polls {} times in them)", c.board.borrow().pads[0].latches);
            // The first frame that completes after the latch.
            c.run_frames(1);
            ran + 1
        }
        None => {
            c.run_frames(frames);
            frames
        }
    };
    if synth_trigger {
        // Two frames past the chosen one, so the sliced synthesis has two
        // full frames after its trigger, as the head's record will, with
        // one to spare for the late-trigger mutation.
        c.run_frames(2);
    }

    // The synthesis: every frame encoded in order, the phase carried.
    let mut picture = Picture::decode_only();
    let encoded: Vec<CompositeFrame> = c.frames.iter().map(|f| picture.encode(f)).collect();
    let chosen = c.frames.len() - if synth_trigger { 3 } else { 1 };
    let last = &c.frames[chosen];
    let raw = std::env::var("SYNTH_RAW").is_ok_and(|v| v == "1");
    let synth = if raw { encoded[chosen].clone() } else { front_end(&encoded[chosen]) };
    let synth = &synth;
    let margin = margin_dots(picture.decoder());
    let regions = regions(last, margin);
    println!(
        "{} frames of {}; {} flat regions of distinct (colour, emphasis) in the last frame ({:?}), scored {margin} dots in from their edges (the decoder's chroma settling); the synthesis {}",
        frames, args[1], regions.len(), last.parity, if raw { "raw" } else { "through the card model's front end" }
    );

    let trigger_sample: Option<usize> = std::env::var("TRIGGER_SAMPLE").ok().and_then(|v| v.parse().ok());
    let (cap, what): (Capture, String) = match &real {
        Some((path, rate)) => {
            let mut raw = read_capture(std::path::Path::new(path), "u8", Some(*rate));
            let sliced = match trigger_sample {
                Some(i) => {
                    assert!(i < raw.samples.len(), "TRIGGER_SAMPLE {i} is past the record's {} samples", raw.samples.len());
                    raw.samples.drain(..i);
                    format!(", from the trigger's sample {i} on")
                }
                None => String::new(),
            };
            (auto_level_nes(&raw).0, format!("real capture {path} at {rate} Hz{sliced}"))
        }
        None => {
            // The model's knobs, for finding which stage a miss belongs
            // to: SYNTH_RATE (Hz), SYNTH_PPM, SYNTH_DC (volts), SYNTH_NOISE
            // (volts), SYNTH_LEVEL=0 skips the re-referencing.
            let knob = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
            let (ppm, dc, noise) = (knob("SYNTH_PPM", 5.0), knob("SYNTH_DC", 0.020), knob("SYNTH_NOISE", 0.002));
            let rate = knob("SYNTH_RATE", SCOPE_RATE);
            let level = std::env::var("SYNTH_LEVEL").map(|v| v != "0").unwrap_or(true);
            let tail: Vec<&CompositeFrame> = encoded.iter().rev().take(if synth_trigger { 6 } else { 3 }).rev().collect();
            let mut model = capture_model(&tail, rate, ppm, dc as f32, noise as f32, 6);
            // SYNTH_OUT=path writes the synthesised record before any
            // slice, with its rate and the trigger's sample beside it,
            // which is what the bench's fake scope serves as a capture.
            if let Ok(path) = std::env::var("SYNTH_OUT") {
                let per_frame = model.samples.len() / tail.len();
                let trig = if synth_trigger { per_frame * 2 + per_frame / 2 } else { 0 };
                // Volts onto the byte the way the scope's 200 mV/div window does
                // (about 200 levels over the signal's 1.1 V; a window that left it
                // 64 levels anchored the recovery on the wrong line, found here).
                let bytes: Vec<u8> = model.samples.iter().map(|&v| ((v + 0.1) / 1.4 * 255.0).round().clamp(0.0, 255.0) as u8).collect();
                std::fs::write(&path, &bytes).expect("SYNTH_OUT");
                std::fs::write(format!("{path}.toml"), format!("file = \"{}\"\nformat = \"u8\"\nrate_hz = {rate:.1}\ntrigger_sample = {trig}\n", std::path::Path::new(&path).file_name().unwrap().to_string_lossy())).expect("SYNTH_OUT toml");
                println!("wrote the synthesised record to {path}: {} samples, trigger at {trig}", bytes.len());
            }
            let mut sliced = String::new();
            if synth_trigger {
                // The trigger as the bridge places it: inside the frame
                // before the chosen one (the fourth of six), past its
                // vertical sync, so the first full frame after the slice
                // is the chosen frame and not the one before it.
                // MUTATE_TRIGGER=1 slices one frame late, inside the
                // chosen frame itself, so the recovery lands on the frame
                // after it: across the bars cartridge's luma-row step
                // that must be red, which is what proves the frame
                // selection is being checked and not just the colours.
                let per_frame = model.samples.len() / tail.len();
                let late = std::env::var("MUTATE_TRIGGER").is_ok_and(|v| v == "1") as usize;
                let i = per_frame * (2 + late) + per_frame / 2;
                model.samples.drain(..i);
                sliced = format!(", sliced from sample {i} as a trigger would{}", if late == 1 { " (MUTATE_TRIGGER: one frame late)" } else { "" });
            }
            let cap = if level { auto_level_nes(&model).0 } else { model };
            (cap, format!("synthetic capture of the last {} frames at {rate} Hz, {ppm:+} ppm, {} mV DC, {} mV noise, re-referenced: {level}{sliced}", tail.len(), dc * 1000.0, noise * 1000.0))
        }
    };
    let rec = recover_nes(&cap);
    println!("{what}: recovered {:+.1} ppm, worst burst residual {:.3}, anchor line {}", rec.rate_error_ppm, rec.worst_burst_residual, rec.anchor_line);

    let dec = picture.decoder();
    let (tol_y, tol_hue, tol_sat_rel, tol_sat_abs, hue_floor): (f64, f64, f64, f64, f64) = (0.01, 1.0, 0.05, 0.005, 0.02);
    let mut misses = 0;
    let (mut worst_y, mut worst_hue, mut worst_sat, mut worst_chroma, mut luma_ok, mut hue_ok, mut sat_ok, mut with_hue) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize, 0usize, 0usize, 0usize);
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
        luma_ok += (dy.abs() <= tol_y) as usize;
        sat_ok += (dsat.abs() <= sat_tol) as usize;
        if s.sat > hue_floor {
            with_hue += 1;
            hue_ok += (dhue.abs() <= tol_hue) as usize;
        }
        let (su, sv) = (s.sat * s.hue.to_radians().cos(), s.sat * s.hue.to_radians().sin());
        let (ku, kv) = (k.sat * k.hue.to_radians().cos(), k.sat * k.hue.to_radians().sin());
        worst_y = worst_y.max(dy.abs());
        worst_hue = worst_hue.max(dhue.abs());
        worst_sat = worst_sat.max(dsat.abs());
        worst_chroma = worst_chroma.max(((ku - su).powi(2) + (kv - sv).powi(2)).sqrt());
        let hue_s = if s.sat > hue_floor { format!("{:+.1}", s.hue) } else { "grey".into() };
        let hue_k = if s.sat > hue_floor { format!("{:+.1}", k.hue) } else { "grey".into() };
        println!(
            "${:02x}   {}   {:>3}..{:<3}  {:>3}..{:<3}  | {:<+8.4} {:<8.4} {:<8} | {:<+8.4} {:<8.4} {:<8} | {:<+8.4} {:<+8.4} {:<+8.1}{}",
            r.colour, r.emphasis, r.row0, r.row1, r.x0, r.x1, s.y, s.sat, hue_s, k.y, k.sat, hue_k, dy, dsat, dhue, if miss { "  MISS" } else { "" }
        );
    }
    println!(
        "within: luma {luma_ok} of {}, hue {hue_ok} of {with_hue} with a hue, saturation {sat_ok} of {}; worst: luma {worst_y:.4}, hue {worst_hue:.1} deg, saturation {worst_sat:.4}, chroma vector {worst_chroma:.4}",
        regions.len(),
        regions.len()
    );
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
