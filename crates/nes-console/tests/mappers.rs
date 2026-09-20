//! The three boards the console gained after MMC3, plugged in and run:
//! MMC1 (mapper 1), UxROM (2) and CNROM (3).
//!
//! What nes-bus's own tests hold is the boards' logic from the outside,
//! a register at a time. What this holds is the two things only a
//! console can say: that the header's mapper number reaches the right
//! board, and that a program running on the die switches a bank and
//! reads back through it.
//!
//! The MMC1 case is the one that needed the console. Its serial port
//! ignores the second of two writes on CONSECUTIVE CPU cycles, and an
//! RMW instruction on the window is exactly such a pair: the dummy
//! write-back and the real write. The dot that decides it is the
//! console's, so `Board` hands it to the cartridge with every CPU write
//! (`Cartridge::cpu_write_at`) and this is what proves it arrives. Run
//! against a board whose writes go through `cpu_write` instead, the
//! `INC` shifts twice and the count reads 2.

use std::cell::RefCell;
use std::rc::Rc;

use nes_bus::cart::{Cartridge, Cnrom, Mirroring, Mmc1, Uxrom};
use nes_console::{ines, Alignment, Console};

/// A board the test keeps a handle on while the console holds it. It
/// delegates every call and decides nothing.
struct Shared<T>(Rc<RefCell<T>>);

