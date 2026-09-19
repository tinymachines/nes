//! The split, measured: where down the picture a scrolling game's
//! horizontal motion begins, on the model and on the part, and which of
//! the model's frames the part's triggered frame is.
//!
//!   split-score <rom.nes> [frames]                       synthetic
//!   split-score <rom.nes> [frames] <capture.u8> <rate>   real
//!
//! A game that scrolls under a fixed status bar (a sprite-0 split)
//! writes the level's scroll part way down the frame. Nothing here is
//! told where. The measurement is the horizontal shift of each picture
//! row between two frames GAP apart (default 2): decoded luma, row
//! against row, the lag of the normalised cross-correlation's peak with
//! the peak interpolated, in dots. A row the status bar owns does not
//! move; a row of the level moves by the scroll's advance. Rows with
//! nothing on them (a flat sky) have no answer and are reported flat
//! rather than guessed; a row whose peak is weak is reported unclear.
//! The split sits between the last still row and the first moving row,
//! and that bracket is as tight as the picture's content lets it be.
//!
//! Three measurements, none told another's answer:
//!   1. the model's frame F against F+GAP (F = the picture the part's
//!      recovery hands back for a trigger at LATCH, placed by where the
//!      latch fell against the vertical sync: `Console::
//!      run_to_picture_after_latch`, as capture-score has it);
//!   2. the part's frame at the trigger against the part's frame GAP
//!      later (the record sliced one frame period further on each time,
//!      the recovery taking the first full frame after the slice);
//!   3. the part's triggered frame against the model's F-1, F, F+1 and
//!      F+2: the status bar rows give the constant offset between the
//!      two pictures, and the level rows' shift beyond it is how far
//!      the scroll differs. The model frame where that is zero is the
//!      frame the part drew.
//!   4. the part's triggered frame's whole luma against the model's F-1
//!      to F+2 and F+FAR (default 30) as Pearson's r, blind to a
//!      constant gain or offset, and needing no scroll: at full
//!      resolution (which frame) and at the screen's coarse shape, 30
//!      by 32 blocks (which screen); with the luma's spread on each side
//!      and the part against itself GAP frames on. nes-bench's b3.py
//!      calls a replay's frame the model's by the two correlations.
//!      Recorded on a real record and on the synthesis alike, not held
//!      (see the section).
//!   5. the part's triggered frame against the model's F-2 to F+2 over
//!      only the samples where those five disagree with each other, as
//!      an rms luma error with each candidate's gain and offset fitted
//!      out. Measurement 4 answers which screen; where one sprite moves
//!      it cannot answer which frame, since F-1 and F+1 agree everywhere
//!      but the sprite. Held on the synthesis (the frame found must be
//!      F), recorded on a real record.
//!
//! SCRIPT, LATCH, TRIGGER_SAMPLE and KNOBS as capture-score reads them.
//! Without a record the part's side is synthesised from the model's own
//! frames through the card model and sliced as a trigger would, and the
//! run is HELD: both brackets must agree, the scroll advance must agree
//! within a quarter dot, and the frame found must be F. MUTATE_FRAME=1
//! synthesises the part one frame late and must be red (the frame found
//! is F+1). MUTATE_STILL=1 synthesises the part's pair from one frame
//! twice, so nothing moves, and must be red (no moving row). A real
//! record is recorded, not held, and each of the three is printed.
//!
//! First real run, 2026-09-18 (nes-bench run 20260918-135721, Super
//! Mario Bros. at latch 600 with Right held from 520): model and part
//! both still through row 32 and moving from row 47, the fourteen sky
//! rows between flat; advance 1.89 dots (model) against 1.94 (part)
//! over two frames, 4.30 against 4.34 over four. The part's triggered
//! frame was the model's F+1 under the rule of the day (the next
//! picture after the frame the latch fell in): this game polls at line
//! 251, after the vertical sync's onset, and the recovery hands back
//! the picture after the next sync. The rule now reads the latch's
//! position (`Console::run_to_picture_after_latch`) and this run names
//! F+0; a still picture (E2's title) could not have shown it, a
//! scrolling one does, 1.3 dots at the wrong frame here.

use nes_bus::ACTIVE_DOTS;
use nes_console::{ines, Console, Picture};
use ntsc_grid::CompositeFrame;
use ntsc_source_cap::ingest::{auto_level_nes, read_capture};
use ntsc_source_cap::{capture_model, front_end, recover_nes, Capture};

