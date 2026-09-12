//! Run an iNES ROM for N frames and write the last frame as a PPM in the
//! PPU's palette indices mapped through an authored NES palette (for a
//! look, not a measurement: the family's real path is ntsc-crt, N6).
//!
//!   cargo run --release -p nes-console --example run-rom -- game.nes 120 out.ppm
//!
//! WAV=out.wav writes the sound: the APU through the DACs, the NES-001
//! stage and the resampler, 48 kHz 16-bit mono at a fixed listening
//! level. CRT=out.ppm writes the picture instead: every frame through
//! `Picture` (ntsc-crt's NES source, Rung C, the CRT stages), the last
//! one displayed. DECODED=out.ppm writes the last decoded grid (2048 x
//! 240, no CRT) beside it.

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
    let cart = rom.cart().expect("a cartridge this console has");
    // ALIGN=cpu,ppu picks a power-on alignment other than the measured
    // one (the sketch's set: the dividers can start in any).
    let alignment = match std::env::var("ALIGN") {
        Ok(v) => {
            let (c, p) = v.split_once(',').expect("ALIGN=cpu,ppu");
            Alignment { cpu_phase: c.parse().unwrap(), ppu_phase: p.parse().unwrap() }
        }
        Err(_) => Alignment::default(),
    };
    let mut c = Console::with_prg_ram(cart, chr_ram, alignment, true);
    let wav_out = std::env::var("WAV").ok();
    if wav_out.is_some() {
        c.sound = Some(nes_console::Sound::default());
    }
    let t = std::time::Instant::now();
    c.run_frames(frames);
    let dt = t.elapsed().as_secs_f64();
    if let (Some(path), Some(s)) = (&wav_out, &c.sound) {
        std::fs::write(path, s.wav(0.25)).unwrap();
        let peak = s.out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        println!("sound: {} code samples at {}/{} Hz, {} output samples at {} Hz, peak {peak:.4} table units x gain; wrote {path}", s.samples, nes_console::sound::RATE_NUM, nes_console::sound::RATE_DEN, s.out.len(), nes_console::sound::OUT_RATE);
    }
    let crt_out = std::env::var("CRT").ok();
    let decoded_out = std::env::var("DECODED").ok();
    if crt_out.is_some() || decoded_out.is_some() {
        let t = std::time::Instant::now();
        let mut picture = if crt_out.is_some() { nes_console::Picture::new() } else { nes_console::Picture::decode_only() };
        let mut last = None;
        for f in &c.frames {
            last = Some(picture.push(f));
        }
        let shown = last.expect("no frames");
        let dt = t.elapsed().as_secs_f64();
        if let Some(path) = &crt_out {
            let d = shown.displayed.as_ref().unwrap();
            std::fs::write(path, nes_console::picture::display_ppm(d)).unwrap();
            println!("picture: {} frames through Rung C and the CRT stages in {dt:.2} s ({:.1} frames/s), phase left {}; wrote {path} ({}x{})", c.frames.len(), c.frames.len() as f64 / dt, picture.origin().get(), d.width, d.height);
        }
        if let Some(path) = &decoded_out {
            std::fs::write(path, nes_console::picture::decoded_ppm(&shown.decoded)).unwrap();
            println!("decoded grid: wrote {path} ({}x{})", shown.decoded.width, shown.decoded.height);
        }
    }
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