impl<T: Cartridge> Cartridge for Shared<T> {
    fn cpu_read(&mut self, a: u16) -> Option<u8> {
        self.0.borrow_mut().cpu_read(a)
    }
    fn cpu_write(&mut self, a: u16, v: u8) {
        self.0.borrow_mut().cpu_write(a, v)
    }
    fn cpu_write_at(&mut self, a: u16, v: u8, dot: u64) {
        self.0.borrow_mut().cpu_write_at(a, v, dot)
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

/// 128 KiB of PRG as eight 16 KiB banks, each filled with its own index
/// so a byte read back names the bank that answered, with `code` in the
/// LAST bank (which every one of these boards leaves reachable at
/// $C000) and the reset vector pointing at it. Byte 0 of each bank is
/// $FF, so a write there is not masked by a bus conflict.
fn banked_prg(code: &[u8]) -> Vec<u8> {
    let mut prg = vec![0u8; 8 * 0x4000];
    for (i, b) in prg.chunks_mut(0x4000).enumerate() {
        b.fill(i as u8);
        b[0] = 0xff;
    }
    let last = prg.len() - 0x4000;
    prg[last + 0x0100..last + 0x0100 + code.len()].copy_from_slice(code);
    let n = prg.len();
    prg[n - 4..n - 2].copy_from_slice(&[0x00, 0xc1]); // reset vector $C100
    prg
}

/// Run a console until it has drawn `frames` pictures, then read a byte
/// of its work RAM.
fn ram_after(cart: Box<dyn Cartridge>, frames: usize, at: u16) -> u8 {
    let mut c = Console::new(cart, Some(vec![0u8; 0x2000]), Alignment::default());
    c.run_frames(frames);
    let b = c.board.borrow();
    b.wram.read(at)
}

/// UxROM switches the low half of the window and leaves the high half
/// where the program is: a program in the fixed bank selects bank 5 and
/// reads a byte through it.
#[test]
fn uxrom_switches_under_a_program_running_in_the_fixed_bank() {
    let code: &[u8] = &[
        0xa9, 0x05, // LDA #$05
        0x8d, 0x00, 0x80, // STA $8000   (the ROM byte there is $FF: no masking)
        0xad, 0x01, 0x80, // LDA $8001   (bank 5, filled with 5)
        0x85, 0x10, // STA $10
        0x4c, 0x0a, 0xc1, // JMP here
    ];
    let cart = Uxrom::new(banked_prg(code), Vec::new(), Mirroring::Vertical).expect("UxROM");
    assert_eq!(ram_after(Box::new(cart), 2, 0x0010), 5, "the byte read through the switched half names bank 5");
}

/// CNROM's PRG does not move and its CHR does: the program writes the
/// bank register and the board latches two bits of it.
#[test]
fn cnrom_latches_two_bits_of_what_a_program_writes() {
    let code: &[u8] = &[
        0xa9, 0x07, // LDA #$07: seven, of which the latch takes three
        0x8d, 0x00, 0x80, // STA $8000
        0x4c, 0x05, 0xc1, // JMP here
    ];
    // 32 KiB, fixed: $8000..$FFFF is the whole of it, and the code sits
    // at $C100 as everywhere else here. Byte 0 is $FF so the write is
    // not masked by the conflict.
    let mut prg = vec![0xffu8; 0x8000];
    prg[0x4100..0x4100 + code.len()].copy_from_slice(code);
    prg[0x7ffc..0x7ffe].copy_from_slice(&[0x00, 0xc1]);
    let board = Rc::new(RefCell::new(Cnrom::new(prg, vec![0u8; 0x8000], Mirroring::Vertical).expect("CNROM")));
    let mut c = Console::new(Box::new(Shared(board.clone())), None, Alignment::default());
    c.run_frames(2);
    assert_eq!(board.borrow().bank(), 3, "seven written, three latched");
}

/// MMC1's serial port takes an RMW instruction's two writes as one.
///
/// `INC $8001` writes the byte back unchanged and then writes it plus
/// one, on consecutive CPU cycles. The part's port accepts the first
/// and ignores the second, so one bit goes in. The byte there is $00
/// (bank 0's fill), so the two writes are $00 and $01: their bit 0
/// differs, and taking both would leave a different value as well as a
/// different count. $8001 and not $8000 because byte 0 of each bank is
/// $FF, and a write with bit 7 set is MMC1's RESET write, not a bit.
#[test]
fn mmc1_takes_an_rmws_two_writes_as_one_through_the_console() {
    let code: &[u8] = &[
        0xee, 0x01, 0x80, // INC $8001
        0x4c, 0x03, 0xc1, // JMP here
    ];
    let board = Rc::new(RefCell::new(Mmc1::new(banked_prg(code), Vec::new(), Mirroring::Vertical).expect("MMC1")));
    let mut c = Console::new(Box::new(Shared(board.clone())), None, Alignment::default());
    c.run_frames(1);
    assert_eq!(board.borrow().shift_state(), (0, 1), "one bit in, and it is the FIRST write's ($00), not the second's ($01)");
}

/// A whole five-bit word through the port from a program, and the bank
/// it selects read back: the ordinary way an SxROM game switches.
#[test]
fn mmc1_loads_a_word_from_five_writes_and_the_bank_answers() {
    // Five STAs of the bits of 5 (1, 0, 1, 0, 0), lowest first, to
    // $E000: the PRG bank register. Separate instructions, so no pair.
    let mut code: Vec<u8> = Vec::new();
    for bit in [1u8, 0, 1, 0, 0] {
        code.extend([0xa9, bit, 0x8d, 0x00, 0xe0]); // LDA #bit; STA $E000
    }
    code.extend([0xad, 0x01, 0x80, 0x85, 0x10]); // LDA $8001; STA $10
    let here = 0xc100 + code.len() as u16;
    code.extend([0x4c, here as u8, (here >> 8) as u8]);
    let cart = Mmc1::new(banked_prg(&code), Vec::new(), Mirroring::Vertical).expect("MMC1");
    assert_eq!(ram_after(Box::new(cart), 2, 0x0010), 5, "PRG mode 3: bank 5 at $8000");
}

/// The header's mapper number reaches the right board, and the ones
/// this console does not have are refused by name rather than falling
/// back to something plausible.
#[test]
fn the_header_names_the_board_and_an_unknown_one_is_refused() {
    let image = |mapper: u8, prg_banks: u8, chr_banks: u8| {
        let mut b = vec![0u8; 16];
        b[0..4].copy_from_slice(b"NES\x1a");
        b[4] = prg_banks;
        b[5] = chr_banks;
        b[6] = (mapper & 0x0f) << 4;
        b[7] = mapper & 0xf0;
        b.resize(16 + prg_banks as usize * 0x4000 + chr_banks as usize * 0x2000, 0);
        b
    };
    for (mapper, prg_banks, chr_banks) in [(0u8, 2u8, 1u8), (1, 8, 0), (2, 8, 0), (3, 2, 4), (4, 16, 16), (66, 4, 2)] {
        let rom = ines::parse(&image(mapper, prg_banks, chr_banks)).expect("iNES");
        assert_eq!(rom.mapper, mapper);
        assert!(rom.cart().is_ok(), "mapper {mapper} is a board this console has");
    }
    // Refused, each for its own reason, and each saying which.
    let why = |mapper, prg, chr| match ines::parse(&image(mapper, prg, chr)).unwrap().cart() {
        Ok(_) => panic!("mapper {mapper} with {prg} PRG and {chr} CHR banks should have been refused"),
        Err(e) => e,
    };
    let e = why(5, 8, 2);
    assert!(e.contains("mapper 5"), "{e}");
    let e = why(2, 8, 2);
    assert!(e.contains("UxROM carries CHR RAM"), "{e}");
    let e = why(3, 2, 0);
    assert!(e.contains("CHR RAM"), "{e}");
}
