//! The knobs file (src/knobs.rs): the shape the bench writes parses and
//! describes itself with its sources; what is not known is refused by
//! name; a fitted knob without its residual is refused; and the one
//! knob the model acts on today reaches the scheduler (a knob that
//! reaches nothing is not a knob). MUTATE=1 feeds the scheduler the
//! same alignment for both runs and the last test must go red. The
//! warmth reaches the encoded picture and only the picture;
//! MUTATE_WARMTH=1 leaves the frame unscaled and must go red.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, pad_program};
use nes_console::{Alignment, Console, Knobs, Picture};

const FILE: &str = r#"
# runs/20260918-000521/knobs.toml
[alignment]
cpu_phase = 4
ppu_phase = 3
source = "measured"
by = "v2a03-sim clk-phase, v2c02-sim clk-phase"   # the dies' own recipes

[capture]
scale_v_per_div = 0.2
offset_v = -0.5
channel = 3
source = "measured"
by = "20260918-000521"
"#;

#[test]
fn the_benchs_file_parses_and_names_its_sources() {
    let k = Knobs::parse(FILE, "knobs.toml").unwrap();
    assert_eq!(k.alignment(), Alignment { cpu_phase: 4, ppu_phase: 3 });
    let c = k.capture.as_ref().unwrap();
    assert_eq!((c.channel, c.scale_v_per_div, c.offset_v), (3, 0.2, -0.5));
    let d = k.describe();
    assert!(d.contains("measured by v2a03-sim clk-phase"), "{d}");
    assert!(d.contains("capture CH3 at 0.2 V/div, offset -0.5 V, measured by 20260918-000521"), "{d}");
}

#[test]
fn an_empty_file_is_the_defaults() {
    let k = Knobs::parse("# nothing set\n", "empty.toml").unwrap();
    assert_eq!(k.alignment(), Alignment::default());
    assert!(k.capture.is_none());
}

#[test]
fn what_is_not_known_is_refused_by_name() {
    let refused = |text: &str| Knobs::parse(text, "k.toml").unwrap_err();
    // A key the reader does not know: a typo cannot run the defaults.
    let e = refused("[alignment]\ncpu_phase = 4\nppu_phaze = 3\nsource = \"measured\"\nby = \"x\"\n");
    assert!(e.contains("`ppu_phaze`") && e.contains("cpu_phase, ppu_phase"), "{e}");
    // A table it does not know.
    let e = refused("[reset]\nhold = 1\nsource = \"authored\"\nby = \"x\"\n");
    assert!(e.contains("[reset]"), "{e}");
    // No source, no by, a source that is not one of the three.
    let e = refused("[alignment]\ncpu_phase = 4\nppu_phase = 3\nby = \"x\"\n");
    assert!(e.contains("no `source`"), "{e}");
    let e = refused("[alignment]\ncpu_phase = 4\nppu_phase = 3\nsource = \"measured\"\n");
    assert!(e.contains("no `by`"), "{e}");
    let e = refused("[alignment]\ncpu_phase = 4\nppu_phase = 3\nsource = \"guessed\"\nby = \"x\"\n");
    assert!(e.contains("\"guessed\""), "{e}");
    // Out of range, the wrong type, a key before any table, a duplicate.
    let e = refused("[alignment]\ncpu_phase = 24\nppu_phase = 3\nsource = \"measured\"\nby = \"x\"\n");
    assert!(e.contains("not in 0..24"), "{e}");
    let e = refused("[alignment]\ncpu_phase = \"4\"\nppu_phase = 3\nsource = \"measured\"\nby = \"x\"\n");
    assert!(e.contains("a string, not an integer"), "{e}");
    let e = refused("cpu_phase = 4\n");
    assert!(e.contains("before any [table]"), "{e}");
    let e = refused("[capture]\nchannel = 3\nchannel = 1\n");
    assert!(e.contains("`channel` twice"), "{e}");
}

#[test]
fn a_fitted_knob_carries_its_residual() {
    let base = "[capture]\nscale_v_per_div = 0.2\noffset_v = -0.5\nchannel = 3\n";
    let e = Knobs::parse(&format!("{base}source = \"fitted\"\nby = \"20260918-000521\"\n"), "k.toml").unwrap_err();
    assert!(e.contains("fitted and carries no `residual`"), "{e}");
    let k = Knobs::parse(&format!("{base}source = \"fitted\"\nby = \"20260918-000521\"\nresidual = 0.009\n"), "k.toml").unwrap();
    assert!(k.describe().contains("fitted by 20260918-000521 (residual 0.009)"), "{}", k.describe());
    let e = Knobs::parse(&format!("{base}source = \"measured\"\nby = \"x\"\nresidual = 0.1\n"), "k.toml").unwrap_err();
    assert!(e.contains("not fitted"), "{e}");
}

fn cpu_half_cycles_after_a_frame(a: Alignment) -> u64 {
    let cart = Nrom::new(pad_program(false), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, a);
    c.run_frames(1);
    c.cpu_half_cycles
}

