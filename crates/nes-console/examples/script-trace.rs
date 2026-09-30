//! A scripted run of a cartridge with the trace on, written to a file
//! for the flow tools (`public/wasm/flow`, `examples/report`): the same
//! records the page's replay writes (`record.rs`), from power-on.
//!
//!   cargo run --release -p nes-console --example script-trace -- ROM.nes SCRIPT FRAMES OUT.trace
//!
//! SCRIPT is lines of `AT <frame> <hh>`: from that frame the pad holds
//! the byte (the register's order, bit 0 = A, set = pressed) until the
//! next line; other lines are skipped by name, like the bench runners.
//! The trace is about 19 MB a second of play, so it is written out as
//! it goes rather than held. A commercial cartridge's trace is its
//! bytes in another arrangement and goes where the ROM store is.

use std::io::Write;

use nes_console::{ines, Alignment, Console};
use nes_glue::controller::Buttons;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("usage: script-trace ROM.nes SCRIPT FRAMES OUT.trace");
        std::process::exit(2);
    }
    let rom = std::fs::read(&a[1]).expect("the ROM");
    let script = std::fs::read_to_string(&a[2]).expect("the script");
    let frames: usize = a[3].parse().expect("FRAMES");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&a[4]).expect("the output"));
    let mut at: Vec<(usize, u8)> = Vec::new();
    for line in script.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        if w.len() == 3 && w[0] == "AT" {
            at.push((w[1].parse().expect("a frame index"), u8::from_str_radix(w[2], 16).expect("a hex byte")));
        }
    }
    at.sort();
    let r = ines::parse(&rom).unwrap_or_else(|e| panic!("{e:?}"));
    let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = r.cart().unwrap_or_else(|e| panic!("{e:?}"));
    let mut console = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
    console.trace = Some(Default::default());
    let mut next = 0;
    let mut pad = 0u8;
    let mut bytes = 0usize;
    let t = std::time::Instant::now();
    for f in 0..frames {
        while next < at.len() && at[next].0 <= f {
            pad = at[next].1;
            next += 1;
        }
        console.set_pad(0, Buttons::from_byte(pad));
        console.run_frames(1);
        if let Some(tr) = console.trace.as_mut() {
            out.write_all(&tr.bytes).expect("write");
            bytes += tr.bytes.len();
            tr.bytes.clear();
        }
    }
    out.flush().expect("flush");
    eprintln!("{frames} frames in {:.2} s, {} MB of trace, {} pad changes honoured", t.elapsed().as_secs_f64(), bytes / 1_000_000, next);
}