const WIDTH: usize = 2048;
const ROWS: usize = 240;
const SAMPLES_PER_DOT: f64 = 8.0;
/// The colour subcarrier's cycle on the recovery's grid.
const CYCLE_SAMPLES: usize = 12;
/// The widest shift looked for, in samples: twelve dots, under the
/// sixteen-dot period of a tiled background, so a row of bricks cannot
/// answer with its own repeat.
const MAX_LAG: isize = 96;
const SCOPE_RATE: f64 = 125_000_000.0;
/// The grid the recovery resamples to: twelve samples per subcarrier
/// cycle, 2728 to the line, 262 lines to the frame.
const GRID_RATE: f64 = 12.0 * 315_000_000.0 / 88.0;
const FRAME_GRID_SAMPLES: f64 = 2728.0 * 262.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    /// Nothing on the row to correlate.
    Flat,
    /// Structure, but no clear peak (a sprite crossing, a row that
    /// changed between the frames).
    Unclear,
    /// The second frame's content sits this many dots LEFT of the
    /// first's: the scroll's advance.
    Shift(f64),
}

/// Luma, rows 1..ROWS (the comb wants both neighbours, so row 0 is not
/// decoded), WIDTH samples a row.
fn luma(dec: &ntsc_decode::Decoder, f: &CompositeFrame) -> Vec<f32> {
    dec.decode_yuv(f, 1, ROWS - 1, WIDTH).y
}

fn row_shift(a: &[f32], b: &[f32]) -> Row {
    let m = MAX_LAG as usize;
    let core = &a[m..WIDTH - m];
    let n = core.len() as f64;
    let mean_a = core.iter().map(|v| *v as f64).sum::<f64>() / n;
    let var_a = core.iter().map(|v| (*v as f64 - mean_a).powi(2)).sum::<f64>() / n;
    if var_a.sqrt() < 0.02 {
        return Row::Flat;
    }
    let mut corr = Vec::with_capacity(2 * m + 1);
    for lag in -MAX_LAG..=MAX_LAG {
        // b's content sits `lag` samples left of a's: a[x] ~ b[x - lag].
        let win = &b[(m as isize - lag) as usize..(WIDTH as isize - m as isize - lag) as usize];
        let mean_b = win.iter().map(|v| *v as f64).sum::<f64>() / n;
        let (mut ab, mut bb) = (0.0f64, 0.0f64);
        for (x, y) in core.iter().zip(win) {
            ab += (*x as f64 - mean_a) * (*y as f64 - mean_b);
            bb += (*y as f64 - mean_b).powi(2);
        }
        corr.push(if bb > 0.0 { ab / (var_a * n * bb).sqrt() } else { 0.0 });
    }
    let (i, peak) = corr.iter().enumerate().fold((0, f64::MIN), |acc, (i, c)| if *c > acc.1 { (i, *c) } else { acc });
    if peak < 0.7 || i == 0 || i == corr.len() - 1 {
        return Row::Unclear;
    }
    let (l, c, r) = (corr[i - 1], corr[i], corr[i + 1]);
    let frac = 0.5 * (l - r) / (l - 2.0 * c + r);
    Row::Shift((i as f64 + frac - m as f64) / SAMPLES_PER_DOT)
}

/// Every row of `a` against the same row of `b`. Index 0 is picture row 1.
fn shifts(a: &[f32], b: &[f32]) -> Vec<Row> {
    (0..ROWS - 1).map(|r| row_shift(&a[r * WIDTH..(r + 1) * WIDTH], &b[r * WIDTH..(r + 1) * WIDTH])).collect()
}

/// A box blur one colour subcarrier cycle wide, along each row: twelve
/// samples on this grid (a dot is eight, and the PPU's dot clock is one
/// and a half times the subcarrier), so the subcarrier and anything
/// riding on its phase average away and what is left is the picture's
/// content. Eight samples does NOT do it, which is how this was found:
/// a box a dot wide left 6% of a still frame still separating the
/// candidates by parity.
fn blur_cycle(v: &[f32]) -> Vec<f32> {
    let w = CYCLE_SAMPLES;
    let mut out = vec![0.0f32; v.len()];
    for row in v.chunks_exact(WIDTH).enumerate() {
        let (r, line) = row;
        for x in 0..WIDTH {
            let lo = x.saturating_sub(w / 2);
            let hi = (x + w / 2).min(WIDTH - 1);
            let s: f32 = line[lo..=hi].iter().sum();
            out[r * WIDTH + x] = s / (hi - lo + 1) as f32;
        }
    }
    out
}