#[test]
fn the_alignment_knob_reaches_the_scheduler() {
    let file = |cpu: u8| format!("[alignment]\ncpu_phase = {cpu}\nppu_phase = 3\nsource = \"measured\"\nby = \"test\"\n");
    let a = Knobs::parse(&file(4), "a.toml").unwrap().alignment();
    let mut b = Knobs::parse(&file(16), "b.toml").unwrap().alignment();
    if std::env::var("MUTATE").is_ok_and(|v| v == "1") {
        b = a;
    }
    // A phi1 twelve master half-steps later is one CPU half-cycle fewer
    // by the time the first frame completes: the knob moved the schedule.
    let (ha, hb) = (cpu_half_cycles_after_a_frame(a), cpu_half_cycles_after_a_frame(b));
    assert_ne!(ha, hb, "the alignment knob changed nothing the scheduler counts ({ha} CPU half-cycles either way)");
}

#[test]
fn the_ram_fill_knob_reaches_the_board() {
    let k = Knobs::parse("[ram]\nfill = 255\nsource = \"authored\"\nby = \"the bench's cold-boot finding\"\n", "k.toml").unwrap();
    assert!(k.describe().contains("ram fill ff at power-on, authored by"), "{}", k.describe());
    let cart = Nrom::new(pad_program(false), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    if std::env::var("MUTATE").is_ok_and(|v| v == "1") {
        // The knob applied to a console the board never sees.
        let cart2 = Nrom::new(pad_program(false), chr(), Mirroring::Vertical).unwrap();
        let mut other = Console::new(Box::new(cart2), None, Alignment::default());
        k.apply(&mut other);
    } else {
        k.apply(&mut c);
    }
    let b = c.board.borrow();
    assert_eq!((b.wram.read(0x0000), b.wram.read(0x07ff)), (0xff, 0xff), "the fill did not reach the work RAM");
    let e = Knobs::parse("[ram]\nfill = 256\nsource = \"authored\"\nby = \"x\"\n", "k.toml").unwrap_err();
    assert!(e.contains("not a byte"), "{e}");
}

const WARM: &str = "[warmth]\nseconds_on = 2708\nsource = \"measured\"\nby = \"20260918-203018 head.log\"\n\n[warmth_curve]\ndepth = 0.0214\ntau_s = 1050\nsource = \"fitted\"\nby = \"warm-up series 20260918-194516..203018\"\nresidual = 0.001\n";

#[test]
fn the_warmth_is_the_seconds_on_the_curve() {
    let k = Knobs::parse(WARM, "k.toml").unwrap();
    let g = k.warmth_gain().unwrap();
    let want = 1.0 - 0.0214 * (1.0 - (-2708.0f64 / 1050.0).exp());
    assert!((g - want).abs() < 1e-12, "{g} against {want}");
    assert!(k.describe().contains("warmth 2708 s on, measured by 20260918-203018 head.log"), "{}", k.describe());
    assert!(k.describe().contains("fitted by warm-up series 20260918-194516..203018 (residual 0.001)"), "{}", k.describe());
    // Cold is gain one; the plateau is one less the depth.
    let c = k.warmth_curve.as_ref().unwrap();
    assert_eq!(c.gain(0.0), 1.0);
    assert!((c.gain(1e9) - (1.0 - 0.0214)).abs() < 1e-12);
    // The seconds without the curve say nothing, and are refused.
    let e = Knobs::parse("[warmth]\nseconds_on = 5\nsource = \"measured\"\nby = \"x\"\n", "k.toml").unwrap_err();
    assert!(e.contains("without [warmth_curve]"), "{e}");
    // The curve alone is kept and acts on nothing.
    let only = WARM.split("[warmth_curve]").nth(1).unwrap();
    let k = Knobs::parse(&format!("[warmth_curve]{only}"), "k.toml").unwrap();
    assert!(k.warmth_curve.is_some() && k.warmth_gain().is_none());
    let e = Knobs::parse("[warmth_curve]\ndepth = 0.02\ntau_s = 0\nsource = \"authored\"\nby = \"x\"\n", "k.toml").unwrap_err();
    assert!(e.contains("not positive"), "{e}");
}

#[test]
fn the_warmth_knob_reaches_the_picture_and_only_the_picture() {
    let k = Knobs::parse(WARM, "k.toml").unwrap();
    let g = k.warmth_gain().unwrap() as f32;
    let cart = Nrom::new(pad_program(false), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.run_frames(2);
    let mut picture = Picture::decode_only();
    let cold = picture.encode(c.frames.last().unwrap());
    let mut warm = cold.clone();
    if !std::env::var("MUTATE_WARMTH").is_ok_and(|v| v == "1") {
        k.apply_warmth(&mut warm);
    }
    let blank = ntsc_source_nes::levels::BLANK;
    let (mut moved, mut picture_samples) = (0usize, 0usize);
    for (a, b) in cold.lines.iter().zip(&warm.lines) {
        // The sync and the burst are the levels the scorer reads: untouched.
        assert_eq!(a.samples[..a.active_start], b.samples[..b.active_start], "the warmth reached the sync or the burst");
        for (x, y) in a.samples[a.active_start..].iter().zip(&b.samples[b.active_start..]) {
            if (x - blank).abs() > 0.05 && *x > blank {
                picture_samples += 1;
                assert!(((y - blank) - (x - blank) * g).abs() < 1e-5, "a picture sample {x} came out {y}, not scaled by {g} about blanking");
                moved += (x != y) as usize;
            }
        }
    }
    assert!(picture_samples > 1000, "the frame has a picture to scale ({picture_samples} samples above blanking)");
    assert!(moved > 0, "the warmth knob moved nothing in the picture (MUTATE_WARMTH=1 leaves it cold: red)");
}
