//! The calibration cartridge (nes-bench/docs/calibration-plan.md, C0):
//! one NROM cartridge whose every screen is built to be measured, off
//! the part and off the model through the same reader, and whose every
//! frame names itself.
//!
//! The strip. Three rows of fifteen blocks along the top of every
//! screen, each block two tiles square (16 by 16 dots), white or black
//! on the backdrop: a bit each. Row A carries a sync pattern, the screen
//! id, the variant within the screen, the hold flag and a parity bit;
//! row B the low twelve bits of a frame counter; row C the counter's top
//! four bits and the byte the console read from the pad at its last
//! poll. A grabbed frame, a decoded scope record and a model frame are
//! matched by reading the strip, not by guessing what a trigger caught,
//! and the pad's echo is what closes the bench's loop on a byte.
//!
//! The picture is drawn from tiles, so every block and every measured
//! region falls on tile boundaries, and `manifest()` writes where each
//! is from the same constants that lay the tiles: nothing about the
//! picture is typed twice. `read_strip` reads the model's frame by those
//! rectangles alone; `MUTATE=1` in the test shifts it one tile and must
//! misread.
//!
//! What runs each frame: the NMI handler reads the pad, writes the
//! sixteen background palette entries and the ninety strip tiles from
//! buffers, resets the scroll and writes the mask (emphasis included);
//! the main loop, after each NMI, advances the counters, steps the
//! screen on its timer or on Select (Start toggles a hold), redraws the
//! nametable when the screen changed (rendering off for that one frame,
//! the frame counter still counting), and builds the next frame's
//! buffers. So a byte polled in the blanking that ends frame k is built
//! into the buffers during frame k+1, written in the blanking that ends
//! k+1, and seen in frame k+2, two frames after the poll and one after
//! the frame whose blanking polled it; a button's colour on the pad
//! screen lands the same frame the echo does. The latency is the same
//! on the part and the model, and the frame counter names both frames. The NMI's PPU writes take about 1,700 cycles of the 2,270 the
//! blanking allows, measured by counting: nothing else touches the PPU
//! while rendering is on.
//!
//! Palette slots: palette 3, colour 3 is the strip's white ($30) on
//! every screen; the backdrop is $0F on every screen and is the strip's
//! black. The eight other content colours are the screen's.

use nes_bus::DotFrame;
use std::collections::HashMap;

// ------------------------------------------------------------ geometry
/// The strip's blocks: two tiles square, starting one tile in and two
/// tiles down, fifteen to a row, three rows. Dots.
pub const BLOCK: usize = 16;
pub const STRIP_X0: usize = 8;
pub const STRIP_Y0: usize = 16;
pub const STRIP_COLS: usize = 15;
pub const STRIP_ROWS: usize = 3;
/// The first tile row of a screen's content, below the strip.
pub const CONTENT_ROW: usize = 8;
/// The palette entries the strip is drawn in.
pub const WHITE: u8 = 0x30;
pub const BLACK: u8 = 0x0f;
/// Frames a screen's variant lasts, and variants per screen; a screen
/// steps when its last variant ends, unless held. Screens 1 and 2 cycle
/// their palettes; the rest have one variant of a fixed length.
pub const SCREENS: usize = 8;
pub const VARIANT_FRAMES: [u8; SCREENS] = [240, 60, 120, 240, 240, 240, 240, 240];
pub const VARIANTS: [u8; SCREENS] = [1, 56, 8, 1, 1, 1, 1, 1];
pub const SCREEN_NAMES: [&str; SCREENS] = ["strip", "palette", "bars", "gratings", "edges", "dotcrawl", "pad", "geometry"];
/// The three sync patterns, one per strip row, MSB first over three blocks.
pub const SYNC: [u8; 3] = [0b101, 0b010, 0b110];

// Tiles, by index in CHR. 1 to 3 are solid in colours 1 to 3; the rest
// draw colour 1 in a pattern.
const T_BLANK: u8 = 0;
const T_C1: u8 = 1;
const T_C2: u8 = 2;
const T_C3: u8 = 3;
const T_V1: u8 = 4;
const T_V2: u8 = 5;
const T_V4: u8 = 6;
const T_H1: u8 = 7;
const T_H2: u8 = 8;
const T_H4: u8 = 9;
const T_CHK1: u8 = 10;
const T_CHK2: u8 = 11;
const T_HTOP: u8 = 12;
const T_HBOT: u8 = 13;
const T_VLEFT: u8 = 14;
const T_VRIGHT: u8 = 15;
const T_TL: u8 = 16;
const T_TR: u8 = 17;
const T_BL: u8 = 18;
const T_BR: u8 = 19;

// Zero page.
const Z_FRAME: u8 = 0x00; // 16 bits
const Z_SCREEN: u8 = 0x02;
const Z_TIMER: u8 = 0x03;
const Z_PAD: u8 = 0x04;
const Z_PREV: u8 = 0x05;
const Z_HOLD: u8 = 0x06;
const Z_VARIANT: u8 = 0x09;
const Z_REDRAW: u8 = 0x0a;
const Z_NMI: u8 = 0x0b;
const Z_NEW: u8 = 0x0d;
const Z_LOADING: u8 = 0x0e;
const Z_BUF: u8 = 0x20; // 90 tiles: rows A, B, C, 30 each
const Z_PAL: u8 = 0x80; // 16 palette bytes
const Z_MASK: u8 = 0x90;
const Z_TMP: u8 = 0x91;
const Z_PTR: u8 = 0x92; // 16 bits

