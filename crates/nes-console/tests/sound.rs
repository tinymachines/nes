//! N7: the sound. Three things hold. The NES-001 stage does the
//! schematic's arithmetic (a step decays with (R7 || R8) C23, a tone is
//! attenuated by R6 C21's first-order amount, the inverter inverts).
//! blargg's four apu_mixer ROMs, each a channel and the DMC's inverse
//! between two reference beeps, cancel through the whole path: in every
//! 100 ms window of the test section the output's RMS is at most 5
//! percent of the beep's (the noise ROM fades noise by design and is
//! held on its beeps and on carrying no tone). `MUTATE_SOUND=1` mixes
//! through the wiki's linear approximation and the square or dmc ROM
//! must go red (its own variable, because the 2A03 rung reads MUTATE
//! itself to swap a fitted APU phase, and a mutated APU never plays the
//! shell's beeps: the first MUTATE=1 here went red on the beep finder,
//! not the mixer). And blargg's real-hardware recordings of the same ROMs, decoded
//! with ffmpeg, are measured the same way and printed beside, recorded
//! and not held.
//!
//! The ROMs are read from the nes-test-roms checkout (NES_TEST_ROMS, or
//! ~/roms/nes-test-roms); the ROM tests SKIP by name without it and
//! the recordings without ffmpeg.

use nes_console::sound::{gain, hp_tau, lp_tau, OUT_RATE, RATE_DEN, RATE_NUM};
use nes_console::{ines, Alignment, Console, Mixer, Sound};
use std::path::PathBuf;

fn roms() -> Option<PathBuf> {
    let base = std::env::var("NES_TEST_ROMS").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("roms").join("nes-test-roms")
    });
    base.join("apu_mixer").join("square.nes").is_file().then_some(base)
}

/// The beep's tone: square 0 at period $6F, sixteen CPU cycles per
/// sequencer step, eight steps.
fn beep_hz() -> f64 {
    RATE_NUM as f64 / RATE_DEN as f64 / 2.0 / (0x70 * 16) as f64
}

