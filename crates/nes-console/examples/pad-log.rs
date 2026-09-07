//! The model's side of the bench's B0: the controller port's poll log,
//! one line per latch, in the form the bridge streams from the part:
//!
//!     L <latch index> <byte the register held> <clocks the poll took>
//!
//! so `nes-bench/tools/compare-logs.py` diffs the two. The byte is in
//! the register's order (bit 0 = A, set = pressed); the clocks are the
//! reads of $4016 between this latch's fall and the previous one's,
//! eight for a game's poll, nine where a DMC fetch clocked the pad
//! twice (the die's rule, `2a03`'s joy-clock-probe, the rung's
//! tests/joypad.rs). The pad is set from PAD=hh (the byte from the
//! start) and a bench script (nes-bench/docs/script.md): its `SET hh`
//! and `AT <latch> <hh>` lines are honoured, by latch index, exactly as
//! the bridge honours them; the other words are the head's and are
//! skipped. A summary line closes: polls, the histogram of clocks per
//! poll, and the latch indices that took nine.
//!
//!   cargo run --release -p nes-console --example pad-log -- rom.nes [frames] [script]

use nes_console::{ines, Alignment, Console};
use nes_glue::controller::Buttons;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let mut pad = std::env::var("PAD").ok().and_then(|v| u8::from_str_radix(&v, 16).ok()).unwrap_or(0);
    let mut schedule: Vec<(u64, u8)> = Vec::new();
    if let Some(path) = args.get(3) {
        for line in std::fs::read_to_string(path).expect("script").lines() {
            let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
            match f.as_slice() {
                ["AT", n, b] => schedule.push((n.parse().expect("latch"), u8::from_str_radix(b, 16).expect("hex byte"))),
                ["SET", b] => pad = u8::from_str_radix(b, 16).expect("hex byte"),
                _ => {}
            }
        }
    }
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.nrom().expect("NROM");
    let mut c = Console::with_prg_ram(Box::new(cart), chr_ram, Alignment::default(), true);
    {
        let mut b = c.board.borrow_mut();
        b.pads[0].log_polls = true;
        b.pads[0].schedule = schedule;
    }
    c.set_pad(0, Buttons::from_byte(pad));
    let mut printed = 0usize;
    for _ in 0..frames {
        c.run_frames(1);
        let b = c.board.borrow();
        let polls = &b.pads[0].polls;
        // Latch i closes poll i-1: its line carries the byte latch i-1
        // loaded and the reads between the two latches. The reads before
        // the first latch belong to no poll.
        for i in polls.len().min(printed.max(1))..polls.len() {
            println!("L {} {:02x} {}", i - 1, polls[i - 1].0, polls[i].1);
        }
        printed = polls.len();
    }
    let b = c.board.borrow();
    let polls: Vec<u32> = b.pads[0].polls.iter().skip(1).map(|&(_, c)| c).collect();
    let mut hist = std::collections::BTreeMap::new();
    for &k in &polls {
        *hist.entry(k).or_insert(0u32) += 1;
    }
    let nines: Vec<usize> = polls.iter().enumerate().filter(|(_, &k)| k == 9).map(|(i, _)| i).collect();
    println!("# {} polls over {frames} frames; clocks per poll {hist:?}; nine at latches {nines:?}", polls.len());
}