// Data in PRG (addresses in the CPU's space).
const A_PAL_PAGES: u16 = 0x9000; // 7 pages x 16
const A_BARS_PAL: u16 = 0x9080; // 8 variants x 16
const A_FIXED_PAL: u16 = 0x9100; // 16
const A_VPERIOD: u16 = 0x9120; // 8
const A_VCOUNT: u16 = 0x9128; // 8
const A_NAMETABLES: u16 = 0xa000; // 8 x 1024

// ---------------------------------------------------------- assembler
/// Enough of an assembler to write this program with labels: bytes in,
/// branches and jumps by name, resolved at the end. Every opcode used is
/// written out at the call, so the listing below reads as 6502.
struct Asm {
    org: u16,
    b: Vec<u8>,
    labels: HashMap<&'static str, u16>,
    fix: Vec<(usize, &'static str, bool)>, // (offset, label, relative)
}

impl Asm {
    fn new(org: u16) -> Asm {
        Asm { org, b: Vec::new(), labels: HashMap::new(), fix: Vec::new() }
    }
    fn pc(&self) -> u16 {
        self.org + self.b.len() as u16
    }
    fn e(&mut self, bytes: &[u8]) {
        self.b.extend_from_slice(bytes);
    }
    fn label(&mut self, name: &'static str) {
        let pc = self.pc();
        assert!(self.labels.insert(name, pc).is_none(), "label {name} twice");
    }
    fn br(&mut self, op: u8, to: &'static str) {
        self.e(&[op, 0]);
        self.fix.push((self.b.len() - 1, to, true));
    }
    fn abs(&mut self, op: u8, to: &'static str) {
        self.e(&[op, 0, 0]);
        self.fix.push((self.b.len() - 2, to, false));
    }
    fn jmp(&mut self, to: &'static str) {
        self.abs(0x4c, to);
    }
    fn jsr(&mut self, to: &'static str) {
        self.abs(0x20, to);
    }
    fn pad_to(&mut self, addr: u16) {
        assert!(self.pc() <= addr, "code at {:04x} runs past {addr:04x}", self.pc());
        while self.pc() < addr {
            self.b.push(0xea);
        }
    }
    fn finish(mut self) -> Vec<u8> {
        for (at, name, rel) in std::mem::take(&mut self.fix) {
            let target = *self.labels.get(name).unwrap_or_else(|| panic!("no label {name}"));
            if rel {
                let from = self.org as i32 + at as i32 + 1;
                let d = target as i32 - from;
                assert!((-128..=127).contains(&d), "branch to {name} out of range ({d})");
                self.b[at] = d as i8 as u8;
            } else {
                self.b[at] = target as u8;
                self.b[at + 1] = (target >> 8) as u8;
            }
        }
        self.b
    }
}

// Opcodes, named where a line would otherwise be a bare number.
const LDA_IMM: u8 = 0xa9;
const LDA_ZP: u8 = 0xa5;
const LDA_ABS: u8 = 0xad;
const LDA_ABX: u8 = 0xbd;
const LDA_IZY: u8 = 0xb1;
const STA_ZP: u8 = 0x85;
const STA_ABS: u8 = 0x8d;
const STA_ZPX: u8 = 0x95;
const STA_ZPY: u8 = 0x99; // STA abs,Y
const LDX_IMM: u8 = 0xa2;
const LDX_ZP: u8 = 0xa6;
const LDY_IMM: u8 = 0xa0;
const INC_ZP: u8 = 0xe6;
const CMP_IMM: u8 = 0xc9;
const CMP_ABX: u8 = 0xdd;
const CPX_IMM: u8 = 0xe0;
const CPY_IMM: u8 = 0xc0;
const AND_IMM: u8 = 0x29;
const AND_ZP: u8 = 0x25;
const EOR_IMM: u8 = 0x49;
const EOR_ZP: u8 = 0x45;
const ORA_IMM: u8 = 0x09;
const ADC_IMM: u8 = 0x69;
const ASL_A: u8 = 0x0a;
const ASL_ZP: u8 = 0x06;
const LSR_A: u8 = 0x4a;
const ROR_ZP: u8 = 0x66;
const BNE: u8 = 0xd0;
const BEQ: u8 = 0xf0;
const BPL: u8 = 0x10;

fn ppu_addr(a: &mut Asm, hi: u8, lo: u8) {
    a.e(&[LDA_IMM, hi, STA_ABS, 0x06, 0x20, LDA_IMM, lo, STA_ABS, 0x06, 0x20]);
}

/// The program, 32 KiB, vectors at the top.
pub fn program() -> Vec<u8> {
    let mut a = Asm::new(0x8000);
    // ---- reset
    a.label("reset");
    a.e(&[0x78, 0xd8, LDX_IMM, 0xff, 0x9a]); // SEI; CLD; LDX #$FF; TXS
    a.e(&[LDA_IMM, 0x40, STA_ABS, 0x17, 0x40]); // $4017 <- $40
    a.e(&[LDA_IMM, 0x00, STA_ABS, 0x00, 0x20, STA_ABS, 0x01, 0x20]); // $2000, $2001 <- 0
    for _ in 0..2 {
        a.e(&[0x2c, 0x02, 0x20, BPL, 0xfb]); // BIT $2002; BPL -5
    }
    a.e(&[LDX_IMM, 0x00, LDA_IMM, 0x00]);
    a.label("zero");
    a.e(&[STA_ZPX, 0x00, 0xe8]); // STA $00,X; INX
    a.br(BNE, "zero");
    a.jsr("load_screen");
    a.jsr("build_palette");
    a.jsr("build_strip");
    a.e(&[LDA_IMM, 0x80, STA_ABS, 0x00, 0x20]); // NMI on
    // ---- main loop: once per NMI
    a.label("main");
    a.e(&[LDA_ZP, Z_NMI]);
    a.br(BEQ, "main");
    a.e(&[LDA_IMM, 0x00, STA_ZP, Z_NMI]);
    // new presses = ~prev & pad
    a.e(&[LDA_ZP, Z_PREV, EOR_IMM, 0xff, AND_ZP, Z_PAD, STA_ZP, Z_NEW]);
    a.e(&[AND_IMM, 0x04]); // Select
    a.br(BEQ, "nosel");
    a.jsr("next_screen");
    a.label("nosel");
    a.e(&[LDA_ZP, Z_NEW, AND_IMM, 0x08]); // Start
    a.br(BEQ, "nostart");
    a.e(&[LDA_ZP, Z_HOLD, EOR_IMM, 0x01, STA_ZP, Z_HOLD]);
    a.label("nostart");
    // the timer: a variant every VPERIOD[screen] frames, VCOUNT variants
    a.e(&[INC_ZP, Z_TIMER, LDX_ZP, Z_SCREEN, LDA_ZP, Z_TIMER, CMP_ABX, A_VPERIOD as u8, (A_VPERIOD >> 8) as u8]);
    a.br(BNE, "nov");
    a.e(&[LDA_IMM, 0x00, STA_ZP, Z_TIMER, INC_ZP, Z_VARIANT, LDA_ZP, Z_VARIANT, CMP_ABX, A_VCOUNT as u8, (A_VCOUNT >> 8) as u8]);
    a.br(BNE, "nov");
    a.e(&[LDA_IMM, 0x00, STA_ZP, Z_VARIANT, LDA_ZP, Z_HOLD]);
    a.br(BNE, "nov");
    a.jsr("next_screen");
    a.label("nov");
    a.e(&[LDA_ZP, Z_REDRAW]);
    a.br(BEQ, "nored");
    a.jsr("load_screen");
    a.e(&[LDA_IMM, 0x00, STA_ZP, Z_REDRAW]);
    a.label("nored");
    a.jsr("build_palette");
    a.jsr("build_strip");
    a.jmp("main");
    // ---- next_screen
    a.label("next_screen");
    a.e(&[INC_ZP, Z_SCREEN, LDA_ZP, Z_SCREEN, AND_IMM, 0x07, STA_ZP, Z_SCREEN]);
    a.e(&[LDA_IMM, 0x00, STA_ZP, Z_TIMER, STA_ZP, Z_VARIANT, LDA_IMM, 0x01, STA_ZP, Z_REDRAW, 0x60]);
    // ---- load_screen: rendering off, the nametable and attributes
    // copied from $A000 + screen * $400, scroll reset. Rendering comes
    // back with the next NMI's mask write.
    a.label("load_screen");
    a.e(&[LDA_IMM, 0x01, STA_ZP, Z_LOADING, LDA_IMM, 0x00, STA_ABS, 0x01, 0x20]);
    a.e(&[LDA_ZP, Z_SCREEN, ASL_A, ASL_A, 0x18, ADC_IMM, (A_NAMETABLES >> 8) as u8, STA_ZP, Z_PTR + 1, LDA_IMM, 0x00, STA_ZP, Z_PTR]);
    ppu_addr(&mut a, 0x20, 0x00);
    a.e(&[LDX_IMM, 0x04]);
    a.label("page");
    a.e(&[LDY_IMM, 0x00]);
    a.label("copy");
    a.e(&[LDA_IZY, Z_PTR, STA_ABS, 0x07, 0x20, 0xc8]); // LDA (ptr),Y; STA $2007; INY
    a.br(BNE, "copy");
    a.e(&[INC_ZP, Z_PTR + 1, 0xca]); // INC ptr+1; DEX
    a.br(BNE, "page");
    ppu_addr(&mut a, 0x20, 0x00);
    a.e(&[STA_ABS, 0x05, 0x20, STA_ABS, 0x05, 0x20]); // A is 0 here
    a.e(&[STA_ZP, Z_LOADING, STA_ZP, Z_NMI, 0x60]);
    // ---- build_palette: the sixteen bytes for the next frame, and the mask
    a.label("build_palette");
    a.e(&[LDX_ZP, Z_SCREEN, CPX_IMM, 0x01]);
    a.br(BEQ, "pal_pages");
    a.e(&[CPX_IMM, 0x02]);
    a.br(BEQ, "pal_bars");
    a.e(&[LDX_IMM, 0x00]);
    a.label("pal_fixed");
    a.e(&[LDA_ABX, A_FIXED_PAL as u8, (A_FIXED_PAL >> 8) as u8, STA_ZPX, Z_PAL, 0xe8, CPX_IMM, 0x10]);
    a.br(BNE, "pal_fixed");
    a.e(&[LDA_IMM, 0x0a, STA_ZP, Z_MASK]);
    a.e(&[LDA_ZP, Z_SCREEN, CMP_IMM, 0x06]);
    a.br(BNE, "pal_done");
    a.e(&[LDA_ZP, Z_PAD]); // the pad screen's field: any button, green; none, grey
    a.br(BEQ, "field_grey");
    a.e(&[LDA_IMM, 0x2a, STA_ZP, Z_PAL + 1, 0x60]);
    a.label("field_grey");
    a.e(&[LDA_IMM, 0x00, STA_ZP, Z_PAL + 1]);
    a.label("pal_done");
    a.e(&[0x60]);
    a.label("pal_pages"); // page = variant / 8; X = page * 16; emphasis = variant & 7
    a.e(&[LDA_ZP, Z_VARIANT, AND_IMM, 0xf8, ASL_A, 0xaa, LDY_IMM, 0x00]);
    a.label("pp");
    a.e(&[LDA_ABX, A_PAL_PAGES as u8, (A_PAL_PAGES >> 8) as u8, STA_ZPY, Z_PAL, 0x00, 0xe8, 0xc8, CPY_IMM, 0x10]);
    a.br(BNE, "pp");
    a.e(&[LDA_ZP, Z_VARIANT, AND_IMM, 0x07, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, ORA_IMM, 0x0a, STA_ZP, Z_MASK, 0x60]);
    a.label("pal_bars"); // X = variant * 16
    a.e(&[LDA_ZP, Z_VARIANT, ASL_A, ASL_A, ASL_A, ASL_A, 0xaa, LDY_IMM, 0x00]);
    a.label("pb");
    a.e(&[LDA_ABX, A_BARS_PAL as u8, (A_BARS_PAL >> 8) as u8, STA_ZPY, Z_PAL, 0x00, 0xe8, 0xc8, CPY_IMM, 0x10]);
    a.br(BNE, "pb");
    a.e(&[LDA_IMM, 0x0a, STA_ZP, Z_MASK, 0x60]);
    // ---- build_strip: ninety tiles into the buffer, a bit a block
    a.label("build_strip");
    a.e(&[LDX_IMM, 0x00]);
    // row A: sync, screen (3), variant (6), hold (1), parity (1), reserved (1)
    a.e(&[LDA_IMM, SYNC[0] << 5, LDY_IMM, 3]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_SCREEN, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, LDY_IMM, 3]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_VARIANT, ASL_A, ASL_A, LDY_IMM, 6]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_HOLD, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, LDY_IMM, 1]);
    a.jsr("emit");
    // parity of screen ^ variant ^ hold ^ frame lo ^ frame hi ^ pad, folded to one bit
    a.e(&[LDA_ZP, Z_SCREEN, EOR_ZP, Z_VARIANT, EOR_ZP, Z_HOLD, EOR_ZP, Z_FRAME, EOR_ZP, Z_FRAME + 1, EOR_ZP, Z_PAD, STA_ZP, Z_TMP]);
    a.e(&[LSR_A, LSR_A, LSR_A, LSR_A, EOR_ZP, Z_TMP, STA_ZP, Z_TMP, LSR_A, LSR_A, EOR_ZP, Z_TMP, STA_ZP, Z_TMP, LSR_A, EOR_ZP, Z_TMP]);
    a.e(&[ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, ASL_A, LDY_IMM, 1]);
    a.jsr("emit");
    a.e(&[LDA_IMM, 0x00, LDY_IMM, 1]);
    a.jsr("emit");
    // row B: sync, frame bits 11..8, frame bits 7..0
    a.e(&[LDA_IMM, SYNC[1] << 5, LDY_IMM, 3]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_FRAME + 1, ASL_A, ASL_A, ASL_A, ASL_A, LDY_IMM, 4]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_FRAME, LDY_IMM, 8]);
    a.jsr("emit");
    // row C: sync, frame bits 15..12, the pad byte
    a.e(&[LDA_IMM, SYNC[2] << 5, LDY_IMM, 3]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_FRAME + 1, LDY_IMM, 4]);
    a.jsr("emit");
    a.e(&[LDA_ZP, Z_PAD, LDY_IMM, 8]);
    a.jsr("emit");
    a.e(&[0x60]);
    // emit: Y bits of A, MSB first, two tiles each (white = tile 3)
    a.label("emit");
    a.e(&[STA_ZP, Z_TMP]);
    a.label("emit_bit");
    a.e(&[ASL_ZP, Z_TMP, LDA_IMM, 0x00, ADC_IMM, 0x00]);
    a.br(BEQ, "emit_z");
    a.e(&[LDA_IMM, T_C3]);
    a.label("emit_z");
    a.e(&[STA_ZPX, Z_BUF, STA_ZPX, Z_BUF + 1, 0xe8, 0xe8, 0x88]); // INX; INX; DEY
    a.br(BNE, "emit_bit");
    a.e(&[0x60]);
    // ---- NMI: the frame counter; then, unless a load is under way, the
    // pad, the palette, the strip, the scroll and the mask
    a.label("nmi");
    a.e(&[0x48, 0x8a, 0x48, 0x98, 0x48]); // PHA; TXA; PHA; TYA; PHA
    a.e(&[INC_ZP, Z_FRAME]);
    a.br(BNE, "nc");
    a.e(&[INC_ZP, Z_FRAME + 1]);
    a.label("nc");
    a.e(&[LDA_ZP, Z_LOADING]);
    a.br(BEQ, "nmi_body"); // the body is a thousand bytes: too far for a branch
    a.jmp("nmi_exit");
    a.label("nmi_body");
    a.e(&[LDA_ZP, Z_PAD, STA_ZP, Z_PREV]);
    a.e(&[LDA_IMM, 0x01, STA_ABS, 0x16, 0x40, LDA_IMM, 0x00, STA_ABS, 0x16, 0x40, LDX_IMM, 0x08]);
    a.label("poll");
    a.e(&[LDA_ABS, 0x16, 0x40, LSR_A, ROR_ZP, Z_PAD, 0xca]);
    a.br(BNE, "poll");
    ppu_addr(&mut a, 0x3f, 0x00);
    for i in 0..16u8 {
        a.e(&[LDA_ZP, Z_PAL + i, STA_ABS, 0x07, 0x20]);
    }
    for r in 0..2 * STRIP_ROWS {
        let addr = 0x2000 + ((STRIP_Y0 / 8 + r) * 32 + STRIP_X0 / 8) as u16;
        ppu_addr(&mut a, (addr >> 8) as u8, addr as u8);
        let buf = Z_BUF + ((r / 2) * 2 * STRIP_COLS) as u8;
        for i in 0..2 * STRIP_COLS as u8 {
            a.e(&[LDA_ZP, buf + i, STA_ABS, 0x07, 0x20]);
        }
    }
    ppu_addr(&mut a, 0x20, 0x00);
    a.e(&[STA_ABS, 0x05, 0x20, STA_ABS, 0x05, 0x20]);
    a.e(&[LDA_ZP, Z_MASK, STA_ABS, 0x01, 0x20]);
    a.label("nmi_exit");
    a.e(&[LDA_IMM, 0x01, STA_ZP, Z_NMI]);
    a.e(&[0x68, 0xa8, 0x68, 0xaa, 0x68, 0x40]); // PLA; TAY; PLA; TAX; PLA; RTI
    a.label("irq");
    a.e(&[0x40]);
    assert!(a.pc() <= A_PAL_PAGES, "the code runs into the data at {:04x}", A_PAL_PAGES);
    // ---- data
    a.pad_to(A_PAL_PAGES);
    for page in 0..7 {
        a.e(&palette_page(page));
    }
    a.pad_to(A_BARS_PAL);
    for v in 0..8 {
        a.e(&bars_palette(v));
    }
    a.pad_to(A_FIXED_PAL);
    a.e(&FIXED_PAL);
    a.pad_to(A_VPERIOD);
    a.e(&VARIANT_FRAMES);
    a.pad_to(A_VCOUNT);
    a.e(&VARIANTS);
    a.pad_to(A_NAMETABLES);
    for s in 0..SCREENS {
        a.e(&nametable(s));
    }
    let nmi = a.labels["nmi"];
    let reset = a.labels["reset"];
    let irq = a.labels["irq"];
    let mut p = a.finish();
    p.resize(0x8000, 0xea);
    p[0x7ffa..0x8000].copy_from_slice(&[nmi as u8, (nmi >> 8) as u8, reset as u8, (reset >> 8) as u8, irq as u8, (irq >> 8) as u8]);
    p
}

