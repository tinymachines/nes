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
//!   1. the model's frame F against F+GAP (F = the first frame that
//!      completes after LATCH, as capture-score has it);
//!   2. the part's frame at the trigger against the part's frame GAP
//!      later (the record sliced one frame period further on each time,
//!      the recovery taking the first full frame after the slice);
//!   3. the part's triggered frame against the model's F-1, F, F+1 and
//!      F+2: the status bar rows give the constant offset between the
//!      two pictures, and the level rows' shift beyond it is how far
//!      the scroll differs. The model frame where that is zero is the
//!      frame the part drew.
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
//! frame was the model's F+1, not F: this game polls at line 251,
//! AFTER the encoder's sync rows (245..247), and the recovery hands
//! back the first full frame after the next sync, which is the picture
//! after the first one drawn from the latch. capture-score's LATCH
//! convention assumes a poll before the sync and is one picture late
//! on this game; a still picture (E2's title) cannot show it, a
//! scrolling one does, 1.3 dots at F+0 here.

use nes_console::{ines, Console, Picture};
use ntsc_grid::CompositeFrame;
use ntsc_source_cap::ingest::{auto_level_nes, read_capture};
use ntsc_source_cap::{capture_model, front_end, recover_nes, Capture};

const WIDTH: usize = 2048;
const ROWS: usize = 240;
const SAMPLES_PER_DOT: f64 = 8.0;
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
    let mut ran = 0usize;
    while c.board.borrow().pads[0].latches <= latch && ran < ceiling {
        c.run_frames(1);
        ran += 1;
    }
    assert!(c.board.borrow().pads[0].latches > latch, "latch {latch} was not reached in {ceiling} frames");
    c.run_frames(1);
    let chosen = c.frames.len() - 1;
    // Past F: the pair's second frame, the cross's F+2, and what a
    // synthesised record needs after its last slice.
    c.run_frames(gap + 5);
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
            let cap = capture_model(&tail, SCOPE_RATE, 5.0, 0.020, 0.002, 6);
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
        if red.is_empty() {
            println!("the synthetic roundtrip holds: same bracket, same advance, the frame found is F");
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