/// Pearson's r between two equal-length sample runs.
fn pearson(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len()) as f64;
    let (ma, mb) = (a.iter().map(|&x| x as f64).sum::<f64>() / n, b.iter().map(|&x| x as f64).sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        let (dx, dy) = (x as f64 - ma, y as f64 - mb);
        sab += dx * dy;
        saa += dx * dx;
        sbb += dy * dy;
    }
    sab / (saa * sbb).sqrt()
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

/// What a pair of frames says about the split.
#[derive(Debug)]
struct Split {
    /// The last picture row of the still block at the top.
    last_still: Option<usize>,
    /// The first picture row that moves.
    first_moving: Option<usize>,
    /// The moving rows' median shift, dots.
    advance: Option<f64>,
    still: usize,
    moving: usize,
    flat: usize,
    unclear: usize,
}

/// A row is still within a quarter dot of zero; it moves with the
/// picture within half a dot of the moving rows' median (the peak's
/// interpolation is good to a quarter, and a row at the edge of that
/// tolerance flipped the first moving row by one between a frame and
/// its synthesised copy).
fn split(rows: &[Row]) -> Split {
    const TOL: f64 = 0.25;
    const MOVING: f64 = 0.5;
    let advance = median(rows.iter().filter_map(|r| if let Row::Shift(s) = r { (s.abs() > TOL).then_some(*s) } else { None }).collect());
    let is_still = |r: &Row| matches!(r, Row::Shift(s) if s.abs() <= TOL);
    let is_moving = |r: &Row| matches!((r, advance), (Row::Shift(s), Some(a)) if (s - a).abs() <= MOVING && s.abs() > TOL);
    let first_moving = rows.iter().position(is_moving).map(|i| i + 1);
    let last_still = match first_moving {
        Some(fm) => rows[..fm - 1].iter().rposition(is_still).map(|i| i + 1),
        None => rows.iter().rposition(is_still).map(|i| i + 1),
    };
    Split {
        last_still,
        first_moving,
        advance,
        still: rows.iter().filter(|r| is_still(r)).count(),
        moving: rows.iter().filter(|r| is_moving(r)).count(),
        flat: rows.iter().filter(|r| **r == Row::Flat).count(),
        unclear: rows.len() - rows.iter().filter(|r| is_still(r) || is_moving(r) || **r == Row::Flat).count(),
    }
}

/// The rows as runs: what the eye would check the summary against.
fn print_runs(rows: &[Row]) {
    let key = |r: &Row| match r {
        Row::Flat => "flat".to_string(),
        Row::Unclear => "unclear".to_string(),
        Row::Shift(s) => format!("{:+.2}", (s * 4.0).round() / 4.0 + 0.0),
    };
    let mut start = 0;
    let mut line = String::new();
    for i in 1..=rows.len() {
        if i == rows.len() || key(&rows[i]) != key(&rows[start]) {
            let span = if i - start == 1 { format!("{}", start + 1) } else { format!("{}..{}", start + 1, i) };
            line.push_str(&format!("{span}: {}  ", key(&rows[start])));
            start = i;
        }
    }
    println!("    rows (shift in dots, to the quarter): {}", line.trim_end());
}

fn print_split(what: &str, rows: &[Row]) -> Split {
    let s = split(rows);
    let row = |r: Option<usize>| r.map_or("none".to_string(), |r| r.to_string());
    println!(
        "{what}: last still row {}, first moving row {}, advance {} dots; {} still, {} moving, {} flat, {} unclear of {}",
        row(s.last_still),
        row(s.first_moving),
        s.advance.map_or("none".to_string(), |a| format!("{a:+.2}")),
        s.still,
        s.moving,
        s.flat,
        s.unclear,
        rows.len()
    );
    print_runs(rows);
    s
}