/// The fixed palette of screens 0 and 3 to 7: palette 0 white, red,
/// green; palette 1 blue, red, green; palette 2 orange, blue, white;
/// palette 3 the strip's, with colour 1 white as well so the geometry
/// screen's border draws through the strip rows.
const FIXED_PAL: [u8; 16] = [BLACK, WHITE, 0x16, 0x2a, BLACK, 0x12, 0x16, 0x2a, BLACK, 0x26, 0x12, WHITE, BLACK, WHITE, BLACK, WHITE];

/// The palette screen's eight patches on page `page` (0 to 6): pages 0
/// to 5 show hues 2p+1 and 2p+2 at the four lumas, page 6 the greys
/// (hue 0) and the $xD column, with $0D, the black below black that
/// upsets a sync separator, replaced by $0F.
pub fn palette_entries(page: usize) -> [u8; 8] {
    let mut e = [0u8; 8];
    for k in 0..8 {
        let (row, luma) = (k / 4, k % 4);
        e[k] = if page < 6 {
            (luma as u8) << 4 | (2 * page + 1 + row) as u8
        } else if row == 0 {
            (luma as u8) << 4
        } else if luma == 0 {
            BLACK
        } else {
            (luma as u8) << 4 | 0x0d
        };
    }
    e
}

fn palette_page(page: usize) -> [u8; 16] {
    let e = palette_entries(page);
    [BLACK, e[0], e[1], e[2], BLACK, e[3], e[4], e[5], BLACK, e[6], e[7], BLACK, BLACK, BLACK, BLACK, WHITE]
}