/// Per 10 ms window: RMS and the beep tone's amplitude (Goertzel).
fn windows(x: &[f32], ms: usize) -> Vec<(f64, f64)> {
    let n = OUT_RATE as usize * ms / 1000;
    let w = 2.0 * std::f64::consts::PI * beep_hz() / OUT_RATE as f64;
    let (cw, sw) = (w.cos(), w.sin());
    x.chunks_exact(n)
        .map(|c| {
            let rms = (c.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
            let (mut re, mut im) = (0.0f64, 0.0f64);
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for &v in c {
                re += v as f64 * cr;
                im += v as f64 * ci;
                let (ncr, nci) = (cr * cw - ci * sw, cr * sw + ci * cw);
                cr = ncr;
                ci = nci;
            }
            let tone = 2.0 * (re * re + im * im).sqrt() / n as f64;
            (rms, tone)
        })
        .collect()
}

struct Split {
    /// The beep's RMS and tone amplitude (mean over its interior).
    beep_rms: f64,
    beep_tone: f64,
    /// The test section, in 10 ms windows.
    test: std::ops::Range<usize>,
    beeps: [(usize, usize); 2],
}

/// The shell's shape: a beep is 300 ms of the tone with 300 ms of
/// silence either side, so the beeps are the runs of the tone between
/// 250 and 400 ms long with the 250 ms before and after them quiet,
/// the first two found after the first second (the power-on settling
/// is before). The test section is what lies between them, less the
/// silences and 50 ms of margin.
fn beeps(w: &[(f64, f64)], from: usize) -> Vec<(usize, usize)> {
    let peak = w[from..].iter().map(|x| x.1).fold(0.0, f64::max);
    let mut runs = Vec::new();
    let mut i = from;
    while i < w.len() {
        if w[i].1 > peak * 0.2 {
            let s = i;
            while i < w.len() && w[i].1 > peak * 0.2 {
                i += 1;
            }
            let level = w[s..i].iter().map(|x| x.1).sum::<f64>() / (i - s) as f64;
            // Quiet over what exists of the 250 ms either side (a
            // recording may start or end inside them), the 30 ms
            // nearest the run left to its edges' decay.
            let quiet = |r: std::ops::Range<usize>| w[r.start.max(from)..r.end.min(w.len())].iter().all(|x| x.1 < 0.1 * level);
            if (25..=40).contains(&(i - s)) && quiet(s.saturating_sub(25)..s.saturating_sub(3)) && quiet(i + 3..i + 25) {
                runs.push((s, i));
            }
        } else {
            i += 1;
        }
    }
    runs
}

fn split_from(w: &[(f64, f64)], from: usize) -> Split {
    let runs = beeps(w, from);
    assert!(runs.len() >= 2, "two beeps expected, {} found", runs.len());
    let (b1, b2) = (runs[0], runs[1]);
    let interior = |r: (usize, usize)| &w[r.0 + 3..r.1 - 3];
    let mean = |v: &[(f64, f64)], f: fn(&(f64, f64)) -> f64| v.iter().map(f).sum::<f64>() / v.len() as f64;
    let beep_rms = (mean(interior(b1), |x| x.0) + mean(interior(b2), |x| x.0)) / 2.0;
    let beep_tone = (mean(interior(b1), |x| x.1) + mean(interior(b2), |x| x.1)) / 2.0;
    Split { beep_rms, beep_tone, test: b1.1 + 35..b2.0 - 35, beeps: [b1, b2] }
}

fn split(w: &[(f64, f64)]) -> Split {
    split_from(w, 100.min(w.len()))
}

/// The worst 100 ms window of the test section: RMS and tone, each as
/// a fraction of the beep's.
fn worst(x: &[f32], sp: &Split) -> (f64, f64) {
    let per = OUT_RATE as usize / 100;
    let section = &x[sp.test.start * per..sp.test.end * per];
    let ws = windows(section, 100);
    let rms = ws.iter().map(|w| w.0).fold(0.0, f64::max) / sp.beep_rms;
    let tone = ws.iter().map(|w| w.1).fold(0.0, f64::max) / sp.beep_tone;
    (rms, tone)
}

fn run(rom: &std::path::Path, mixer: Mixer, frames: usize) -> Vec<f32> {
    let bytes = std::fs::read(rom).unwrap();
    let r = ines::parse(&bytes).expect("iNES");
    let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = r.nrom().expect("NROM");
    let mut c = Console::with_prg_ram(Box::new(cart), chr_ram, Alignment::default(), true);
    c.sound = Some(Sound::new(mixer));
    c.run_frames(frames);
    let s = c.sound.take().unwrap();
    assert_eq!(s.samples, c.cpu_half_cycles, "one code sample per CPU half-cycle");
    let expect = s.samples as f64 * OUT_RATE as f64 * RATE_DEN as f64 / RATE_NUM as f64;
    assert!((s.out.len() as f64 - expect).abs() < 800.0, "{} output samples for {} input, expected about {expect:.0} (the resampler's half-width behind)", s.out.len(), s.samples);
    s.out
}

#[test]
fn the_stage_does_the_schematics_arithmetic() {
    let dt = RATE_DEN as f64 / RATE_NUM as f64;
    // A step: from 0 to 1 after 50 ms of silence, then 100 ms held.
    let mut s = Sound::default();
    let n0 = (0.05 / dt) as usize;
    for i in 0..(0.15 / dt) as usize {
        s.push_level(if i < n0 { 0.0 } else { 1.0 });
    }
    let at = |t: f64| s.out[((0.05 + t) * OUT_RATE as f64) as usize] as f64;
    // The inverter: a rising step goes negative; and it decays with
    // the high-pass time constant, e^-1 per tau.
    let (a, b) = (at(hp_tau()), at(2.0 * hp_tau()));
    assert!(a < 0.0, "a positive step comes out inverted: {a}");
    let ratio = b / a;
    assert!((ratio - (-1.0f64).exp()).abs() < 0.02, "one tau on: {ratio:.4} of the level, expected e^-1 (tau {:.2} ms)", hp_tau() * 1e3);
    // And its size is the gain: at 0.5 ms the low-pass (10 us) has
    // settled and the high-pass (7.5 ms) has barely begun.
    let early = at(0.0005);
    assert!((early / gain() - (-0.0005 / hp_tau()).exp()).abs() < 0.01, "the step's size against the gain: {early:.4} for gain {:.3}", gain());
    // Two tones: 200 Hz and 10 kHz, the ratio of their outputs is the
    // first-order high-pass times low-pass at those frequencies.
    let h = |f: f64| {
        let (wh, wl) = (2.0 * std::f64::consts::PI * f * hp_tau(), 2.0 * std::f64::consts::PI * f * lp_tau());
        wh / (1.0 + wh * wh).sqrt() / (1.0 + wl * wl).sqrt()
    };
    let rms_of = |f: f64| {
        let mut s = Sound::default();
        for i in 0..(0.2 / dt) as usize {
            s.push_level((2.0 * std::f64::consts::PI * f * i as f64 * dt).sin());
        }
        let tail = &s.out[s.out.len() - OUT_RATE as usize / 20..];
        (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt()
    };
    let measured = rms_of(10_000.0) / rms_of(200.0);
    let expected = h(10_000.0) / h(200.0);
    assert!((measured / expected - 1.0).abs() < 0.01, "10 kHz over 200 Hz: {measured:.4} measured, {expected:.4} from the schematic's values");
    eprintln!("stage: tau_hp {:.2} ms, tau_lp {:.2} us, gain {:.3}; step e^-1 {ratio:.4}, 10 kHz/200 Hz {measured:.4} vs {expected:.4}", hp_tau() * 1e3, lp_tau() * 1e6, gain());
}

#[test]
fn blarggs_mixer_roms_cancel_through_the_console() {
    let Some(base) = roms() else {
        eprintln!("SKIP: no nes-test-roms checkout (NES_TEST_ROMS)");
        return;
    };
    let mutate = std::env::var("MUTATE_SOUND").is_ok_and(|v| v == "1");
    let tol = 0.05;
    let mut reds = Vec::new();
    for name in ["square", "triangle", "noise", "dmc"] {
        if mutate && !(name == "square" || name == "dmc") {
            continue;
        }
        let mixer = if mutate { Mixer::Linear } else { Mixer::Table };
        let out = run(&base.join("apu_mixer").join(format!("{name}.nes")), mixer, 1200);
        let w = windows(&out, 10);
        let sp = split(&w);
        let (rms, tone) = worst(&out, &sp);
        // The beep itself is a real signal: above 20 percent of the
        // table's full pulse level through the gain.
        let full = (v2a03_dac::ad1(15, 15) as f64 * gain()).abs();
        assert!(sp.beep_rms > 0.2 * full, "{name}: the beep's RMS {:.4} is under a fifth of full pulse {full:.4}", sp.beep_rms);
        let held = if name == "noise" { tone <= tol } else { rms <= tol };
        eprintln!(
            "{name} ({:?}): beeps at {:.2}..{:.2} s and {:.2}..{:.2} s, RMS {:.4}; worst 100 ms window of the test: RMS {:.1}% of the beep, tone {:.1}%{}",
            mixer, sp.beeps[0].0 as f64 / 100.0, sp.beeps[0].1 as f64 / 100.0, sp.beeps[1].0 as f64 / 100.0, sp.beeps[1].1 as f64 / 100.0,
            sp.beep_rms, rms * 100.0, tone * 100.0, if held { "" } else { "  MISS" }
        );
        if !held {
            reds.push(name);
        }
    }
    if mutate {
        assert!(!reds.is_empty(), "MUTATE_SOUND=1: the linear approximation cancelled as well as the table");
        panic!("MUTATE_SOUND=1: red on {reds:?} (this panic is the red)");
    }
    assert!(reds.is_empty(), "the residual exceeds {}% of the beep on {reds:?}", tol * 100.0);
}

#[test]
fn blarggs_recordings_measured_the_same_way() {
    let Some(base) = roms() else {
        eprintln!("SKIP: no nes-test-roms checkout (NES_TEST_ROMS)");
        return;
    };
    for name in ["square", "triangle", "noise", "dmc"] {
        let mp3 = base.join("apu_mixer_recordings").join(format!("{name}.mp3"));
        let Ok(dec) = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-i"])
            .arg(&mp3)
            .args(["-ac", "1", "-ar", &OUT_RATE.to_string(), "-f", "s16le", "-"])
            .output()
        else {
            eprintln!("SKIP: ffmpeg not available");
            return;
        };
        if !dec.status.success() {
            eprintln!("SKIP: ffmpeg could not decode {}", mp3.display());
            return;
        }
        let x: Vec<f32> = dec.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0).collect();
        let w = windows(&x, 10);
        // The recordings start at the first beep: no first second to skip.
        let sp = split_from(&w, 0);
        let (b1, b2) = (sp.beeps[0], sp.beeps[1]);
        let (rms, tone) = worst(&x, &sp);
        eprintln!(
            "{name} (blargg's recording, real hardware): beeps {:.2}..{:.2} s and {:.2}..{:.2} s, RMS {:.4} of the file's full scale; worst 100 ms window: RMS {:.1}% of the beep, tone {:.1}% (recorded, not held)",
            b1.0 as f64 / 100.0, b1.1 as f64 / 100.0, b2.0 as f64 / 100.0, b2.1 as f64 / 100.0, sp.beep_rms, rms * 100.0, tone * 100.0
        );
    }
}


