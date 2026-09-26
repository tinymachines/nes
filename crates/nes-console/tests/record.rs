//! The recording and its trace (`record`): a run logged live replays to
//! the same pictures, a log that says anything else is refused, and the
//! trace the replay writes says what the CPU did in terms a reader can
//! hold it to.
//!
//! The program reads the pad in its NMI handler and paints the backdrop
//! from what it read, so the pictures depend on the inputs: a replay that
//! dropped or moved an input would make a different picture, and the
//! tampered log below is refused for exactly that.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::record::{self, Played, Replay};
use nes_console::{Alignment, Console};
use nes_glue::controller::Buttons;

const RESET_AT: usize = 0x4100; // $C100
const NMI_AT: usize = 0x4120; // $C120

fn prg() -> Vec<u8> {
    let mut prg = vec![0xffu8; 0x8000];
    let reset: &[u8] = &[
        0x78, // SEI
        0xa2, 0xff, 0x9a, // LDX #$FF; TXS
        0xa9, 0x00, 0x8d, 0x00, 0x20, // LDA #0; STA $2000
        0x2c, 0x02, 0x20, 0x10, 0xfb, // BIT $2002; BPL
        0x2c, 0x02, 0x20, 0x10, 0xfb, // twice
        0xa9, 0x80, 0x8d, 0x00, 0x20, // LDA #$80; STA $2000 (NMI on)
        0x4c, 0x18, 0xc1, // JMP $C118, to itself
    ];
    let nmi: &[u8] = &[
        0xa9, 0x01, 0x8d, 0x16, 0x40, // LDA #1; STA $4016
        0xa9, 0x00, 0x8d, 0x16, 0x40, // LDA #0; STA $4016
        0xa2, 0x08, // LDX #8
        0xad, 0x16, 0x40, 0x4a, 0x26, 0x10, 0xca, 0xd0, 0xf7, // LDA $4016; LSR; ROL $10; DEX; BNE
        0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, // $2006 <- $3F00
        0xa5, 0x10, 0x29, 0x3f, 0x8d, 0x07, 0x20, // the backdrop <- the pad's low six
        0xa9, 0x00, 0x8d, 0x06, 0x20, 0x8d, 0x06, 0x20, // $2006 <- $0000
        0x40, // RTI
    ];
    prg[RESET_AT..RESET_AT + reset.len()].copy_from_slice(reset);
    prg[NMI_AT..NMI_AT + nmi.len()].copy_from_slice(nmi);
    prg[0x7ffa..0x7ffe].copy_from_slice(&[0x20, 0xc1, 0x00, 0xc1]);
    prg
}

fn console() -> Console {
    let cart = Nrom::new(prg(), vec![0u8; 0x2000], Mirroring::Vertical).expect("NROM");
    Console::new(Box::new(cart), None, Alignment::default())
}

/// A run played live, as the page plays one: pads set between batches of
/// frames, the reset button once. Returns the log and every digest.
fn live() -> (Vec<u8>, Vec<u32>) {
    let mut c = console();
    c.inputs = Some(Default::default());
    let mut digests = Vec::new();
    let mut run = |c: &mut Console, n: usize| {
        c.run_frames(n);
        digests.extend(c.frames.drain(..).map(|f| record::frame_digest(&f)));
    };
    for (i, pad) in [0x00u8, 0x80, 0x80, 0x90, 0x10, 0x01, 0x00, 0x02].into_iter().enumerate() {
        c.set_pad(0, Buttons::from_byte(pad));
        run(&mut c, 3 + i);
        if i == 4 {
            c.reset_button(89_342 * 8);
        }
    }
    run(&mut c, 2);
    let m = c.master;
    let mut log = c.inputs.take().unwrap();
    log.push(record::END, 0, 0, 0, m);
    (log.bytes, digests)
}

fn replay(log: &[u8]) -> Result<(Console, u64), String> {
    let mut c = console();
    c.trace = Some(Default::default());
    let mut r = Replay::new(log)?;
    while r.run(&mut c, 7)? == Played::More {}
    Ok((c, r.frames_checked()))
}

#[test]
fn a_run_logged_live_replays_to_the_same_pictures() {
    let (log, digests) = live();
    let distinct: std::collections::HashSet<_> = digests.iter().collect();
    assert!(distinct.len() >= 4, "the pads must change the pictures, or a replay proves nothing: {} distinct", distinct.len());
    let events = record::events(&log).unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == record::FRAME).count(), digests.len());
    assert_eq!(events.iter().filter(|e| e.kind == record::RESET).count(), 1);
    // Eight sets, one of them a repeat: seven changes are events.
    assert_eq!(events.iter().filter(|e| e.kind == record::PAD).count(), 7);

    let (_, checked) = replay(&log).expect("the log replays");
    assert_eq!(checked, digests.len() as u64);
}