/// The bars screen's eleven hue slots on variant `v` (0 to 7): luma v/2,
/// hues 1 to 11 on even variants and 2 to 12 on odd, so every hue shows
/// at every luma across the eight.
pub fn bars_entries(v: usize) -> [u8; 11] {
    let (luma, half) = (v / 2, v % 2);
    let mut e = [0u8; 11];
    for (k, x) in e.iter_mut().enumerate() {
        *x = (luma as u8) << 4 | (k + 1 + half) as u8;
    }
    e
}

fn bars_palette(v: usize) -> [u8; 16] {
    let e = bars_entries(v);
    [BLACK, e[0], e[1], e[2], BLACK, e[3], e[4], e[5], BLACK, e[6], e[7], e[8], BLACK, e[9], e[10], WHITE]
}

// ------------------------------------------------------------- screens
/// Tile and palette at (tx, ty) of screen `s`, content only (rows from
/// CONTENT_ROW); the strip rows are laid by the common frame.
fn content(s: usize, tx: usize, ty: usize) -> (u8, u8) {
    let y = ty - CONTENT_ROW; // 0..22
    match s {
        1 => {
            // eight patches, 8 tiles wide and 10 tall, two rows of four
            if y >= 20 {
                return (T_BLANK, 0);
            }
            let k = (y / 10) * 4 + tx / 8;
            ((k % 3) as u8 + 1, (k / 3) as u8)
        }
        2 => {
            // 4 x 4 cells, eight across and five down, slot (r*8+c) mod 12
            if y >= 20 {
                return (T_BLANK, 0);
            }
            let k = ((y / 4) * 8 + tx / 4) % 12;
            if k == 11 {
                (T_BLANK, 0)
            } else {
                ((k % 3) as u8 + 1, (k / 3) as u8)
            }
        }
        3 => {
            let band = tx / 8;
            if y < 8 {
                ([T_V1, T_V2, T_V4, if tx % 2 == 0 { T_C1 } else { T_BLANK }][band], 0)
            } else if y < 16 {
                ([T_H1, T_H2, T_H4, if y % 2 == 0 { T_C1 } else { T_BLANK }][band], 0)
            } else {
                (if tx < 16 { T_CHK1 } else { T_CHK2 }, 0)
            }
        }
        4 => {
            let half = (tx / 8) % 2;
            if y < 10 {
                (if half == 1 { T_C1 } else { T_BLANK }, 0)
            } else if y < 20 {
                (if half == 0 { T_C2 } else { T_C3 }, 0)
            } else {
                (T_BLANK, 0)
            }
        }
        5 => {
            if tx < 16 {
                (T_CHK1, 1)
            } else {
                (T_CHK2, 2)
            }
        }
        6 => (if y < 20 { T_C1 } else { T_BLANK }, 0),
        _ => (T_BLANK, 0),
    }
}

