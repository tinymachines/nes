//! Run an iNES ROM for N frames and write the last frame as a PPM in the
//! PPU's palette indices mapped through an authored NES palette (for a
//! look, not a measurement: the family's real path is ntsc-crt, N6).
//!
//!   cargo run --release -p nes-console --example run-rom -- game.nes 120 out.ppm

use std::io::Write as _;

use nes_console::{ines, Alignment, Console};

/// An authored RGB approximation of the 2C02's 64 colours (the common
/// "2C02G" table). For looking at frames only.
const PALETTE: [u32; 64] = [
    0x626262, 0x001fb2, 0x2404c8, 0x5200b2, 0x730076, 0x800024, 0x730b00, 0x522800, 0x244400, 0x005700, 0x005c00, 0x005324, 0x003c76, 0x000000, 0x000000, 0x000000,
    0xababab, 0x0d57ff, 0x4b30ff, 0x8a13ff, 0xbc08d6, 0xd21269, 0xc72e00, 0x9d5400, 0x607b00, 0x209800, 0x00a300, 0x009942, 0x007db4, 0x000000, 0x000000, 0x000000,
    0xffffff, 0x53aeff, 0x9085ff, 0xd365ff, 0xff57ff, 0xff5dcf, 0xff7757, 0xfa9e00, 0xbdc700, 0x7ae700, 0x43f011, 0x26e6a6, 0x2ccaff, 0x4e4e4e, 0x000000, 0x000000,
    0xffffff, 0xb6e1ff, 0xced1ff, 0xe9c3ff, 0xffbcff, 0xffbdf4, 0xffc6c3, 0xffd59a, 0xe9e681, 0xcef481, 0xb6fb9a, 0xa9fac3, 0xa9f0f4, 0xb8b8b8, 0x000000, 0x000000,
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let out = args.get(3).cloned().unwrap_or_else(|| "frame.ppm".into());
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.nrom().expect("NROM");
    // ALIGN=cpu,ppu picks a power-on alignment other than the measured
    // one (the sketch's set: the dividers can start in any).
    let alignment = match std::env::var("ALIGN") {
        Ok(v) => {
            let (c, p) = v.split_once(',').expect("ALIGN=cpu,ppu");
            Alignment { cpu_phase: c.parse().unwrap(), ppu_phase: p.parse().unwrap() }
        }
        Err(_) => Alignment::default(),
    };
    let mut c = Console::with_prg_ram(Box::new(cart), chr_ram, alignment, true);
    let t = std::time::Instant::now();
    c.run_frames(frames);
    let dt = t.elapsed().as_secs_f64();
    let f = c.frames.last().unwrap();
    let mut ppm = Vec::new();
    write!(ppm, "P6\n256 240\n255\n").unwrap();
    for row in 0..240 {
        for d in 1..=256 {
            let (idx, _) = f.at(row, d);
            let rgb = PALETTE[idx as usize & 63];
            ppm.extend_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
        }
    }
    std::fs::write(&out, ppm).unwrap();
    // blargg's reporting window: $6000 the result, $6001..$6003 the
    // magic DE B0 61 while a test runs, $6004.. the text.
    {
        let b = c.board.borrow();
        if let Some(ram) = &b.prg_ram {
            if ram[1..4] == [0xde, 0xb0, 0x61] {
                let text: String = ram[4..].iter().take_while(|&&x| x != 0).map(|&x| x as char).collect();
                println!("$6000 result: {:02x}{}; text: {:?}", ram[0], if ram[0] == 0x80 { " (still running)" } else { "" }, text.trim());
            }
        }
    }
    if let Ok(list) = std::env::var("DUMP_CHR") {
        let b = c.board.borrow();
        let cart = b.cart.borrow();
        for t in list.split(',') {
            let t = u16::from_str_radix(t, 16).unwrap();
            let bytes: Vec<String> = (0..16).map(|i| {
                let a = t * 16 + i;
                let v = match &cart.chr_ram { Some(r) => r[a as usize], None => 0 };
                format!("{v:02x}")
            }).collect();
            println!("tile {t:02x}: {}", bytes.join(" "));
        }
    }
    println!(
        "{frames} frames in {dt:.2} s ({:.1} frames/s, {:.2}x real time); {} CPU half-cycles, {} reads, {} writes; wrote {out}",
        frames as f64 / dt,
        frames as f64 / dt / 60.0988,
        c.cpu_half_cycles,
        c.board.borrow().reads,
        c.board.borrow().writes
    );
}