fn recover_at(raw: &Capture, from: usize) -> CompositeFrame {
    assert!(from < raw.samples.len(), "slice at {from} is past the record's {} samples", raw.samples.len());
    let sliced = Capture { declared_rate_hz: raw.declared_rate_hz, samples: raw.samples[from..].to_vec() };
    let rec = recover_nes(&auto_level_nes(&sliced).0);
    println!("  sliced at {from}: recovered {:+.1} ppm, worst burst residual {:.3}, anchor line {}", rec.rate_error_ppm, rec.worst_burst_residual, rec.anchor_line);
    rec.frame
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let ceiling: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let real = args.get(3).map(|p| (p.clone(), args.get(4).and_then(|s| s.parse::<f64>().ok()).expect("<capture> <rate>")));
    let gap: usize = std::env::var("GAP").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let mut c = Console::with_prg_ram(cart, chr_ram, nes_console::knobs::alignment_from_env(), true);
    nes_console::knobs::configure_from_env(&mut c);
    // The bench script's SET and AT lines, as capture-score plays them.
    let latch: u64 = std::env::var("LATCH").ok().and_then(|v| v.parse().ok()).expect("LATCH=<n>: the latch the capture was triggered at");
    let path = std::env::var("SCRIPT").expect("SCRIPT=<bench script>");
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
    let (chosen, pos) = c.run_to_picture_after_latch(latch, ceiling).unwrap_or_else(|e| panic!("{e}"));
    println!("latch {latch} fell at PPU line {} dot {}, {} the vertical sync's onset", pos.line, pos.dot, if nes_console::after_vsync_onset(pos) { "after" } else { "before" });
    // Past F: the pair's second frame, the cross's F+2, and what a
    // synthesised record needs after its last slice.
    // FAR: section 4's contrast frame, F+FAR.
    let far: usize = std::env::var("FAR").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    c.run_frames((gap + 5).max(far + 1));
    assert!(chosen >= 3, "the latch came before the model had drawn three frames");

    // The synthesis: every frame encoded in order, the phase carried;
    // the ones looked at through the card model's front end.
    let mut picture = Picture::decode_only();
    let encoded: Vec<CompositeFrame> = c.frames.iter().map(|f| picture.encode(f)).collect();
    let dec = picture.decoder();
    let model = |i: usize| luma(dec, &front_end(&encoded[i]));
    println!("{}: the model's frame F is its frame {chosen} (the first to complete after latch {latch}); pairs are {gap} frames apart", args[1]);

    // 1. The model alone.
    let m = print_split(&format!("model, F against F+{gap}"), &shifts(&model(chosen), &model(chosen + gap)));

    // 2. The part alone.
    let mutate_frame = std::env::var("MUTATE_FRAME").is_ok_and(|v| v == "1") as usize;
    let mutate_still = std::env::var("MUTATE_STILL").is_ok_and(|v| v == "1");
    let (p0, p1) = match &real {
        Some((path, rate)) => {
            let raw = read_capture(std::path::Path::new(path), "u8", Some(*rate));
            let trig: usize = std::env::var("TRIGGER_SAMPLE").ok().and_then(|v| v.parse().ok()).expect("TRIGGER_SAMPLE=<i>: the trigger's sample in the record");
            let per_frame = rate * FRAME_GRID_SAMPLES / GRID_RATE;
            println!("part: {path} at {rate} Hz, the trigger at sample {trig}, a frame {per_frame:.0} samples");
            (recover_at(&raw, trig), recover_at(&raw, trig + (gap as f64 * per_frame) as usize))
        }
        None => {
            // The record a trigger inside frame F-1 would leave: F-2
            // on, sliced half way through F-1 (past its vertical sync),
            // so the first full frame after the slice is F.
            let first = chosen - 2 + mutate_frame;
            let tail: Vec<&CompositeFrame> = encoded[first..first + gap + 5].iter().collect();
            // SYN_RATE, SYN_PPM, SYN_DC, SYN_NOISE: the synthetic record's
            // stand-in parameters, overridable to find what costs what.
            let knob = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
            let cap = capture_model(&tail, knob("SYN_RATE", SCOPE_RATE), knob("SYN_PPM", 5.0), knob("SYN_DC", 0.020) as f32, knob("SYN_NOISE", 0.002) as f32, 6);
            let per_frame = cap.samples.len() / tail.len();
            let trig = per_frame + per_frame / 2;
            println!("part: synthesised from the model's frames {first}.. through the card model at {SCOPE_RATE} Hz, sliced at {trig} as a trigger would{}", if mutate_frame == 1 { " (MUTATE_FRAME: one frame late)" } else { "" });
            let a = recover_at(&cap, trig);
            let b = if mutate_still { recover_at(&cap, trig) } else { recover_at(&cap, trig + gap * per_frame) };
            (a, b)
        }
    };
    let (p0, p1) = (luma(dec, &p0), luma(dec, &p1));
    let p = print_split(&format!("part, the triggered frame against {gap} later"), &shifts(&p0, &p1));

    // 3. The part's triggered frame against the model's frames around F.
    // The boundary between bar and level is the MODEL's own measurement
    // (1), so the cross is not told where the split is either.
    println!("part's triggered frame against the model's frames (offset = status rows' median; scroll = level rows' median beyond it):");
    let mut found: Option<(isize, f64)> = None;
    let boundary = m.first_moving.unwrap_or(ROWS);
    let bar_end = m.last_still.unwrap_or(0);
    for j in -1isize..=2 {
        let rows = shifts(&model((chosen as isize + j) as usize), &p0);
        let of = |lo: usize, hi: usize| median(rows.iter().enumerate().filter(|(i, _)| (lo..hi).contains(&(i + 1))).filter_map(|(_, r)| if let Row::Shift(s) = r { Some(*s) } else { None }).collect());
        let (bar, level) = (of(1, bar_end + 1), of(boundary, ROWS));
        let fmt = |v: Option<f64>| v.map_or("none".to_string(), |v| format!("{v:+.2}"));
        let beyond = bar.zip(level).map(|(b, l)| l - b);
        println!("  F{j:+}: status rows {} dots, level rows {} dots, the level beyond the bar {} dots", fmt(bar), fmt(level), fmt(beyond));
        if let Some(d) = beyond {
            if found.is_none_or(|(_, best)| d.abs() < best.abs()) {
                found = Some((j, d));
            }
        }
    }
    match found {
        Some((j, d)) => println!("the part's triggered frame is the model's F{j:+} (the level {d:+.2} dots beyond the bar there)"),
        None => println!("no model frame could be compared: the bar or the level had no row with an answer"),
    }

    // 4. The whole picture: the part's triggered frame's luma against
    // the model's frames around F, as a Pearson correlation over every
    // decoded sample, so a constant gain or offset (the part's luma
    // runs a few hundredths low, E2) does not count and a different
    // screen does. F+FAR is the contrast: the same game some frames on.
    // Unlike 3 it needs no scroll, so it answers on a still screen too;
    // nes-bench's b3.py reads it to call a replay's frame the model's.
    // First reading (2026-09-18, the split record): F+0 0.923, F-1..F+2
    // otherwise 0.68 to 0.70, F+30 0.52. The coarse shape then read
    // 0.997 to 0.999 on every true match the bench had (the black world
    // card too, whose small text reads 0.71 at full resolution against
    // every frame, while the part against itself reads 0.999: the two
    // picture chains differ at fine detail, not the frames), 0.72 to
    // 0.78 thirty frames on, 0.66 against a record with Right dropped.
    // The synthetic roundtrip read only 0.77 here, tied between F and
    // F+1, until ntsc-crt v0.2.12: the recovery assumed every frame began
    // at subcarrier origin 0 and slid a frame that began a third of a
    // cycle on by half a dot to make it so (the registration below read
    // 4 samples, r 1.0000 there). It reads 1.0000 at F now, and the part
    // lines up a steady quarter dot over (1 to 3 samples on 8 records
    // from three sessions), recorded, not fitted.
    // DUMP=<prefix>: the part's triggered frame and the model's F as
    // greyscale PGM (decoded luma, WIDTH wide, every row), to look at.
    if let Ok(prefix) = std::env::var("DUMP") {
        let pgm = |v: &[f32], path: String| {
            let rows = v.len() / WIDTH;
            let mut out = format!("P5 {WIDTH} {rows} 255\n").into_bytes();
            out.extend(v.iter().map(|&y| (y.clamp(0.0, 1.0) * 255.0) as u8));
            std::fs::write(&path, out).expect("DUMP");
            println!("wrote {path}");
        };
        pgm(&p0, format!("{prefix}-part.pgm"));
        pgm(&model(chosen), format!("{prefix}-model.pgm"));
    }
    // The registration: the horizontal shift, in decoded samples (eight
    // to a dot), that best lines the part's frame up with the model's F,
    // searched a dot either way. The synthetic roundtrip reads 0 since
    // ntsc-crt v0.2.12 (the recovery measures the frame's subcarrier
    // origin instead of assuming it, and before read 4, half a dot).
    {
        let m = model(chosen);
        let rows = m.len() / WIDTH;
        let mut best = (0isize, -2.0f64);
        for dx in -8isize..=8 {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            for y in 0..rows {
                for x in 16..WIDTH - 16 {
                    a.push(m[y * WIDTH + x]);
                    b.push(p0[y * WIDTH + (x as isize + dx) as usize]);
                }
            }
            let r = pearson(&a, &b);
            if r > best.1 {
                best = (dx, r);
            }
        }
        println!("the registration: the part's frame lines up with the model's F {} samples over ({:+.3} dots), r {:.4} there", best.0, best.0 as f64 / SAMPLES_PER_DOT, best.1);
    }
    let mut corr: Vec<(isize, f64)> = Vec::new();
    for j in [-1isize, 0, 1, 2, far as isize] {
        corr.push((j, pearson(&model((chosen as isize + j) as usize), &p0)));
    }
    // The same at the screen's coarse shape: the luma averaged into
    // blocks of 8 rows by 64 samples (8 dots), 30 by 32 of them, where
    // the two picture chains' differences at fine detail (a card of
    // small text read 0.71 at full resolution against every frame)
    // average away and what is left is which screen it is.
    let blocks = |v: &[f32]| {
        let rows = v.len() / WIDTH;
        let (bh, bw) = (8usize, 64usize);
        let mut out = Vec::new();
        for by in 0..rows / bh {
            for bx in 0..WIDTH / bw {
                let mut acc = 0.0f32;
                for y in by * bh..(by + 1) * bh {
                    acc += v[y * WIDTH + bx * bw..y * WIDTH + (bx + 1) * bw].iter().sum::<f32>();
                }
                out.push(acc / (bh * bw) as f32);
            }
        }
        out
    };
    let pb = blocks(&p0);
    let coarse: Vec<String> = [-1isize, 0, 1, 2, far as isize].iter().map(|&j| format!("F{j:+} {:.4}", pearson(&blocks(&model((chosen as isize + j) as usize)), &pb))).collect();
    println!("the screen's coarse shape, 8-row by 8-dot blocks, part against model (Pearson r): {}", coarse.join("  "));
    let line: Vec<String> = corr.iter().map(|(j, r)| format!("F{j:+} {r:.4}")).collect();
    println!("the whole picture's luma, part against model (Pearson r): {}", line.join("  "));
    // How much there is to correlate: the luma's spread over the frame,
    // the model's F and the part's. A screen near one level (a game's
    // black card) answers every frame alike whatever the part drew.
    let sd = |v: &[f32]| {
        let m = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
        (v.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
    };
    println!("the luma's spread over the frame (standard deviation): model F {:.4}, part {:.4}", sd(&model(chosen)), sd(&p0));
    // The part against itself GAP frames on: on a still screen the
    // ceiling any match can reach through the part's own noise (a
    // noiseless model that matches reads about its square root).
    println!("the part against itself {gap} frames on (Pearson r): {:.4}", pearson(&p0, &p1));

    // 5. Only where the candidates differ from each other. A whole-frame
    // correlation answers "which screen"; on a screen where one sprite
    // moves it cannot answer "which frame", because F-1 and F+1 agree
    // everywhere but the sprite: Duck Hunt's field moves about half a
    // percent of its dots a frame, and 2026-09-19's records read F-1 and
    // F+1 alike (r 0.94) and F worse (0.91), which is the frame's parity
    // speaking, not its content. So: take the samples where the five
    // candidates F-2..F+2 do not agree with each other (the decoded luma
    // spread across them above DIFF, default 0.05, edges left out), fit
    // each candidate's gain and offset to the part over the whole frame
    // (the part's luma runs low, E2), and score each one there as an rms
    // error. The same candidate's error over the samples OUTSIDE the
    // mask is the measurement's own floor: the two picture chains'
    // disagreement on a part of the screen that no candidate disputes.
    // A winner whose error is near that floor while the others sit well
    // above it is the frame the part drew.
    let decide = |label: &str, frames: &[Vec<f32>], p0: &[f32], parts: (Vec<usize>, Vec<usize>)| -> Option<isize> {
        let cands: Vec<isize> = vec![-2, -1, 0, 1, 2];
        let rows = frames[0].len() / WIDTH;
        let (mask, rest) = parts;
        let total = rows * (WIDTH - 48);
        println!(
            "{label}: the candidates F-2..F+2 are told apart on {} samples of {total} ({:.3}% of the frame)",
            mask.len(),
            100.0 * mask.len() as f64 / total as f64
        );
        if mask.is_empty() {
            println!("{label}: nothing separates the candidates, so no frame can be told from its neighbours");
            return None;
        }
        // The part's own registration, fitted where no candidate is in
        // dispute: the shift against the model's F alone would absorb
        // the very difference being measured (a scrolling game's
        // neighbouring frames differ by a shift, and the first version
        // of this measurement was fooled by exactly that: MUTATE_FRAME=1
        // went green). The undisputed samples are the same picture on
        // every candidate, so the shift they give does not prefer one.
        let shift = {
            let f = &frames[2];
            let mut best = (0isize, f64::MAX);
            for dx in -24isize..=24 {
                let acc: f64 = rest.iter().map(|&i| (f[i] as f64 - p0[(i as isize + dx) as usize] as f64).powi(2)).sum();
                let e = (acc / rest.len() as f64).sqrt();
                if e < best.1 {
                    best = (dx, e);
                }
            }
            println!("{label}: aligned on the {} samples no candidate disputes, {} samples over ({:+.3} dots)", rest.len(), best.0, best.0 as f64 / SAMPLES_PER_DOT);
            best.0
        };
        let part = |i: usize| p0[(i as isize + shift) as usize];
        // The gain and offset that best carry a candidate onto the part,
        // least squares over every sample looked at.
        let rms = |f: &[f32], at: &[usize]| {
            let (n, mut sx, mut sy, mut sxx, mut sxy) = (rest.len() + mask.len(), 0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for &i in rest.iter().chain(mask.iter()) {
                let (x, y) = (f[i] as f64, part(i) as f64);
                sx += x;
                sy += y;
                sxx += x * x;
                sxy += x * y;
            }
            let d = n as f64 * sxx - sx * sx;
            let (a, b) = if d.abs() < 1e-12 { (1.0, 0.0) } else { ((n as f64 * sxy - sx * sy) / d, (sy * sxx - sx * sxy) / d) };
            let acc: f64 = at.iter().map(|&i| (a * f[i] as f64 + b - part(i) as f64).powi(2)).sum();
            (acc / at.len() as f64).sqrt()
        };
        // Each candidate at its OWN best alignment, a dot and a half
        // either way. The colour phase turns with the alignment (a
        // third of a subcarrier cycle is four samples on this grid, and
        // a frame's phase step on this part is a third of a cycle too),
        // so a candidate that only wins because the part sits a few
        // samples over is a registration, not a frame.
        let rms_at = |f: &[f32], dx: isize| {
            let part = |i: usize| p0[(i as isize + dx) as usize];
            let (n, mut sx, mut sy, mut sxx, mut sxy) = (rest.len() + mask.len(), 0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for &i in rest.iter().chain(mask.iter()) {
                let (x, y) = (f[i] as f64, part(i) as f64);
                sx += x;
                sy += y;
                sxx += x * x;
                sxy += x * y;
            }
            let d = n as f64 * sxx - sx * sx;
            let (a, b) = if d.abs() < 1e-12 { (1.0, 0.0) } else { ((n as f64 * sxy - sx * sy) / d, (sy * sxx - sx * sxy) / d) };
            let acc: f64 = mask.iter().map(|&i| (a * f[i] as f64 + b - part(i) as f64).powi(2)).sum();
            (acc / mask.len() as f64).sqrt()
        };
        let sweep: Vec<String> = cands
            .iter()
            .zip(frames.iter())
            .map(|(&j, f)| {
                let best = (-12isize..=12).map(|dx| (dx, rms_at(f, dx))).reduce(|a, b| if b.1 < a.1 { b } else { a }).unwrap();
                format!("F{j:+} {:.4} at {:+} samples", best.1, best.0)
            })
            .collect();
        println!("{label}: each candidate at its own best alignment: {}", sweep.join("  "));
        let scored: Vec<(isize, f64, f64)> = cands.iter().zip(frames.iter()).map(|(&j, f)| (j, rms(f, &mask), rms(f, &rest))).collect();
        let line: Vec<String> = scored.iter().map(|(j, e, _)| format!("F{j:+} {e:.4}")).collect();
        println!("{label}: the luma's rms error there, part against model: {}", line.join("  "));
        let best = scored.iter().copied().reduce(|a, b| if b.1 < a.1 { b } else { a }).unwrap();
        let runner = scored.iter().filter(|s| s.0 != best.0).map(|s| s.1).fold(f64::MAX, f64::min);
        println!(
            "{label}: the frame the part drew is F{:+} (rms {:.4} there, the next candidate {:.4}, and the same frame reads {:.4} where no candidate disputes)",
            best.0, best.1, runner, best.2, 
        );
        if (runner - best.1).abs() < 1e-6 {
            println!("{label}: two candidates read the same, so this cannot name one of them");
            return None;
        }
        Some(best.0)
    };
    let cands: Vec<isize> = vec![-2, -1, 0, 1, 2];
    let frames: Vec<Vec<f32>> = cands.iter().map(|&j| model((chosen as isize + j) as usize)).collect();
    let rows = frames[0].len() / WIDTH;
    // Where the candidates differ in the decoded picture by more than
    // DIFF (default 0.05): the samples the phase reading is made on.
    let by_picture = || {
        let diff: f32 = std::env::var("DIFF").ok().and_then(|v| v.parse().ok()).unwrap_or(0.05);
        let (mut mask, mut rest) = (Vec::new(), Vec::new());
        for y in 0..rows {
            for x in 24..WIDTH - 24 {
                let i = y * WIDTH + x;
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for f in &frames {
                    lo = lo.min(f[i]);
                    hi = hi.max(f[i]);
                }
                if hi - lo > diff {
                    mask.push(i);
                } else {
                    rest.push(i);
                }
            }
        }
        (mask, rest)
    };
    // Where the candidates differ in the PPU's own dots: exact, and
    // blind to the colour phase by construction, which the decoded
    // picture is not (a still frame's neighbours differ there on 4% of
    // their samples, and no blur takes it out: the residue at an edge
    // is a transient, not a sinusoid). A disputed dot claims its eight
    // samples and a dot either side, since the decoder spreads an edge;
    // the undisputed side stands five dots clear of any dispute, so the
    // floor it gives is not that spread either.
    let by_dots = || {
        let entries: Vec<Vec<u16>> = cands.iter().map(|&j| c.frames[(chosen as isize + j) as usize].active_entries()).collect();
        let mut disputed = vec![false; rows * WIDTH];
        for r in 0..rows {
            // Luma row r is picture row r + 1 (the comb wants both
            // neighbours, so row 0 is not decoded).
            let row = r + 1;
            for d in 0..ACTIVE_DOTS {
                let v = entries[0][row * ACTIVE_DOTS + d];
                if entries.iter().any(|e| e[row * ACTIVE_DOTS + d] != v) {
                    let lo = (d * SAMPLES_PER_DOT as usize).saturating_sub(SAMPLES_PER_DOT as usize);
                    let hi = ((d + 2) * SAMPLES_PER_DOT as usize).min(WIDTH);
                    for x in lo..hi {
                        disputed[r * WIDTH + x] = true;
                    }
                }
            }
        }
        let clear = 5 * SAMPLES_PER_DOT as usize;
        let (mut mask, mut rest) = (Vec::new(), Vec::new());
        for y in 0..rows {
            for x in 24..WIDTH - 24 {
                let i = y * WIDTH + x;
                if disputed[i] {
                    mask.push(i);
                } else if !disputed[y * WIDTH + x.saturating_sub(clear)..y * WIDTH + (x + clear).min(WIDTH)].iter().any(|&d| d) {
                    rest.push(i);
                }
            }
        }
        (mask, rest)
    };
    // The two readings are of different things, and the still record
    // that raised the question could not tell them apart: at the
    // decoder's own resolution the neighbouring frames differ by the
    // colour subcarrier's phase, which alternates with the PPU's frame
    // parity while a game renders, so F-2, F and F+2 read one number
    // and F-1 and F+1 the other WHATEVER is drawn. Blurred to a dot
    // that phase is gone and only what moved is left.
    println!("5. which frame, two ways: the colour phase (the decoded picture where the candidates differ, which alternates with the PPU's frame parity and so names a parity, not a frame) and what moved (the dots the candidates draw differently, scored on the picture blurred over a subcarrier cycle)");
    let phase = decide("the colour phase", &frames, &p0, by_picture());
    let blurred: Vec<Vec<f32>> = frames.iter().map(|f| blur_cycle(f)).collect();
    let content = decide("what moved", &blurred, &blur_cycle(&p0), by_dots());

    if real.is_none() {
        let mut red = Vec::new();
        if m.first_moving.is_none() {
            red.push("the model shows no moving row: choose a latch where the screen scrolls".to_string());
        }
        if (m.last_still, m.first_moving) != (p.last_still, p.first_moving) {
            red.push(format!("the brackets differ: model {:?}..{:?}, part {:?}..{:?}", m.last_still, m.first_moving, p.last_still, p.first_moving));
        }
        match (m.advance, p.advance) {
            (Some(a), Some(b)) if (a - b).abs() <= 0.25 => {}
            (a, b) => red.push(format!("the advance differs: model {a:?}, part {b:?}")),
        }
        if found.map(|(j, _)| j) != Some(0) {
            red.push(format!("the frame found is {:?}, not F", found.map(|(j, _)| j)));
        }
        if content != Some(0) {
            red.push(format!("the frame found where the candidates' content differs is {content:?}, not F"));
        }
        if phase != Some(0) {
            red.push(format!("the frame found by the colour phase is {phase:?}, not F"));
        }
        if red.is_empty() {
            println!("the synthetic roundtrip holds: same bracket, same advance, the frame found is F by the scroll and where the candidates disagree");
        } else {
            for r in &red {
                println!("RED: {r}");
            }
            std::process::exit(1);
        }
    } else {
        println!("a real record is recorded, not held");
    }
}
