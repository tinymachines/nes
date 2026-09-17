//! The knobs file (src/knobs.rs): the shape the bench writes parses and
//! describes itself with its sources; what is not known is refused by
//! name; a fitted knob without its residual is refused; and the one
//! knob the model acts on today reaches the scheduler (a knob that
//! reaches nothing is not a knob). MUTATE=1 feeds the scheduler the
//! same alignment for both runs and the last test must go red.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, pad_program};
use nes_console::{Alignment, Console, Knobs};

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
