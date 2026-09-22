//! The cartridge RAM leaves the console and comes back: what a battery
//! does on a real cartridge, done by whoever keeps the save.
//!
//! A program on the die writes to $6000 and the bytes are in what
//! `battery_ram` hands out; a save put back with `set_battery_ram` is
//! what a program then reads at $6000. A board with no RAM refuses, and
//! so does a save of the wrong size, rather than loading half of one.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{ines, Alignment, Console};

/// 32 KiB of PRG with `code` at $C100 and the reset vector pointing at it.
fn prg(code: &[u8]) -> Vec<u8> {
    let mut prg = vec![0xffu8; 0x8000];
    prg[0x4100..0x4100 + code.len()].copy_from_slice(code);
    prg[0x7ffc..0x7ffe].copy_from_slice(&[0x00, 0xc1]);
    prg
}

fn console(code: &[u8], ram: bool) -> Console {
    let cart = Nrom::new(prg(code), vec![0u8; 0x2000], Mirroring::Vertical).expect("NROM");
    Console::with_prg_ram(Box::new(cart), None, Alignment::default(), ram)
}

#[test]
fn what_a_program_writes_at_6000_is_in_the_battery_ram() {
    let code: &[u8] = &[
        0xa9, 0x5a, // LDA #$5A
        0x8d, 0x00, 0x60, // STA $6000
        0xa9, 0xa5, // LDA #$A5
        0x8d, 0xff, 0x7f, // STA $7FFF
        0x4c, 0x0a, 0xc1, // JMP here
    ];
    let mut c = console(code, true);
    c.run_frames(2);
    let ram = c.battery_ram().expect("the board fits RAM");
    assert_eq!(ram.len(), 0x2000);
    assert_eq!((ram[0], ram[0x1fff]), (0x5a, 0xa5));
    assert!(ram[1..0x1fff].iter().all(|&b| b == 0), "only the two bytes the program wrote");
}

#[test]
fn a_save_put_back_is_what_a_program_reads() {
    let code: &[u8] = &[
        0xad, 0x34, 0x62, // LDA $6234
        0x85, 0x10, // STA $10
        0x4c, 0x05, 0xc1, // JMP here
    ];
    let mut c = console(code, true);
    let mut save = vec![0u8; 0x2000];
    save[0x234] = 0xc7;
    c.set_battery_ram(&save).expect("a save the size of the RAM");
    c.run_frames(2);
    assert_eq!(c.board.borrow().wram.read(0x0010), 0xc7, "the program read the save's byte");
    // Without the save the same program reads the power-on zero, which is
    // what says the assertion above is about the save.
    let mut fresh = console(code, true);
    fresh.run_frames(2);
    assert_eq!(fresh.board.borrow().wram.read(0x0010), 0);
}

#[test]
fn no_ram_and_the_wrong_size_are_refused() {
    let mut none = console(&[0x4c, 0x00, 0xc1], false);
    assert_eq!(none.battery_ram(), None);
    assert!(none.set_battery_ram(&[0u8; 0x2000]).unwrap_err().contains("no cartridge RAM"));
    let mut some = console(&[0x4c, 0x00, 0xc1], true);
    let err = some.set_battery_ram(&[0u8; 0x1000]).unwrap_err();
    assert!(err.contains("8192") && err.contains("4096"), "{err}");
    assert!(some.battery_ram().unwrap().iter().all(|&b| b == 0), "a refused save changed nothing");
}

#[test]
fn the_header_says_whether_there_is_a_battery() {
    let mut rom = b"NES\x1a\x02\x01\x12\x00".to_vec(); // mapper 1, vertical, battery
    rom.extend_from_slice(&[0u8; 8]);
    rom.extend_from_slice(&[0u8; 0x8000 + 0x2000]);
    assert!(ines::parse(&rom).unwrap().battery);
    rom[6] = 0x10;
    assert!(!ines::parse(&rom).unwrap().battery);
}