/// The geometry screen's border and crosshair, over the whole picture.
fn geometry(tx: usize, ty: usize) -> Option<u8> {
    let tick_x = tx % 4 == 0 && tx != 0;
    let tick_y = ty % 4 == 0 && ty != 0;
    match (tx, ty) {
        (0, 0) => Some(T_TL),
        (31, 0) => Some(T_TR),
        (0, 29) => Some(T_BL),
        (31, 29) => Some(T_BR),
        (_, 0) => Some(if tick_x { T_C1 } else { T_HTOP }),
        (_, 29) => Some(if tick_x { T_C1 } else { T_HBOT }),
        (0, _) => Some(if tick_y { T_C1 } else { T_VLEFT }),
        (31, _) => Some(if tick_y { T_C1 } else { T_VRIGHT }),
        (16, 15) => Some(T_TL),
        (16, y) if y >= CONTENT_ROW => Some(T_VLEFT),
        (_, 15) => Some(T_HTOP),
        _ => None,
    }
}

/// Tile and palette at (tx, ty) of screen `s`, the whole picture.
fn cell(s: usize, tx: usize, ty: usize) -> (u8, u8) {
    if ty < CONTENT_ROW {
        // the strip rows, palette 3; tiles written by the NMI
        if s == 7 {
            if let Some(t) = geometry(tx, ty) {
                return (t, 3);
            }
        }
        return (T_BLANK, 3);
    }
    if s == 7 {
        return (geometry(tx, ty).unwrap_or(T_BLANK), 0);
    }
    content(s, tx, ty)
}