#[test]
fn a_log_that_says_anything_else_is_refused() {
    let (log, _) = live();
    let events = record::events(&log).unwrap();
    // A pad event's value changed: the pictures part from the log's.
    let i = events.iter().position(|e| e.kind == record::PAD && e.value == 0x90).unwrap();
    let mut bad = log.clone();
    bad[i * record::EVENT_BYTES + 2] = 0x88;
    let err = replay(&bad).err().expect("a changed pad is refused");
    assert!(err.contains("left the recording"), "{err}");
    // The reset moved a frame later.
    let i = events.iter().position(|e| e.kind == record::RESET).unwrap();
    let j = i + events[i..].iter().position(|e| e.kind == record::FRAME).unwrap();
    let mut bad: Vec<u8> = log.clone();
    let reset = bad[i * 16..i * 16 + 16].to_vec();
    bad.copy_within((i + 1) * 16..(j + 1) * 16, i * 16);
    bad[j * 16..j * 16 + 16].copy_from_slice(&reset);
    bad[j * 16 + 8..j * 16 + 16].copy_from_slice(&events[j].master.to_le_bytes());
    assert!(replay(&bad).is_err(), "a moved reset is refused");
    // No END: a recording cut short.
    assert!(replay(&log[..log.len() - 16]).err().unwrap().contains("no END"));
    // Not a log at all.
    assert!(replay(&log[..log.len() - 3]).is_err());
}

#[test]
fn the_trace_says_what_the_cpu_did() {
    let (log, digests) = live();
    let (c, _) = replay(&log).unwrap();
    let t = c.trace.unwrap().bytes;
    assert_eq!(t.len() % record::RECORD_BYTES, 0);
    let prg = prg();
    let recs: Vec<&[u8]> = t.chunks_exact(8).collect();
    let (mut cycles, mut frames, mut inputs, mut syncs, mut ldas) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut lda_pending: Option<u8> = None;
    let mut lda_operand_next = false;
    for (i, r) in recs.iter().enumerate() {
        match r[3] & 0xc0 {
            record::KIND_CYCLE => {
                cycles += 1;
                let flags = r[3] & 0x3f;
                let ab = u16::from_le_bytes([r[0], r[1]]);
                let prg_at = u32::from_le_bytes([r[4], r[5], r[6], r[7]]);
                if flags & record::F_READ != 0 && ab >= 0x8000 {
                    assert_eq!(prg_at as usize, (ab as usize - 0x8000) + 1, "record {i}: ${ab:04x}");
                    assert_eq!(prg[prg_at as usize - 1], r[2], "record {i}: the byte read is the ROM's");
                } else {
                    assert_eq!(prg_at, 0, "record {i}: ${ab:04x} is not the ROM");
                }
                if lda_operand_next {
                    lda_pending = Some(r[2]);
                    lda_operand_next = false;
                }
                if flags & record::F_SYNC != 0 && flags & record::F_HELD == 0 {
                    syncs += 1;
                    assert_eq!(recs[i + 1][3] & 0xc0, record::KIND_REGS, "record {i}: an opcode fetch is followed by the registers");
                    // The registers entering this instruction hold the
                    // LDA #imm before it, if that is what it was.
                    if let Some(v) = lda_pending.take() {
                        assert_eq!(recs[i + 1][0], v, "record {i}: A after LDA #${v:02x}");
                        ldas += 1;
                    }
                    lda_operand_next = r[2] == 0xa9;
                }
            }
            record::KIND_REGS => {}
            record::KIND_INPUT => inputs += 1,
            _ => frames += 1,
        }
    }
    assert_eq!(frames, digests.len());
    // The log has seven pad events (its first is the pad as first set);
    // the trace records a change to the pad, and the first set is the
    // pad the console powered on with: six, and the reset.
    assert_eq!(inputs, 6 + 1, "six pad changes and the reset");
    assert!(ldas > 100, "the LDA #imm check ran: {ldas}");
    // A frame is 341 x 262 dots, three to a CPU cycle, less the odd
    // frames' skipped dot: 29,780 and two thirds cycles, or near it.
    let per = cycles as f64 / frames as f64;
    assert!((per - 29_780.67).abs() < 1.0, "{per} cycles a frame");
    assert!(syncs > frames * 1000, "{syncs} instructions");
}
