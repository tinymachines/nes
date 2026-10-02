//! A script played from power-on, kept as what a reader of a game needs
//! beside its trace: the console's own memory after every frame, and the
//! picture of the frames asked for.
//!
//!   cargo run --release -p nes-console --example storyboard -- \
//!       ROM.nes SCRIPT FRAMES OUT_DIR [FRAME,FRAME,...]
//!
//! SCRIPT is `script-trace`'s (`AT <frame> <pad byte in hex>` lines), so
//! the same file gives the trace, and a frame number means the same
//! thing in both. OUT_DIR gets `ram.bin` (2048 bytes a frame, $0000 to
//! $07FF as the frame ended), `pad.bin` (the pad byte held during each
//! frame) and `frame-NNNNN.ppm` for each frame listed: the PPU's colour
//! indices through an authored palette, for looking at, as `run-rom`
//! writes them.
//!
//! A commercial cartridge's memory and pictures are as private as the
//! cartridge: OUT_DIR is the caller's to keep out of every repository.

use std::io::Write as _;

use nes_console::{ines, Alignment, Console};
use nes_glue::controller::Buttons;

/// An authored RGB approximation of the 2C02's 64 colours (the common
/// "2C02G" table), the one `run-rom` looks at frames through.
const PALETTE: [u32; 64] = [
    0x626262, 0x001fb2, 0x2404c8, 0x5200b2, 0x730076, 0x800024, 0x730b00, 0x522800, 0x244400, 0x005700, 0x005c00, 0x005324, 0x003c76, 0x000000, 0x000000, 0x000000,
    0xababab, 0x0d57ff, 0x4b30ff, 0x8a13ff, 0xbc08d6, 0xd21269, 0xc72e00, 0x9d5400, 0x607b00, 0x209800, 0x00a300, 0x009942, 0x007db4, 0x000000, 0x000000, 0x000000,
    0xffffff, 0x53aeff, 0x9085ff, 0xd365ff, 0xff57ff, 0xff5dcf, 0xff7757, 0xfa9e00, 0xbdc700, 0x7ae700, 0x43f011, 0x26e6a6, 0x2ccaff, 0x4e4e4e, 0x000000, 0x000000,
    0xffffff, 0xb6e1ff, 0xced1ff, 0xe9c3ff, 0xffbcff, 0xffbdf4, 0xffc6c3, 0xffd59a, 0xe9e681, 0xcef481, 0xb6fb9a, 0xa9fac3, 0xa9f0f4, 0xb8b8b8, 0x000000, 0x000000,
];

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 && a.len() != 6 {
        eprintln!("usage: storyboard ROM.nes SCRIPT FRAMES OUT_DIR [FRAME,FRAME,...]");
        std::process::exit(2);
    }
    let rom = std::fs::read(&a[1]).expect("the ROM");
    let script = std::fs::read_to_string(&a[2]).expect("the script");
    let frames: usize = a[3].parse().expect("FRAMES");
    let out = std::path::PathBuf::from(&a[4]);
    std::fs::create_dir_all(&out).expect("the output directory");
    let grabs: std::collections::BTreeSet<usize> = a.get(5).map(|g| g.split(',').filter(|s| !s.is_empty()).map(|s| s.parse().expect("a frame number")).collect()).unwrap_or_default();
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
    let mut ram = std::io::BufWriter::new(std::fs::File::create(out.join("ram.bin")).expect("ram.bin"));
    let mut pads = Vec::with_capacity(frames);
    let (mut next, mut pad, mut written) = (0, 0u8, 0usize);
    for f in 0..frames {
        while next < at.len() && at[next].0 <= f {
            pad = at[next].1;
            next += 1;
        }
        console.set_pad(0, Buttons::from_byte(pad));
        console.run_frames(1);
        pads.push(pad);
        let cells: Vec<u8> = (0..0x800u16).map(|k| console.peek(k)).collect();
        ram.write_all(&cells).expect("write");
        if grabs.contains(&f) {
            let frame = console.frames.last().expect("a frame");
            let mut ppm = Vec::with_capacity(15 + 256 * 240 * 3);
            write!(ppm, "P6\n256 240\n255\n").unwrap();
            for row in 0..240 {
                for d in 1..=256 {
                    let (idx, _) = frame.at(row, d);
                    let rgb = PALETTE[idx as usize & 63];
                    ppm.extend_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
                }
            }
            std::fs::write(out.join(format!("frame-{f:05}.ppm")), ppm).expect("a frame's picture");
            written += 1;
        }
        // Only the newest picture is ever read here.
        console.frames.clear();
    }
    ram.flush().expect("flush");
    std::fs::write(out.join("pad.bin"), &pads).expect("pad.bin");
    eprintln!("storyboard: {frames} frames, {written} pictures, {} bytes of memory a frame, in {}", 0x800, out.display());
}
