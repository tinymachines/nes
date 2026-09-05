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

/// CHR: tile 0 blank, tile 1 solid colour 1 (plane 0 set).
pub fn chr() -> Vec<u8> {
    let mut c = vec![0u8; 0x2000];
    c[16..24].fill(0xff);
    c
}

