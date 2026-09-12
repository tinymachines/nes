//! T0 of the bench's trace plan (nes-bench/docs/trace-plan.md): a console
//! run as files the 6502 stack reads.
//!
//!   cargo run --release -p nes-console --example trace -- <rom.nes> <name> [frames] [script.txt] [out_dir]
//!
//! Runs the console from power-on for `frames` frames (default 120) with
//! the bench script's `SET hh` and `AT n hh` lines honoured by latch index
//! (the other words are the head's and are skipped), and writes:
//!
//!   <name>.pins         the CPU at its pins, one line per half-cycle, in
//!                       v6502-pins' own text format: the same contract the
//!                       6502 ladder is verified against. It carries every
//!                       byte the program fetched, so a trace of a
//!                       commercial cartridge is ROM content and stays
//!                       where the ROM store is.
//!   <name>.stim         the input pins by half-cycle (reset, IRQ, NMI,
//!                       RDY, SO), so any rung replays the same run. RDY
//!                       is the level the 2A03 feeds its 6502 core, in
//!                       the record and the stimulus alike (the package
//!                       has no RDY pin; see CpuStep::core_rdy).
//!   <name>.events.json  what the console knows and the pins do not:
//!                       frame ends, every latch with its byte and its
//!                       reads, every $4016/$4017 read with the bit it
//!                       returned, every PPU register write with the dot
//!                       and line it landed on, every write into ROM
//!                       space (a mapper register), every NMI edge.
//!   <name>-f<i>.ppm     the last `pictures` frames as the PPU's colour
//!                       indices through an authored palette (for a look,
//!                       not a measurement; the family's real path is
//!                       ntsc-crt). PICTURES=n sets how many (default 2).
//!
//! Everything is in half-cycles, never converted: `h` is the pin
//! contract's own count, and the events file's `h` values are those.
//!
//! Latches and reads are derived from the pins (a write to $4016 whose D0
//! falls; a read of $4016 on the phi2 half-cycle), and the count is held
//! to the board's own poll log, which is the derivation pad-log.rs
//! prints. If they disagree the tool refuses: two instruments reading
//! one run must agree before either is believed.
use nes_console::{ines, Alignment, Console};
use nes_glue::controller::Buttons;
use std::io::Write as _;
use v6502_pins::{compare, parse_stim, parse_trace, write_stim, write_trace, Header, PinEngine as _, Stim, Trace};

