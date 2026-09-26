//! Record a run from a pad script, the way the page records one, then
//! replay it with the trace on, the way the flow tools do: the native
//! twin of the browser's path, for measuring it and for the roof's
//! flow checks on a real cartridge.
//!
//!     cargo run --release -p nes-wasm --example record-replay -- \
//!         ROM FRAMES SCRIPT OUT
//!
//! SCRIPT is `frame:pad,frame:pad,...`, the pad byte (A=1, B=2, Select=4,
//! Start=8, Up=16, Down=32, Left=64, Right=128) set before that frame.
//! Writes OUT.log (the recording) and OUT.trace (the replay's trace),
//! and says how long each took. The ROM is read, never copied.

use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("usage: record-replay ROM FRAMES SCRIPT OUT");
        std::process::exit(2);
    }
    let rom = std::fs::read(&a[1]).expect("the ROM");
    let frames: usize = a[2].parse().expect("FRAMES");
    let mut script: Vec<(usize, u8)> = a[3]
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (f, p) = s.split_once(':').expect("frame:pad");
            (f.parse().unwrap(), p.parse().unwrap())
        })
        .collect();
    script.sort();

    let t = Instant::now();
    let mut m = nes_wasm::Machine::new(&rom).expect("the ROM loads");
    m.record_start().unwrap();
    for f in 0..frames {
        for &(_, p) in script.iter().filter(|&&(at, _)| at == f) {
            m.set_pad(p);
        }
        m.run_frames(1);
    }
    let log = m.record_stop();
    let live = t.elapsed();

    let t = Instant::now();
    let mut r = nes_wasm::Replayer::new(&rom, &[], &log).expect("the log");
    let mut trace = Vec::new();
    loop {
        let ended = r.run(60).unwrap_or_else(|e| panic!("{e}"));
        trace.extend(r.take_trace());
        if ended {
            break;
        }
    }
    let replay = t.elapsed();
    std::fs::write(format!("{}.log", a[4]), &log).unwrap();
    std::fs::write(format!("{}.trace", a[4]), &trace).unwrap();
    let secs = frames as f64 / 60.0988;
    println!(
        "{frames} frames ({secs:.1} s of play): recorded in {:.2} s ({:.2}x real time), log {} bytes; replayed with the trace in {:.2} s ({:.2}x), {} of {} pictures matched, trace {} MB",
        live.as_secs_f64(),
        secs / live.as_secs_f64(),
        log.len(),
        replay.as_secs_f64(),
        secs / replay.as_secs_f64(),
        r.frames_checked(),
        r.frames(),
        trace.len() / 1_000_000
    );
}
