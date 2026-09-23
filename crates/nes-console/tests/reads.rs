//! A debugger's reads: what the machine holds, without moving it.
//!
//! The program writes things the reads should see (RAM, palette RAM, OAM,
//! the nametable) and the reads are checked against what it wrote; then
//! the same reads are taken twice, and a read that changed anything would
//! show in the second. The register peek is checked against the one thing
//! it must not do: $2002 through `peek` leaves the vblank flag standing.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::{Alignment, Console};

fn prg(code: &[u8]) -> Vec<u8> {
    let mut prg = vec![0xffu8; 0x8000];
    prg[0x4100..0x4100 + code.len()].copy_from_slice(code);
    prg[0x7ffc..0x7ffe].copy_from_slice(&[0x00, 0xc1]);
    prg
}

fn console(code: &[u8]) -> Console {
    let cart = Nrom::new(prg(code), vec![0u8; 0x2000], Mirroring::Vertical).expect("NROM");
    Console::with_prg_ram(Box::new(cart), None, Alignment::default(), true)
}

#[test]
fn the_reads_see_what_the_program_wrote_and_move_nothing() {
    let code: &[u8] = &[
        0xa9, 0x5a, // LDA #$5A
        0x85, 0x10, // STA $10
        0xa2, 0x07, // LDX #$07
        0x8e, 0xff, 0x01, // STX $01FF
        0xa9, 0x3f, 0x8d, 0x06, 0x20, // LDA #$3F; STA $2006
        0xa9, 0x00, 0x8d, 0x06, 0x20, // LDA #$00; STA $2006   (palette $3F00)
        0xa9, 0x21, 0x8d, 0x07, 0x20, // LDA #$21; STA $2007   (backdrop = $21)
        0xa9, 0x20, 0x8d, 0x06, 0x20, // LDA #$20; STA $2006
        0xa9, 0x00, 0x8d, 0x06, 0x20, // LDA #$00; STA $2006   (nametable $2000)
        0xa9, 0x42, 0x8d, 0x07, 0x20, // LDA #$42; STA $2007   (tile $42 at the top left)
        0xa9, 0x04, 0x8d, 0x03, 0x20, // LDA #$04; STA $2003   (OAM address 4: sprite 1)
        0xa9, 0x77, 0x8d, 0x04, 0x20, // LDA #$77; STA $2004   (sprite 1's Y)
        0xa0, 0x99, // LDY #$99            ($C131)
        0x4c, 0x31, 0xc1, // JMP $C131 (the LDY, so A, X and Y hold)
    ];
    let mut c = console(code);
    c.run_frames(2);

    let (a, x, y, s, _p, pc) = c.cpu_registers();
    assert_eq!((a, x, y), (0x77, 0x07, 0x99), "the registers the program left");
    assert!((0xc131..=0xc135).contains(&pc), "the PC is in the loop: {pc:#06x}");
    // The stack pointer is whatever the core's power-on left it; it is not
    // asserted here, only read again below and required to hold.
    let _ = s;
    let (fetch_pc, op) = c.last_fetch();
    assert!((0xc131..=0xc135).contains(&fetch_pc), "the last fetch is in the loop: {fetch_pc:#06x}");
    assert!(op == 0xa0 || op == 0x4c, "the opcode fetched is the loop's: {op:#04x}");

    assert_eq!(c.peek(0x0010), 0x5a, "zero page, as written");
    assert_eq!(c.peek(0x01ff), 0x07, "the stack page, as written");
    assert_eq!(c.peek(0xc100), 0xa9, "the cartridge, the program's first byte");
    assert_eq!(c.peek(0xfffc), 0x00, "the reset vector, low");

    assert_eq!(c.ppu_palette()[0], 0x21, "the backdrop the program set");
    assert_eq!(c.ppu_oam()[4], 0x77, "sprite 1's Y as written");
    assert_eq!(c.ciram()[0], 0x42, "the nametable's first tile as written (vertical mirroring: $2000 is CIRAM 0)");
    assert_eq!(c.ciram().len(), 0x800);
    assert!(c.chr_ram().is_none(), "NROM with CHR-ROM: the file has the tiles, the console no RAM");

    let st = c.ppu_status();
    assert!(st.line < 262 && st.dot < 341, "a position on the frame: {} {}", st.line, st.dot);
    assert_eq!(st.oamaddr, 0x05, "OAM address advanced past the byte written");

    // Reading changes nothing: the same reads twice, and the machine's
    // counters between them.
    let before = (c.master, c.cpu_half_cycles, c.dots, c.board.borrow().reads, c.board.borrow().writes);
    let first = (c.cpu_registers(), c.last_fetch(), c.peek(0x0010), c.peek(0x2002), c.ppu_palette(), c.ppu_oam(), c.ppu_status(), c.ciram());
    let second = (c.cpu_registers(), c.last_fetch(), c.peek(0x0010), c.peek(0x2002), c.ppu_palette(), c.ppu_oam(), c.ppu_status(), c.ciram());
    assert_eq!(first, second);
    let after = (c.master, c.cpu_half_cycles, c.dots, c.board.borrow().reads, c.board.borrow().writes);
    assert_eq!(before, after, "no read counted as a bus access, no step was taken");
}

#[test]
fn a_peek_at_2002_leaves_the_vblank_flag_standing() {
    // A program that does nothing: the PPU raises vblank on its own each
    // frame, and a real read of $2002 would clear it. The peek must not.
    let code: &[u8] = &[0x4c, 0x00, 0xc1];
    let mut c = console(code);
    // Run to somewhere inside vblank: line 241 and on, before the pre-render
    // line. A frame is 89,342 dots of eight master half-steps each, so the
    // ceiling is three frames.
    for _ in 0..(3 * 89_342 * 8) {
        c.master_half_step();
        let st = c.ppu_status();
        if st.vbl && st.line >= 242 && st.line < 260 {
            break;
        }
    }
    assert!(c.ppu_status().vbl, "the flag is up inside vblank");
    let _ = c.peek(0x2002);
    let _ = c.peek(0x2002);
    assert!(c.ppu_status().vbl, "still up: a peek is not a read of the register");
}