/// An authored RGB approximation of the 2C02's 64 colours (the common
/// "2C02G" table), as run-rom.rs has it. For looking at frames only.
const PALETTE: [u32; 64] = [
    0x626262, 0x001fb2, 0x2404c8, 0x5200b2, 0x730076, 0x800024, 0x730b00, 0x522800, 0x244400, 0x005700, 0x005c00, 0x005324, 0x003c76, 0x000000, 0x000000, 0x000000,
    0xababab, 0x0d57ff, 0x4b30ff, 0x8a13ff, 0xbc08d6, 0xd21269, 0xc72e00, 0x9d5400, 0x607b00, 0x209800, 0x00a300, 0x009942, 0x007db4, 0x000000, 0x000000, 0x000000,
    0xffffff, 0x53aeff, 0x9085ff, 0xd365ff, 0xff57ff, 0xff5dcf, 0xff7757, 0xfa9e00, 0xbdc700, 0x7ae700, 0x43f011, 0x26e6a6, 0x2ccaff, 0x4e4e4e, 0x000000, 0x000000,
    0xffffff, 0xb6e1ff, 0xced1ff, 0xe9c3ff, 0xffbcff, 0xffbdf4, 0xffc6c3, 0xffd59a, 0xe9e681, 0xcef481, 0xb6fb9a, 0xa9fac3, 0xa9f0f4, 0xb8b8b8, 0x000000, 0x000000,
];

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = args.get(1).expect("usage: trace <rom.nes> <name> [frames] [script.txt] [out_dir]");
    let name = args.get(2).expect("a name for the files");
    let frames: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(120);
    let script = args.get(4).filter(|s| !s.is_empty()).cloned();
    let out_dir = std::path::PathBuf::from(args.get(5).cloned().unwrap_or_else(|| ".".into()));
    let pictures: usize = std::env::var("PICTURES").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    std::fs::create_dir_all(&out_dir).unwrap();

    let bytes = std::fs::read(rom_path).expect("rom");
    let rom = ines::parse(&bytes).expect("iNES");
    let rom_crc = crc32(&bytes[16..]);
    let mut pad = 0u8;
    let mut schedule: Vec<(u64, u8)> = Vec::new();
    let mut script_lines: Vec<String> = Vec::new();
    if let Some(path) = &script {
        for line in std::fs::read_to_string(path).expect("script").lines() {
            let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
            match f.as_slice() {
                ["AT", n, b] => {
                    schedule.push((n.parse().expect("latch"), u8::from_str_radix(b, 16).expect("hex byte")));
                    script_lines.push(line.trim().to_string());
                }
                ["SET", b] => {
                    pad = u8::from_str_radix(b, 16).expect("hex byte");
                    script_lines.push(line.trim().to_string());
                }
                _ => {}
            }
        }
    }

    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.cart().expect("a cartridge this console has");
    let reset_vector = {
        let p = &rom.prg;
        let n = p.len();
        // The vector as the CPU fetches it at power-on: the last bank's $FFFC.
        (p[n - 4] as u16) | ((p[n - 3] as u16) << 8)
    };
    let alignment = Alignment::default();
    let mut c = Console::with_prg_ram(cart, chr_ram, alignment, true);
    c.cpu_trace = Some(Vec::new());
    {
        let mut b = c.board.borrow_mut();
        b.pads[0].log_polls = true;
        b.pads[0].schedule = schedule.clone();
    }
    c.set_pad(0, Buttons::from_byte(pad));
    // The frame the reset sequence left behind is h = 0: the contract's
    // first line, before any step.
    let h0 = c.cpu.pins();

    // Run frame by frame, recording where each frame ended in the master
    // count and the dot count, so an event's dot and line are frame
    // relative.
    let mut frame_ends: Vec<(u64, u64, u64)> = Vec::new(); // (master, dots, cpu half-cycles) at the end of frame i
    let t = std::time::Instant::now();
    while c.frames.len() < frames {
        let before = c.frames.len();
        c.master_half_step();
        if c.frames.len() > before {
            frame_ends.push((c.master, c.dots, c.cpu_half_cycles));
        }
    }
    let dt = t.elapsed().as_secs_f64();
    // RAM=<path> writes the work RAM as the run left it (2 KiB): the stack
    // page and the rest, for T3's overlays and for checking a DMA's source
    // against what the record says it read.
    if let Ok(path) = std::env::var("RAM") {
        let b = c.board.borrow();
        let ram: Vec<u8> = (0..0x800u16).map(|a| b.wram.read(a)).collect();
        std::fs::write(&path, ram).unwrap();
    }
    let steps = c.cpu_trace.take().unwrap();
    eprintln!("ran {frames} frames, {} CPU half-cycles, in {dt:.1} s", steps.len());

    // ---------------------------------------------------------- the pins
    // The record's RDY is the level the 2A03 feeds its core, not the
    // rung's pin-level account of the hold (CpuStep::core_rdy): a 6502
    // released as the pin shows it goes straight on where the 2A03's core
    // re-runs its held read, and the switch-level 6502 on the record
    // parted at the first sprite DMA until this. The h = 0 frame is
    // before any hold.
    // The 2A03 rung feeds its core AFTER the step that decided the hold,
    // so the level a step recorded is in force from the NEXT step: frame
    // h carries the level fed after step h - 1, which is what the pin
    // crate's driver means by an input in frame h (driven before the
    // step that produced it). Written unshifted, the die was held one
    // cycle early at every DMA.
    let mut prev_core_rdy = true;
    let pins: Vec<v6502_pins::PinFrame> = std::iter::once(h0)
        .chain(steps.iter().map(|s| {
            let f = v6502_pins::PinFrame { rdy: prev_core_rdy, ..s.frame };
            prev_core_rdy = s.core_rdy;
            f
        }))
        .collect();
    let stamp = format!(
        "nes-console trace of {} (crc32 {rom_crc:08X}, mapper {}) over {frames} frames, alignment cpu {} ppu {}, script {}",
        std::path::Path::new(rom_path).file_name().unwrap().to_string_lossy(),
        rom.mapper, alignment.cpu_phase, alignment.ppu_phase,
        script.as_deref().unwrap_or("none")
    );
    let trace = Trace {
        header: Header {
            name: name.clone(),
            loads: Vec::new(),
            reset_vector,
            stim: format!("{name}.stim"),
            stamp,
            half_cycles: pins.len().saturating_sub(1) as u64,
        },
        frames: pins.clone(),
    };
    let text = write_trace(&trace);
    // What was written is what the pin crate reads back: the ladder's
    // own parser, then its own comparison, frame for frame.
    let back = parse_trace(&text).unwrap_or_else(|e| { eprintln!("REFUSED: the .pins written does not parse: {e}"); std::process::exit(1) });
    if let Err(m) = compare(&trace.frames, &back.frames) {
        eprintln!("REFUSED: the .pins read back differs from what was recorded: {m:?}");
        std::process::exit(1);
    }
    std::fs::write(out_dir.join(format!("{name}.pins")), text).unwrap();

    // ---------------------------------------------------------- the stim
    // The pin crate's driver applies a stimulus line at h BEFORE the step
    // that produces frame h + 1, and a frame carries the inputs as driven
    // through the step that produced it, so a level first seen in frame h
    // was driven at h - 1. (Written at h, every interrupt replayed a
    // half-cycle late; rung 0 on the record found it at the first NMI.)
    let mut stim: Vec<Stim> = Vec::new();
    let mut last: Option<(bool, bool, bool, bool, bool)> = None;
    for f in &pins {
        let now = (f.res, f.irq, f.nmi, f.rdy, f.so);
        if last != Some(now) {
            stim.push(Stim { h: f.h.saturating_sub(1), res: f.res, irq: f.irq, nmi: f.nmi, rdy: f.rdy, so: f.so });
            last = Some(now);
        }
    }
    let stim_text = write_stim(name, &stim);
    let stim_back = parse_stim(&stim_text).unwrap_or_else(|e| { eprintln!("REFUSED: the .stim written does not parse: {e}"); std::process::exit(1) });
    if stim_back != stim {
        eprintln!("REFUSED: the .stim read back differs from what was recorded");
        std::process::exit(1);
    }
    std::fs::write(out_dir.join(format!("{name}.stim")), stim_text).unwrap();

    // -------------------------------------------------------- the events
    // Frame, dot and line of a CPU half-cycle from its master step: a dot
    // is stepped on every master step congruent to the PPU phase.
    let dots_at = |m: u64| -> u64 { if m < alignment.ppu_phase as u64 { 0 } else { (m - alignment.ppu_phase as u64) / 8 + 1 } };
    let frame_of = |m: u64| -> (usize, u64) {
        let mut start_dots = 0u64;
        for (i, &(fm, fd, _)) in frame_ends.iter().enumerate() {
            if m <= fm {
                return (i, dots_at(m).saturating_sub(start_dots));
            }
            start_dots = fd;
        }
        (frame_ends.len(), dots_at(m).saturating_sub(start_dots))
    };
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(" \"name\": \"{name}\",\n \"rom\": {{\"file\": \"{}\", \"crc32_body\": \"{rom_crc:08X}\", \"mapper\": {}}},\n",
        std::path::Path::new(rom_path).file_name().unwrap().to_string_lossy(), rom.mapper));
    out.push_str(&format!(" \"alignment\": {{\"cpu_phase\": {}, \"ppu_phase\": {}}},\n", alignment.cpu_phase, alignment.ppu_phase));
    out.push_str(&format!(" \"script\": [{}],\n", script_lines.iter().map(|l| format!("\"{l}\"")).collect::<Vec<_>>().join(", ")));
    out.push_str(&format!(" \"half_cycles\": {},\n \"units\": \"h is the CPU half-cycle count of the .pins file; dot and line are frame relative (the odd frame's skipped dot is not corrected)\",\n", pins.len()));
    out.push_str(" \"frames\": [");
    for (i, &(m, d, h)) in frame_ends.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("{{\"index\": {i}, \"h_end\": {h}, \"master_end\": {m}, \"dots_end\": {d}}}"));
    }
    out.push_str("],\n");

    // Latches from the pins: a write to $4016 whose D0 falls. The byte and
    // the reads per latch from the board's own log.
    let polls = c.board.borrow().pads[0].polls.clone();
    let mut latches: Vec<(u64, usize)> = Vec::new(); // (h, frame)
    let mut strobe = false;
    let mut reads: Vec<(u64, u16, u8, usize)> = Vec::new(); // (h, addr, bit, latch index so far)
    let mut ppu_writes: Vec<(u64, u16, u8, usize, u64)> = Vec::new();
    // A write into ROM space is a mapper register (GxROM's bank latch):
    // the one thing the cartridge does that a picture would not explain.
    let mut cart_writes: Vec<(u64, u16, u8, usize)> = Vec::new();
    let mut nmi_edges: Vec<(u64, bool)> = Vec::new();
    let mut last_nmi = true;
    for s in &steps {
        let f = &s.frame;
        // The contract: a write is serviced as clk0 rises and a read as it
        // falls, so a write's byte is on the clk0-high frame and a read's
        // on the clk0-low one. Each is looked at on its own half only.
        if f.clk0 && !f.rw {
            if f.ab == 0x4016 {
                let d0 = f.db & 1 != 0;
                if strobe && !d0 {
                    latches.push((f.h, frame_of(s.master).0));
                }
                strobe = d0;
            }
            if (0x2000..=0x2007).contains(&f.ab) || f.ab == 0x4014 {
                let (fi, d) = frame_of(s.master);
                ppu_writes.push((f.h, f.ab, f.db, fi, d));
            }
            if f.ab >= 0x8000 {
                cart_writes.push((f.h, f.ab, f.db, frame_of(s.master).0));
            }
        }
        if !f.clk0 && f.rw && (f.ab == 0x4016 || f.ab == 0x4017) {
            reads.push((f.h, f.ab, f.db & 1, latches.len()));
        }
        let nmi_low = !f.nmi;
        if nmi_low != !last_nmi {
            nmi_edges.push((f.h, nmi_low));
        }
        last_nmi = f.nmi;
    }
    // Two instruments, one run: the board logged a poll per latch too.
    // The board pushes one entry per latch: the byte it loaded and the
    // reads since the latch before (pad-log.rs prints it the same way).
    let board_latches = polls.len();
    if board_latches != latches.len() {
        eprintln!("REFUSED: the pins show {} latches and the board's poll log {}; one instrument is wrong", latches.len(), board_latches);
        std::process::exit(1);
    }
    // Per latch, from the pins alone: the reads of $4016 that followed it
    // (its clocks) and the byte those reads spell (bit 0 = A, as the
    // port's register shifts out, set = pressed). The board logged the
    // same run: its clocks per poll must equal the pin count wherever
    // the poll has closed, and the byte the pins spell must equal what
    // the SCRIPT put on the pad at that latch index, which is the
    // derivation that makes the script the oracle and not the board.
    // MUTATE=1 reads the script one latch late and must go red.
    let mutate = std::env::var("MUTATE").is_ok();
    let expected_at = |i: u64| -> u8 {
        let mut b = pad;
        for &(n, v) in &schedule {
            if n <= i { b = v; }
        }
        b
    };
    let mut byte_failures = 0usize;
    out.push_str(" \"latches\": [");
    for (i, &(h, fi)) in latches.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        let bits: Vec<u8> = reads.iter().filter(|r| r.3 == i + 1 && r.1 == 0x4016).map(|r| r.2).collect();
        let clocks = bits.len();
        if let Some(p) = polls.get(i + 1) {
            if p.1 as usize != clocks {
                eprintln!("REFUSED: latch {i}: the pins show {clocks} reads of $4016 and the board's poll log {}", p.1);
                std::process::exit(1);
            }
        }
        // A read past the eighth returns 1 on D0 at this port; only the
        // register's eight bits spell the byte, and a pressed button reads
        // as 1 in the pin's D0 too (the register is loaded with the
        // buttons as they are, set = pressed).
        let spelled: u8 = bits.iter().take(8).enumerate().fold(0u8, |acc, (k, &b)| acc | (b << k));
        let mask: u8 = if clocks >= 8 { 0xff } else { (1u16 << clocks) as u8 - 1 };
        let want = expected_at(i as u64 + if mutate { 1 } else { 0 });
        let byte_ok = spelled & mask == want & mask;
        if !byte_ok { byte_failures += 1; }
        out.push_str(&format!("{{\"index\": {i}, \"h\": {h}, \"frame\": {fi}, \"byte\": \"{want:02x}\", \"clocks\": {clocks}, \"bits_read\": \"{spelled:02x}\", \"agree\": {byte_ok}}}"));
    }
    out.push_str("],\n");
    out.push_str(" \"reads\": [");
    for (i, &(h, a, bit, li)) in reads.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("{{\"h\": {h}, \"addr\": \"{a:04x}\", \"bit\": {bit}, \"after_latch\": {}}}", li as i64 - 1));
    }
    out.push_str("],\n");
    out.push_str(" \"ppu_writes\": [");
    for (i, &(h, a, v, fi, d)) in ppu_writes.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("{{\"h\": {h}, \"reg\": \"{a:04x}\", \"value\": \"{v:02x}\", \"frame\": {fi}, \"line\": {}, \"dot\": {}}}", d / 341, d % 341));
    }
    out.push_str("],\n");
    out.push_str(" \"cart_writes\": [");
    for (i, &(h, a, v, fi)) in cart_writes.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("{{\"h\": {h}, \"addr\": \"{a:04x}\", \"value\": \"{v:02x}\", \"frame\": {fi}}}"));
    }
    out.push_str("],\n");
    out.push_str(" \"nmi\": [");
    for (i, &(h, low)) in nmi_edges.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("{{\"h\": {h}, \"asserted\": {low}}}"));
    }
    out.push_str("]\n}\n");
    std::fs::write(out_dir.join(format!("{name}.events.json")), out).unwrap();

    // -------------------------------------------------------- the pictures
    let n = c.frames.len();
    for (k, f) in c.frames.iter().enumerate().skip(n.saturating_sub(pictures)) {
        let mut ppm = Vec::new();
        write!(ppm, "P6\n256 240\n255\n").unwrap();
        for y in 0..240usize {
            for x in 0..256usize {
                let (idx, _) = f.at(y, x + 1);
                let rgb = PALETTE[idx as usize & 63];
                ppm.extend_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
            }
        }
        std::fs::write(out_dir.join(format!("{name}-f{k}.ppm")), ppm).unwrap();
    }

    let bytes_assembled_ok = byte_failures == 0;
    println!(
        "{name}: {} half-cycles, {} frames, {} latches, {} pad reads, {} PPU writes, {} cart writes, {} NMI edges; bits read back {} the script's bytes; wrote {}.pins .stim .events.json and {} picture(s) in {}",
        pins.len(), frame_ends.len(), latches.len(), reads.len(), ppu_writes.len(), cart_writes.len(), nmi_edges.len(),
        if bytes_assembled_ok { "spell" } else { "DO NOT SPELL" }, name, pictures.min(n), out_dir.display()
    );
    if !bytes_assembled_ok {
        eprintln!("REFUSED: at {byte_failures} latch(es) the bits the pins read back are not the byte the script put on the pad{}", if mutate { " (MUTATE=1: the script read one latch late)" } else { "" });
        std::process::exit(2);
    }
}