/// Nametable and attributes of screen `s`: 960 tiles then 64 bytes.
pub fn nametable(s: usize) -> Vec<u8> {
    let mut n = Vec::with_capacity(1024);
    for ty in 0..30 {
        for tx in 0..32 {
            n.push(cell(s, tx, ty).0);
        }
    }
    for ay in 0..8 {
        for ax in 0..8 {
            let mut b = 0u8;
            for (q, (dx, dy)) in [(0, 0), (2, 0), (0, 2), (2, 2)].iter().enumerate() {
                let (tx, ty) = (ax * 4 + dx, ay * 4 + dy);
                let pal = if ty < 30 { cell(s, tx, ty).1 } else { 0 };
                b |= pal << (2 * q);
            }
            n.push(b);
        }
    }
    // Every 2x2 quadrant is one palette: the layouts above are on even
    // tile boundaries, asserted here so a change cannot tear one.
    for ty in (0..30).step_by(2) {
        for tx in (0..32).step_by(2) {
            let p = cell(s, tx, ty).1;
            for (dx, dy) in [(1, 0), (0, 1), (1, 1)] {
                let q = cell(s, tx + dx, ty + dy);
                assert!(q.0 == T_BLANK || q.1 == p, "screen {s}: tile ({},{}) needs palette {} in a quadrant of palette {p}", tx + dx, ty + dy, q.1);
            }
        }
    }
    n
}

/// CHR: the tiles named above, colour 1 in plane 0 unless said.
pub fn chr() -> Vec<u8> {
    let mut c = vec![0u8; 0x2000];
    let mut tile = |i: u8, p0: [u8; 8], p1: [u8; 8]| {
        let o = i as usize * 16;
        c[o..o + 8].copy_from_slice(&p0);
        c[o + 8..o + 16].copy_from_slice(&p1);
    };
    let f = [0xffu8; 8];
    let z = [0u8; 8];
    tile(T_C1, f, z);
    tile(T_C2, z, f);
    tile(T_C3, f, f);
    tile(T_V1, [0xaa; 8], z);
    tile(T_V2, [0xcc; 8], z);
    tile(T_V4, [0xf0; 8], z);
    tile(T_H1, [0xff, 0, 0xff, 0, 0xff, 0, 0xff, 0], z);
    tile(T_H2, [0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0], z);
    tile(T_H4, [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0], z);
    tile(T_CHK1, [0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55], z);
    tile(T_CHK2, [0xcc, 0xcc, 0x33, 0x33, 0xcc, 0xcc, 0x33, 0x33], z);
    tile(T_HTOP, [0xff, 0, 0, 0, 0, 0, 0, 0], z);
    tile(T_HBOT, [0, 0, 0, 0, 0, 0, 0, 0xff], z);
    tile(T_VLEFT, [0x80; 8], z);
    tile(T_VRIGHT, [0x01; 8], z);
    tile(T_TL, [0xff, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80], z);
    tile(T_TR, [0xff, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01], z);
    tile(T_BL, [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xff], z);
    tile(T_BR, [0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0xff], z);
    c
}

