//! Where a frame's time goes on one core: the console itself, the
//! encoder, the comb decoder, the CRT stages, the sound. The N8 plan's
//! numbers.
//!   cargo run --release -p nes-console --example picture-bench -- rom.nes [frames]
use nes_console::{ines, Alignment, Console, Picture, Sound};
use ntsc_crt::{CrtParams, CrtPipeline};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("rom");
    let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let rom = ines::parse(&bytes).expect("iNES");
    let chr_ram = rom.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = rom.nrom().expect("NROM");
    let mut c = Console::with_prg_ram(Box::new(cart), chr_ram, Alignment::default(), true);
    c.run_frames(30);
    let t = std::time::Instant::now();
    c.run_frames(frames);
    let console_ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    c.sound = Some(Sound::default());
    let t = std::time::Instant::now();
    c.run_frames(frames);
    let with_sound_ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    let tail: Vec<_> = c.frames.iter().rev().take(frames).rev().cloned().collect();
    let mut p = Picture::decode_only();
    let t = std::time::Instant::now();
    let encoded: Vec<_> = tail.iter().map(|f| p.encode(f)).collect();
    let encode_ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    let t = std::time::Instant::now();
    let decoded: Vec<_> = encoded.iter().map(|f| p.decoder().decode(f, 1, 240, 2048)).collect();
    let decode_ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    let mut crt = CrtPipeline::new(CrtParams::authored(3));
    let t = std::time::Instant::now();
    for d in &decoded {
        let _ = crt.process(d);
    }
    let crt_ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    println!(
        "per frame, one core: console {console_ms:.2} ms, console with sound {with_sound_ms:.2} ms, encode {encode_ms:.2} ms, decode (comb) {decode_ms:.2} ms, CRT stages {crt_ms:.2} ms; the frame period is 16.64 ms"
    );
}
