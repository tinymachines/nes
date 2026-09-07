//! The test cartridge (testrom.rs: the program that paints a picture and
//! counts NMIs, and its tiles) as an iNES file: mapper 0, 32 KiB of PRG,
//! 8 KiB of CHR, vertical mirroring. For a page that needs a cartridge
//! nobody owns (the roof's e2e), never a game.
//!   cargo run --release -p nes-console --example export-testrom -- out.nes [bars|pad|pad-dmc]
//! With `bars`, the colour-bars cartridge (testrom::bars_program); with
//! `pad` or `pad-dmc`, the bench's polling cartridge (testrom::pad_program).
use nes_console::testrom::{bars_chr, bars_program, chr, pad_program, program};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "testcart.nes".into());
    let kind = std::env::args().nth(2).unwrap_or_default();
    let (prg, chr) = match kind.as_str() {
        "bars" => (bars_program(), bars_chr()),
        "pad" => (pad_program(false), chr()),
        "pad-dmc" => (pad_program(true), chr()),
        _ => (program(), chr()),
    };
    assert_eq!(prg.len(), 0x8000);
    assert_eq!(chr.len(), 0x2000);
    let mut bytes = vec![b'N', b'E', b'S', 0x1a, 2, 1, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&prg);
    bytes.extend_from_slice(&chr);
    std::fs::write(&out, &bytes).unwrap();
    println!("wrote {out}: {} bytes, mapper 0, 32 KiB PRG, 8 KiB CHR, vertical mirroring", bytes.len());
}
