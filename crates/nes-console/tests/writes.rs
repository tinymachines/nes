//! The write log (`nes_console::writes`) held to what `storyboard` wrote
//! before the log was a module: the four files it made of 300 frames of
//! the write log's cartridge (`testrom::writes_rom`) and of the test
//! cartridge, by line count and digest, taken from the example as it
//! stood at 85a9bc3 and never from this code. The first is the case
//! with something in every log; the test cartridge writes a handful of
//! lines while it sets up and, being an NROM, none to its own range,
//! which is said here rather than passed over.
//!
//! MUTATE_WRITELOG=1 drops every write of an odd value and must go red.

use nes_console::writes::{Kind, WriteLog};
use nes_console::{ines, testrom, Alignment, Console};

/// `storyboard`'s run: the console the image powers on into, its trace
/// on, a frame at a time.
fn storyboard(rom: &[u8], frames: u64) -> WriteLog {
    let r = ines::parse(rom).expect("an iNES image");
    let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
    let mut console = Console::with_prg_ram(r.cart().expect("a board"), chr_ram, Alignment::default(), true);
    console.trace = Some(Default::default());
    let mut log = WriteLog::default();
    for f in 0..frames {
        console.run_frames(1);
        let tr = console.trace.as_mut().unwrap();
        log.frame(f, &tr.bytes);
        tr.bytes.clear();
        console.frames.clear();
    }
    log
}

/// The test cartridge as `export-testrom` writes it.
fn testcart() -> Vec<u8> {
    let mut rom = vec![b'N', b'E', b'S', 0x1a, 2, 1, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
    rom.extend(testrom::program());
    rom.extend(testrom::chr());
    rom
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn held(log: &mut WriteLog, want: [(usize, u64); 4]) {
    for (kind, (lines, digest)) in Kind::ALL.into_iter().zip(want) {
        let text = log.take(kind);
        assert_eq!(text.lines().count(), lines, "{}: lines", kind.name());
        assert_eq!(fnv(&text), digest, "{}: the text\n{}", kind.name(), text.lines().take(12).collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn the_write_log_s_cartridge_logs_what_storyboard_wrote_of_it() {
    let mut log = storyboard(&testrom::writes_rom(), 300);
    // Something in every log, before the digests are worth anything.
    for (text, name) in [(&log.vram, "vram"), (&log.scroll, "scroll"), (&log.apu, "apu"), (&log.mapper, "mapper")] {
        assert!(text.lines().count() > 250, "{name}: a line a frame at least");
    }
    assert!(log.vram.starts_with("1 3F00 4 1\n2 2001 3 1\n2 2101 2 32\n"), "a run at each step");
    assert!(log.scroll.starts_with("2 241 1 0\n3 161 1 0\n"), "the blank's pair, then the split's");
    assert!(log.apu.starts_with("0 17 64\n2 15 15\n2 02 1\n"));
    assert!(log.mapper.starts_with("2 FFF0 1\n3 FFF0 0\n"));
    held(&mut log, [(597, 0x9b0c_a986_1b1e_69be), (595, 0xfc14_de8c_a560_ae83), (1193, 0x2c64_711b_87bf_f2ea), (298, 0xf985_7204_f42f_2f2f)]);
}

#[test]
fn the_test_cartridge_logs_its_setup_and_nothing_to_its_own_range() {
    let mut log = storyboard(&testcart(), 300);
    assert_eq!(log.vram, "1 3F00 4 1\n1 2000 246 1\n2 20F6 778 1\n");
    assert_eq!(log.scroll, "2 60 0 0\n");
    assert_eq!(log.apu, "0 17 64\n");
    assert_eq!(log.mapper, "", "an NROM has no register to write");
    assert_eq!(log.take(Kind::Vram).lines().count(), 3);
    assert!(log.vram.is_empty(), "a take empties the log");
}

#[test]
fn a_log_is_asked_for_by_the_name_its_file_has() {
    for kind in Kind::ALL {
        assert_eq!(Kind::named(kind.name()), Some(kind));
    }
    assert_eq!(Kind::named("ppu"), None);
}
