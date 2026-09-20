//! MMC3, mapper 4, on the console: blargg's own test cartridges run to
//! their report, and what each one says recorded by name.
//!
//! The suite is `mmc3_test_2`, whose ROMs clock the board's counter by
//! hand through $2006 and then against the PPU's own fetches, and
//! report through the $6000 window. Five of the six pass. The one that
//! does not tests the OTHER chip: blargg's readme names the two, his
//! Crystalis (revision A) stopping when $C000 holds 0 and his Super
//! Mario Bros. 3 and Mega Man 3 (revision B) reloading every time.
//! `nes-bus`'s board is the revision B one, which is the board the
//! games this console is for are on, and `5-MMC3` is the ROM that
//! holds it.
//!
//! `4-scanline_timing` joined the passing list on 2026-09-20, and it
//! took two things, because it brackets the interrupt's arrival to ONE
//! PPU clock and the console was wrong by more than that in two
//! independent ways:
//!
//! - The A12 filter was one dot too permissive, so with the background
//!   at $1000 the rise at the first pattern fetch of line 0 was counted
//!   and a frame came to 242 clocks on alternate frames. `A12_FILTER_DOTS`
//!   says why nine is the wrong rounding of "three falling edges of M2".
//! - The cartridge's /IRQ reached the core with no delay at all, as a
//!   level read at whatever CPU half-cycle came next. It is a line, and
//!   `CART_IRQ_DELAY` holds it behind the board by sixteen master
//!   half-steps, which `examples/irq-sweep` is the measurement of.
//!
//! The ROMs are read from the nes-test-roms checkout (NES_TEST_ROMS, or
//! ~/roms/nes-test-roms) and the test SKIPS by name without it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use nes_bus::cart::{Cartridge, Mirroring, Mmc3};
use nes_console::{ines, Alignment, Console};

fn roms() -> Option<PathBuf> {
    let base = std::env::var("NES_TEST_ROMS").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("roms").join("nes-test-roms")
    });
    base.join("mmc3_test_2").join("rom_singles").join("1-clocking.nes").is_file().then_some(base)
}

/// Run a ROM until its $6000 window stops saying "running", at most
/// `frames`, and return (result byte, text).
fn report(path: &PathBuf, frames: usize) -> (u8, String) {
    let bytes = std::fs::read(path).expect("rom");
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let mut c = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
    let mut done = None;
    for _ in 0..frames {
        c.run_frames(1);
        let b = c.board.borrow();
        let ram = b.prg_ram.as_ref().expect("the reporting window");
        if ram[1..4] == [0xde, 0xb0, 0x61] && ram[0] != 0x80 {
            let text: String = ram[4..].iter().take_while(|&&x| x != 0).map(|&x| x as char).collect();
            done = Some((ram[0], text.trim().to_string()));
            break;
        }
    }
    done.unwrap_or_else(|| panic!("{} never reported in {frames} frames", path.display()))
}

#[test]
fn blarggs_mmc3_roms_say_what_this_board_is() {
    let Some(base) = roms() else {
        eprintln!("SKIP: no nes-test-roms checkout (NES_TEST_ROMS)");
        return;
    };
    let dir = base.join("mmc3_test_2").join("rom_singles");
    // The five that must pass. They are the whole of what a game uses:
    // that the counter clocks at all, that it clocks once a line and
    // 241 times a frame, that a game can clock it by hand through
    // $2006, that the interrupt lands where the part puts it to one PPU
    // clock, and that the reload behaves as the board Super Mario Bros.
    // 3 is on.
    for rom in ["1-clocking", "2-details", "3-A12_clocking", "4-scanline_timing", "5-MMC3"] {
        let (code, text) = report(&dir.join(format!("{rom}.nes")), 900);
        assert_eq!(code, 0, "{rom}: {text}");
    }
    // The one that does not, failing where this file says it does.
    // Recorded, not tolerated: if it starts passing, or fails somewhere
    // else, this says so.
    let (code, text) = report(&dir.join("6-MMC3_alt.nes"), 600);
    assert_eq!((code, text.lines().next().unwrap_or("")), (2, "IRQ shouldn't be set when reloading to 0 due to counter naturally reaching 0 previously"), "6-MMC3_alt is the other revision; this board is the one Super Mario Bros. 3 is on");
}

/// The same claim from outside a ROM that does nothing else: a
/// cartridge whose whole program turns rendering on with the
/// background in one pattern table and the sprites in the other, and
/// then loops. The board's counter is clocked 241 times a frame, once
/// for each of the 240 visible lines and once for the pre-render line.
/// With both tables the same it is clocked not at all, which is the
/// part's behaviour and the reason a game separates them.
#[test]
fn the_board_counts_a_line_at_a_time_and_only_across_the_two_tables() {
    for (ctrl, want) in [(0x08u8, 241u64), (0x00, 0)] {
        // 32 KiB of PRG; the code sits in the last 8 KiB, which MMC3
        // fixes at $E000 in both of its modes, so it runs whatever the
        // bank registers power up as.
        let mut prg = vec![0u8; 0x8000];
        let code: &[u8] = &[
            0xa9, ctrl, // LDA #ctrl
            0x8d, 0x00, 0x20, // STA $2000
            0xa9, 0x18, // LDA #$18: background and sprites shown
            0x8d, 0x01, 0x20, // STA $2001
            0x4c, 0x0a, 0xe0, // JMP here
        ];
        let base = prg.len() - 0x2000;
        prg[base..base + code.len()].copy_from_slice(code);
        let n = prg.len();
        prg[n - 4..n - 2].copy_from_slice(&[0x00, 0xe0]); // reset vector $E000
        let board = Rc::new(RefCell::new(Mmc3::new(prg, vec![0u8; 0x2000], Mirroring::Vertical).expect("MMC3")));
        let mut c = Console::new(Box::new(Shared(board.clone())), None, Alignment::default());
        // Past the CPU's reset and the PPU's warm-up, then whole frames.
        c.run_frames(3);
        let mut per_frame = Vec::new();
        let mut last = board.borrow().clocks().0;
        for _ in 0..3 {
            c.run_frames(1);
            let now = board.borrow().clocks().0;
            per_frame.push(now - last);
            last = now;
        }
        assert!(per_frame.iter().all(|&n| n == want), "$2000 = {ctrl:02x}: expected {want} clocks a frame, got {per_frame:?}");
    }
}

/// The board, shared, so a test can watch the same one the console
/// holds. It delegates every call and decides nothing.
struct Shared(Rc<RefCell<Mmc3>>);

impl Cartridge for Shared {
    fn cpu_read(&mut self, a: u16) -> Option<u8> {
        self.0.borrow_mut().cpu_read(a)
    }
    fn cpu_write(&mut self, a: u16, v: u8) {
        self.0.borrow_mut().cpu_write(a, v)
    }
    fn chr_read(&mut self, a: u16) -> Option<u8> {
        self.0.borrow_mut().chr_read(a)
    }
    fn chr_write(&mut self, a: u16, v: u8) {
        self.0.borrow_mut().chr_write(a, v)
    }
    fn ciram(&self, ppu_a: u16) -> (bool, bool) {
        self.0.borrow().ciram(ppu_a)
    }
    fn ppu_bus(&mut self, ppu_a: u16, dot: u64) {
        self.0.borrow_mut().ppu_bus(ppu_a, dot)
    }
    fn irq(&self) -> bool {
        self.0.borrow().irq()
    }
    fn owns_chr_ram(&self) -> bool {
        self.0.borrow().owns_chr_ram()
    }
}
