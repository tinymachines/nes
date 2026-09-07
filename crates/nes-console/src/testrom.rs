//! A cartridge built in code for the plumbing gate and its traces: a
//! program that paints through the PPU's registers, counts NMIs in RAM
//! and reads the controller on request.

/// A program at $8000: palette, a nametable fill, rendering on, NMI on,
/// then a spin; the NMI handler bumps $00 and, when $01 is nonzero,
/// strobes and reads the controller into $02.
pub fn program() -> Vec<u8> {
    let mut p: Vec<u8> = Vec::new();
    let w = |p: &mut Vec<u8>, bytes: &[u8]| p.extend_from_slice(bytes);
    // SEI; $4017 <- $40 (no frame IRQs); $2000 <- 0 (no NMI yet);
    // $00..$02 <- 0 (RAM powers on as the SRAM's fill, not zero).
    w(&mut p, &[0x78, 0xa9, 0x40, 0x8d, 0x17, 0x40, 0xa9, 0x00, 0x8d, 0x00, 0x20, 0x85, 0x00, 0x85, 0x01, 0x85, 0x02]);
    // wait two vblanks: BIT $2002 / BPL -5, twice
    for _ in 0..2 {
        w(&mut p, &[0x2c, 0x02, 0x20, 0x10, 0xfb]);
    }
    // palette: $3F00 <- 0F 30 16 2A
    w(&mut p, &[0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20]);
    for c in [0x0fu8, 0x30, 0x16, 0x2a] {
        w(&mut p, &[0xa9, c, 0x8d, 0x07, 0x20]);
    }
    // nametable $2000: 960 bytes of tile 1 for rows 4..7 (index 128..255), 0 elsewhere.
    w(&mut p, &[0xa9, 0x20, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20]);
    // X loop: 4 pages: LDX #0; loop: LDA table,X pattern -> use A=(X>=128)?1:0 for page 0, 0 for others.
    // Simpler: write 128 zeros, 128 ones, then 704 zeros via nested loops.
    w(&mut p, &[0xa2, 0x80, 0xa9, 0x00]);              // LDX #$80; LDA #0
    let l1 = p.len();
    w(&mut p, &[0x8d, 0x07, 0x20, 0xca, 0xd0, 0xfa]); // loop: STA $2007; DEX; BNE loop
    let _ = l1;
    w(&mut p, &[0xa2, 0x80, 0xa9, 0x01]);
    w(&mut p, &[0x8d, 0x07, 0x20, 0xca, 0xd0, 0xfa]);
    w(&mut p, &[0xa0, 0x03, 0xa9, 0x00]);              // LDY #3; LDA #0
    w(&mut p, &[0xa2, 0x00]);                          // LDX #0
    w(&mut p, &[0x8d, 0x07, 0x20, 0xca, 0xd0, 0xfa, 0x88, 0xd0, 0xf7]); // inner 256 x3 = 768: the rest of the nametable and its attributes
    // attributes: leave. scroll 0,0
    w(&mut p, &[0xa9, 0x00, 0x8d, 0x05, 0x20, 0x8d, 0x05, 0x20]);
    // $2001 <- $0A (background on, left column on... bit3 bg), $2000 <- $80 (NMI)
    w(&mut p, &[0xa9, 0x0a, 0x8d, 0x01, 0x20, 0xa9, 0x80, 0x8d, 0x00, 0x20]);
    // spin
    let spin = 0x8000 + p.len() as u16;
    w(&mut p, &[0x4c, spin as u8, (spin >> 8) as u8]);
    // NMI handler at $8100: INC $00; LDA $01; BEQ done; LDA #1; STA $4016; LDA #0; STA $4016; LDA $4016; STA $02; done: RTI
    while p.len() < 0x100 {
        p.push(0xea);
    }
    w(&mut p, &[0xe6, 0x00, 0xa5, 0x01, 0xf0, 0x0f, 0xa9, 0x01, 0x8d, 0x16, 0x40, 0xa9, 0x00, 0x8d, 0x16, 0x40, 0xad, 0x16, 0x40, 0x85, 0x02, 0x40]);
    // IRQ handler at $8140: RTI. Pad to 32 KiB; vectors NMI $8100,
    // RESET $8000, IRQ $8140.
    while p.len() < 0x140 {
        p.push(0xea);
    }
    p.push(0x40);
    p.resize(0x8000, 0xea);
    p[0x7ffa] = 0x00;
    p[0x7ffb] = 0x81;
    p[0x7ffc] = 0x00;
    p[0x7ffd] = 0x80;
    p[0x7ffe] = 0x40;
    p[0x7fff] = 0x81;
    p
}

