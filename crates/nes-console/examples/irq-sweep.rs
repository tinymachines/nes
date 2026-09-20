//! How far behind the board a cartridge's /IRQ reaches the core, swept.
//!
//!   cargo run --release -p nes-console --example irq-sweep -- [rom.nes] [max]
//!
//! Runs blargg's `mmc3_test_2/4-scanline_timing` (or a ROM named on the
//! command line) once for every delay from 0 to `max` master half-steps
//! and prints what each one reports. Twelve master half-steps are a CPU
//! half-cycle and eight are a PPU dot, so this is the only grain fine
//! enough: that ROM brackets the interrupt's arrival to one PPU clock,
//! which a CPU half-cycle steps over.
//!
//! What it is for is `CART_IRQ_DELAY`, and what it produces is a fit to
//! one ROM, not a propagation time measured on a part. Print the whole
//! sweep, not just the winner: a value that passes with failures either
//! side of it is a bracket, and one that passes at the end of a long run
//! of passes is a ROM that stopped discriminating.

use nes_console::{ines, Alignment, Console};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = args
        .get(1)
        .filter(|a| a.ends_with(".nes"))
        .cloned()
        .unwrap_or_else(|| {
            let base = std::env::var("NES_TEST_ROMS").unwrap_or_else(|_| format!("{}/roms/nes-test-roms", std::env::var("HOME").unwrap_or_default()));
            format!("{base}/mmc3_test_2/rom_singles/4-scanline_timing.nes")
        });
    let max: u64 = args.iter().skip(1).find_map(|s| s.parse().ok()).unwrap_or(30);
    let frames: usize = std::env::var("FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(900);
    let bytes = match std::fs::read(&rom) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("SKIP: {rom}: {e}");
            return;
        }
    };
    let parsed = ines::parse(&bytes).expect("iNES");
    println!("{rom}: mapper {}", parsed.mapper);
    println!("delay is in MASTER half-steps: 12 to a CPU half-cycle, 8 to a PPU dot\n");
    for delay in 0..=max {
        let chr_ram = parsed.chr_ram.then(|| vec![0u8; 0x2000]);
        let cart = parsed.cart().expect("a board this console has");
        let mut c = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
        c.cart_irq_delay = delay;
        let mut report = None;
        for _ in 0..frames {
            c.run_frames(1);
            let b = c.board.borrow();
            let ram = b.prg_ram.as_ref().expect("the reporting window");
            if ram[1..4] == [0xde, 0xb0, 0x61] && ram[0] != 0x80 {
                let text: String = ram[4..].iter().take_while(|&&x| x != 0).map(|&x| x as char).collect();
                report = Some((ram[0], text.trim().lines().next().unwrap_or("").to_string()));
                break;
            }
        }
        match report {
            Some((0, _)) => println!("  {delay:2}: PASSED"),
            Some((code, text)) => println!("  {delay:2}: failed #{code}: {text}"),
            None => println!("  {delay:2}: never reported in {frames} frames"),
        }
    }
}