// ------------------------------------------------------------ manifest
/// A rectangle of the picture worth measuring, in dots, and what is in it.
pub struct Region {
    pub name: String,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub what: What,
}

pub enum What {
    /// A flat field: the palette entry per variant (None where the
    /// backdrop shows), and the emphasis bits per variant.
    Flat { entries: Vec<Option<u8>> },
    /// A pattern in colour 1 (white) on the backdrop: `pitch` dots of
    /// white then `pitch` of black along `axis` (x, y, or xy for a
    /// checkerboard).
    Pattern { kind: &'static str, pitch: usize, axis: &'static str },
}

/// Every region of every screen, from the same functions that lay the
/// tiles. Flat regions are the largest rectangles of one tile and
/// palette; patterns are named per band.
pub fn regions(s: usize) -> Vec<Region> {
    let mut out = Vec::new();
    match s {
        1 => {
            for k in 0..8 {
                let (tx, ty) = ((k % 4) * 8, CONTENT_ROW + (k / 4) * 10);
                flat(&mut out, s, format!("patch{k}"), tx, ty, 8, 10);
            }
        }
        2 => {
            for r in 0..5 {
                for c in 0..8 {
                    flat(&mut out, s, format!("cell{r}{c}"), c * 4, CONTENT_ROW + r * 4, 4, 4);
                }
            }
        }
        3 => {
            for (band, (kind, pitch)) in [("v1", 1usize), ("v2", 2), ("v4", 4), ("v8", 8)].into_iter().enumerate() {
                out.push(Region { name: kind.to_string(), x: band * 64, y: CONTENT_ROW * 8, w: 64, h: 64, what: What::Pattern { kind, pitch, axis: "x" } });
            }
            for (band, (kind, pitch)) in [("h1", 1usize), ("h2", 2), ("h4", 4), ("h8", 8)].into_iter().enumerate() {
                out.push(Region { name: kind.to_string(), x: band * 64, y: (CONTENT_ROW + 8) * 8, w: 64, h: 64, what: What::Pattern { kind, pitch, axis: "y" } });
            }
            out.push(Region { name: "chk1".into(), x: 0, y: (CONTENT_ROW + 16) * 8, w: 128, h: 48, what: What::Pattern { kind: "chk1", pitch: 1, axis: "xy" } });
            out.push(Region { name: "chk2".into(), x: 128, y: (CONTENT_ROW + 16) * 8, w: 128, h: 48, what: What::Pattern { kind: "chk2", pitch: 2, axis: "xy" } });
        }
        4 => {
            for half in 0..4 {
                flat(&mut out, s, format!("luma{half}"), half * 8, CONTENT_ROW, 8, 10);
                flat(&mut out, s, format!("chroma{half}"), half * 8, CONTENT_ROW + 10, 8, 10);
            }
        }
        5 => {
            out.push(Region { name: "crawl1".into(), x: 0, y: CONTENT_ROW * 8, w: 128, h: 176, what: What::Pattern { kind: "chk1", pitch: 1, axis: "xy" } });
            out.push(Region { name: "crawl2".into(), x: 128, y: CONTENT_ROW * 8, w: 128, h: 176, what: What::Pattern { kind: "chk2", pitch: 2, axis: "xy" } });
        }
        6 => flat(&mut out, s, "field".into(), 0, CONTENT_ROW, 32, 20),
        _ => {}
    }
    out
}

/// A flat region: the tile and palette at its top-left corner say what
/// it holds on each variant, through the same palette tables the
/// program carries.
fn flat(out: &mut Vec<Region>, s: usize, name: String, tx: usize, ty: usize, tw: usize, th: usize) {
    let (tile, pal) = content(s, tx, ty);
    let entries = (0..VARIANTS[s] as usize)
        .map(|v| match (s, tile) {
            (_, T_BLANK) => None,
            (1, t) => Some(palette_entries(v / 8)[(pal * 3 + t - 1) as usize]),
            (2, t) => Some(bars_entries(v)[(pal * 3 + t - 1) as usize]),
            (6, T_C1) => None, // the field: the pad decides
            (_, t) => Some(FIXED_PAL[(pal * 4 + t) as usize]),
        })
        .collect();
    out.push(Region { name, x: tx * 8, y: ty * 8, w: tw * 8, h: th * 8, what: What::Flat { entries } });
}

/// The strip's fields: (row, first block, bits, name, fixed value if any).
pub const FIELDS: [(usize, usize, usize, &str, Option<u8>); 12] = [
    (0, 0, 3, "sync", Some(SYNC[0])),
    (0, 3, 3, "screen", None),
    (0, 6, 6, "variant", None),
    (0, 12, 1, "hold", None),
    (0, 13, 1, "parity", None),
    (0, 14, 1, "reserved", Some(0)),
    (1, 0, 3, "sync", Some(SYNC[1])),
    (1, 3, 4, "frame_11_8", None),
    (1, 7, 8, "frame_7_0", None),
    (2, 0, 3, "sync", Some(SYNC[2])),
    (2, 3, 4, "frame_15_12", None),
    (2, 7, 8, "pad", None),
];