/// The polling cartridge for the bench's B0: the base program's screen,
/// and an NMI handler that polls the pad the way a game does, one
/// strobe and eight reads, the byte assembled into $02 and the poll
/// counted in $00. With `dmc`, the main program also starts a looping
/// DMC sample at the fastest rate from $C000 before the spin, so that
/// fetches land on the poll's reads (the double clock the die shows,
/// `2a03`'s joy-clock-probe, and the part's count for the bridge).
/// Same vectors and CHR as `program`.
pub fn pad_program(dmc: bool) -> Vec<u8> {
    let mut p = program();
    // Replace the spin: the base program's last three bytes before the
    // NOP padding are JMP spin. Find it and, with dmc, insert the DMC
    // start before it; the padding to $8100 has room.
    let spin = p.iter().position(|&b| b == 0xea).expect("padding");
    let jmp = spin - 3;
    assert_eq!(p[jmp], 0x4c, "the spin is where the base program leaves it");
    let mut tail: Vec<u8> = Vec::new();
    if dmc {
        // $4010 <- $4F (loop, rate 15); $4012 <- 0 ($C000); $4013 <- $FF; $4015 <- $10
        for (r, v) in [(0x10u8, 0x4fu8), (0x12, 0x00), (0x13, 0xff), (0x15, 0x10)] {
            tail.extend([0xa9, v, 0x8d, r, 0x40]);
        }
    }
    let spin_at = 0x8000 + jmp as u16 + tail.len() as u16;
    tail.extend([0x4c, spin_at as u8, (spin_at >> 8) as u8]);
    assert!(jmp + tail.len() <= 0x100, "the DMC start fits before the NMI handler");
    p[jmp..jmp + tail.len()].copy_from_slice(&tail);
    // NMI at $8100: INC $00; LDA #1; STA $4016; LDA #0; STA $4016;
    // LDX #8; loop: LDA $4016; LSR; ROL $02; DEX; BNE loop; RTI
    let nmi: [u8; 25] = [0xe6, 0x00, 0xa9, 0x01, 0x8d, 0x16, 0x40, 0xa9, 0x00, 0x8d, 0x16, 0x40, 0xa2, 0x08, 0xad, 0x16, 0x40, 0x4a, 0x26, 0x02, 0xca, 0xd0, 0xf7, 0x40, 0xea];
    p[0x100..0x100 + nmi.len()].copy_from_slice(&nmi);
    // The sample: $C000.. ($4000 into PRG), a ramp, clear of the vectors.
    for i in 0..0x2000usize {
        p[0x4000 + i] = (i as u8).wrapping_mul(7);
    }
    p
}

/// CHR: tile 0 blank, tile 1 solid colour 1 (plane 0 set).
pub fn chr() -> Vec<u8> {
    let mut c = vec![0u8; 0x2000];
    c[16..24].fill(0xff);
    c
}


/// A colour-bars cartridge the repository owns (nobody's game): eight
/// columns of 32-dot cells over the picture, each cell one of the
/// twelve hues at one luma row or the backdrop, the luma row stepping
/// every 120 frames through 1, 2, 3, 0. The cells are as wide as the
/// signal path's chroma settles, which blargg's full_palette bars, at
/// sixteen dots, are not (docs/n6-report.md, the procedure decision).
/// The picture is data at $9000 (the nametable and its attributes) and
/// $9400 (the four palette rows); the program copies it in and spins,
/// the NMI handler counting frames and rewriting the palette in vblank.
pub fn bars_program() -> Vec<u8> {
    let mut p: Vec<u8> = Vec::new();
    let w = |p: &mut Vec<u8>, bytes: &[u8]| p.extend_from_slice(bytes);
    // $8000 reset: SEI; $4017 <- $40; $2000 <- 0; $2001 <- 0; $00 <- 0; $01 <- 1 (luma row 1 first)
    w(&mut p, &[0x78, 0xa9, 0x40, 0x8d, 0x17, 0x40, 0xa9, 0x00, 0x8d, 0x00, 0x20, 0x8d, 0x01, 0x20, 0x85, 0x00, 0xa9, 0x01, 0x85, 0x01]);
    for _ in 0..2 {
        w(&mut p, &[0x2c, 0x02, 0x20, 0x10, 0xfb]); // BIT $2002; BPL -5
    }
    w(&mut p, &[0x20, 0x00, 0x82]); // JSR $8200 (the palette for row $01)
    // nametable: $2006 <- $20, $00; four pages from $9000
    w(&mut p, &[0xa9, 0x20, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20]);
    for page in 0x90u8..0x94 {
        // LDY #0; l: LDA $pp00,Y; STA $2007; INY; BNE l
        w(&mut p, &[0xa0, 0x00, 0xb9, 0x00, page, 0x8d, 0x07, 0x20, 0xc8, 0xd0, 0xf7]);
    }
    // t back to the top left: $2006 <- $20, $00; $2005 <- 0, 0
    w(&mut p, &[0xa9, 0x20, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0x8d, 0x05, 0x20, 0x8d, 0x05, 0x20]);
    // $2001 <- $0A (background on, left column on); $2000 <- $80 (NMI on)
    w(&mut p, &[0xa9, 0x0a, 0x8d, 0x01, 0x20, 0xa9, 0x80, 0x8d, 0x00, 0x20]);
    let spin = 0x8000 + p.len() as u16;
    w(&mut p, &[0x4c, spin as u8, (spin >> 8) as u8]);
    // $8100 NMI: INC $00; LDA $00; CMP #120; BNE done; LDA #0; STA $00;
    // INC $01; LDA $01; AND #3; STA $01; JSR $8200; done: RTI
    while p.len() < 0x100 {
        p.push(0xea);
    }
    w(&mut p, &[0xe6, 0x00, 0xa5, 0x00, 0xc9, 120, 0xd0, 0x0e, 0xa9, 0x00, 0x85, 0x00, 0xe6, 0x01, 0xa5, 0x01, 0x29, 0x03, 0x85, 0x01, 0x20, 0x00, 0x82, 0x40]);
    // $8140 IRQ: RTI
    while p.len() < 0x140 {
        p.push(0xea);
    }
    p.push(0x40);
    // $8200 write_palette: $2006 <- $3F, $00; X <- row * 16; LDY #16;
    // l: LDA $9400,X; STA $2007; INX; DEY; BNE l; then t back: $2006 <-
    // $20, $00; $2005 <- 0, 0; RTS
    while p.len() < 0x200 {
        p.push(0xea);
    }
    w(&mut p, &[0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20]);
    w(&mut p, &[0xa5, 0x01, 0x0a, 0x0a, 0x0a, 0x0a, 0xaa, 0xa0, 0x10]); // LDA $01; ASL x4; TAX; LDY #16
    w(&mut p, &[0xbd, 0x00, 0x94, 0x8d, 0x07, 0x20, 0xe8, 0x88, 0xd0, 0xf6]); // l: LDA $9400,X; STA $2007; INX; DEY; BNE l (ten bytes back)
    w(&mut p, &[0xa9, 0x20, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, 0x8d, 0x05, 0x20, 0x8d, 0x05, 0x20, 0x60]);
    // $9000: the nametable (960 bytes) and its attributes (64), cell by cell.
    while p.len() < 0x1000 {
        p.push(0xea);
    }
    for y in 0..30usize {
        for x in 0..32usize {
            p.push(bars_tile(y / 4, x / 4));
        }
    }
    for r in 0..8usize {
        for c in 0..8usize {
            let pal = bars_palette(r, c);
            p.push(pal | pal << 2 | pal << 4 | pal << 6);
        }
    }
    // $9400: four palette rows, luma 0..3: backdrop $0F, then the twelve
    // hues in the three colours of each of the four palettes.
    assert_eq!(p.len(), 0x1400);
    for luma in 0..4u8 {
        for pal in 0..4u8 {
            p.push(0x0f);
            for k in 0..3u8 {
                p.push(luma << 4 | (pal * 3 + k + 1));
            }
        }
    }
    p.resize(0x8000, 0xea);
    p[0x7ffa] = 0x00;
    p[0x7ffb] = 0x81;
    p[0x7ffc] = 0x00;
    p[0x7ffd] = 0x80;
    p[0x7ffe] = 0x40;
    p[0x7fff] = 0x81;
    p
}

/// Cell (row, column) of the bars picture: slot k = row * 8 + column
/// modulo 13, slot 12 the backdrop, otherwise hue slot / 3 + 1.. of
/// palette slot / 3.
fn bars_slot(r: usize, c: usize) -> usize {
    (r * 8 + c) % 13
}

fn bars_tile(r: usize, c: usize) -> u8 {
    let s = bars_slot(r, c);
    if s == 12 {
        0
    } else {
        (s % 3) as u8 + 1
    }
}

fn bars_palette(r: usize, c: usize) -> u8 {
    let s = bars_slot(r, c);
    if s == 12 {
        0
    } else {
        (s / 3) as u8
    }
}

/// CHR for the bars: tile 0 blank, tiles 1, 2, 3 solid colours 1, 2, 3.
pub fn bars_chr() -> Vec<u8> {
    let mut c = vec![0u8; 0x2000];
    c[16..24].fill(0xff); // tile 1: plane 0
    c[40..48].fill(0xff); // tile 2: plane 1
    c[48..56].fill(0xff); // tile 3: both planes
    c[56..64].fill(0xff);
    c
}