/// The manifest as JSON text: the strip's geometry and fields, and every
/// screen's regions with what each holds per variant.
pub fn manifest() -> String {
    let mut j = String::new();
    j.push_str("{\n  \"cartridge\": \"cal\",\n  \"mapper\": 0,\n  \"prg\": 32768,\n  \"chr\": 8192,\n  \"mirroring\": \"vertical\",\n");
    j.push_str(&format!(
        "  \"strip\": {{\n    \"x0\": {STRIP_X0}, \"y0\": {STRIP_Y0}, \"block\": {BLOCK}, \"cols\": {STRIP_COLS}, \"rows\": {STRIP_ROWS},\n    \"white\": {WHITE}, \"black\": {BLACK},\n    \"note\": \"a block is white when its bit is 1; bits read MSB first left to right; the pad byte has A at bit 0; the frame counter counts NMIs since reset; a byte polled in the blanking that ends frame k is in the strip of frame k+2; parity is the XOR of screen, variant, hold, both frame bytes and pad, folded to one bit\",\n    \"fields\": [\n"
    ));
    for (i, (row, col, bits, name, value)) in FIELDS.iter().enumerate() {
        j.push_str(&format!("      {{\"row\": {row}, \"col\": {col}, \"bits\": {bits}, \"name\": \"{name}\"{}}}{}\n", value.map(|v| format!(", \"value\": {v}")).unwrap_or_default(), if i + 1 < FIELDS.len() { "," } else { "" }));
    }
    j.push_str("    ]\n  },\n  \"screens\": [\n");
    for s in 0..SCREENS {
        j.push_str(&format!(
            "    {{\"id\": {s}, \"name\": \"{}\", \"variant_frames\": {}, \"variants\": {}, \"emphasis\": \"{}\", \"regions\": [\n",
            SCREEN_NAMES[s],
            VARIANT_FRAMES[s],
            VARIANTS[s],
            if s == 1 { "variant % 8" } else { "0" }
        ));
        let rs = regions(s);
        for (i, r) in rs.iter().enumerate() {
            let what = match &r.what {
                What::Flat { entries } => format!("\"entries\": [{}]", entries.iter().map(|e| e.map(|v| v.to_string()).unwrap_or_else(|| "null".into())).collect::<Vec<_>>().join(", ")),
                What::Pattern { kind, pitch, axis } => format!("\"pattern\": \"{kind}\", \"pitch\": {pitch}, \"axis\": \"{axis}\", \"colour\": {WHITE}"),
            };
            j.push_str(&format!("      {{\"name\": \"{}\", \"x\": {}, \"y\": {}, \"w\": {}, \"h\": {}, {what}}}{}\n", r.name, r.x, r.y, r.w, r.h, if i + 1 < rs.len() { "," } else { "" }));
        }
        j.push_str(&format!("    ]}}{}\n", if s + 1 < SCREENS { "," } else { "" }));
    }
    j.push_str("  ]\n}\n");
    j
}

// -------------------------------------------------------------- reader
/// What a frame's strip says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strip {
    pub screen: u8,
    pub variant: u8,
    pub hold: bool,
    pub frame: u16,
    pub pad: u8,
}

/// One block of the model's frame, sampled over its middle 8 by 8 dots:
/// white, black, or neither (a refusal, never a guess). `dx`, `dy` shift
/// the sampling grid, for the mutation.
fn block(f: &DotFrame, row: usize, col: usize, dx: isize, dy: isize) -> Result<bool, String> {
    let x0 = (STRIP_X0 + col * BLOCK + 4) as isize + dx;
    let y0 = (STRIP_Y0 + row * BLOCK + 4) as isize + dy;
    let (mut white, mut black) = (0, 0);
    for y in y0..y0 + 8 {
        for x in x0..x0 + 8 {
            match f.at(y as usize, x as usize + nes_bus::ACTIVE_FIRST_DOT).0 {
                WHITE => white += 1,
                BLACK => black += 1,
                _ => {}
            }
        }
    }
    match (white, black) {
        (64, 0) => Ok(true),
        (0, 64) => Ok(false),
        _ => Err(format!("block ({row},{col}) is neither white nor black: {white} white, {black} black of 64")),
    }
}

/// Read the strip off a frame of the model, by the manifest's rectangles.
pub fn read_strip(f: &DotFrame) -> Result<Strip, String> {
    read_strip_shifted(f, 0, 0)
}

pub fn read_strip_shifted(f: &DotFrame, dx: isize, dy: isize) -> Result<Strip, String> {
    let mut v: HashMap<&str, u32> = HashMap::new();
    for (row, col, bits, name, fixed) in FIELDS {
        let mut x = 0u32;
        for b in 0..bits {
            x = x << 1 | block(f, row, col + b, dx, dy)? as u32;
        }
        if let Some(want) = fixed {
            if x != want as u32 {
                return Err(format!("{name} on row {row} reads {x:0b}, not {want:0b}"));
            }
        }
        v.insert(name, x);
    }
    let frame = (v["frame_15_12"] << 12 | v["frame_11_8"] << 8 | v["frame_7_0"]) as u16;
    let s = Strip { screen: v["screen"] as u8, variant: v["variant"] as u8, hold: v["hold"] != 0, frame, pad: v["pad"] as u8 };
    let mut p = s.screen ^ s.variant ^ s.hold as u8 ^ frame as u8 ^ (frame >> 8) as u8 ^ s.pad;
    p ^= p >> 4;
    p ^= p >> 2;
    p ^= p >> 1;
    if (p & 1) as u32 != v["parity"] {
        return Err(format!("parity: the strip says {}, the fields fold to {}", v["parity"], p & 1));
    }
    Ok(s)
}
