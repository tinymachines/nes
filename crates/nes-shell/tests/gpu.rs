//! N8 step 1's gate: the GPU picture against ntsc-crt's CPU chain.
//! Two consecutive console frames (the test cartridge, rendering on)
//! and a black frame after them, where the held picture shows, through `Decoder::decode` and `CrtPipeline::process`, and through the
//! eight compute passes, twice: the authored parameters, then with the
//! mask and the geometry on so every pass is exercised. Tolerance,
//! stated in the plan: every component of every pixel within 1e-3 in
//! linear light and the mean absolute difference within 1e-5. `MUTATE=1`
//! skips the persistence pass and the second and third frames must go
//! red. SKIPs by name without an adapter; `REQUIRE_GPU=1` insists.
//! The frame time on this box is measured and printed, not held.

use nes_bus::cart::{Mirroring, Nrom};
use nes_console::testrom::{chr, program};
use nes_console::{Alignment, Console, Picture};
use nes_shell::gpu::GpuPicture;
use ntsc_crt::{CrtParams, CrtPipeline, GeometryParams, MaskParams};
use ntsc_decode::Decoder;
use ntsc_grid::Profile;
use ntsc_source_nes::{burst_axis_offset, levels};

fn decoder() -> Decoder {
    Decoder::comb_three_line(Profile::Nes, burst_axis_offset(), levels::LOW[1], levels::HIGH[2])
}

/// Two console frames, then a black frame at the carried phase: the
/// third is where persistence shows (the first two are the test
/// cartridge's static picture, which a held previous frame can never
/// exceed).
fn frames() -> Vec<ntsc_grid::CompositeFrame> {
    let cart = Nrom::new(program(), chr(), Mirroring::Vertical).unwrap();
    let mut c = Console::new(Box::new(cart), None, Alignment::default());
    c.run_frames(8);
    let mut p = Picture::decode_only();
    let mut out: Vec<_> = c.frames.iter().skip(6).map(|f| p.encode(f)).collect();
    let black = nes_bus::DotFrame::filled(nes_bus::FrameParity::Even, 0x0f, 0);
    out.push(p.encode(&black));
    out
}

fn compare(label: &str, cpu: &ntsc_crt::DisplayFrame, gpu: &ntsc_crt::DisplayFrame) -> (f32, f64) {
    assert_eq!((cpu.width, cpu.height), (gpu.width, gpu.height));
    let mut worst = 0.0f32;
    let mut sum = 0.0f64;
    let mut at = (0, 0, 0);
    for (i, (a, b)) in cpu.data.iter().zip(&gpu.data).enumerate() {
        let d = (a - b).abs();
        sum += d as f64;
        if d > worst {
            worst = d;
            at = (i / 3 % cpu.width, i / 3 / cpu.width, i % 3);
        }
    }
    let mean = sum / cpu.data.len() as f64;
    eprintln!("{label}: worst {worst:.2e} at x {} y {} channel {}, mean {mean:.2e} over {} components", at.0, at.1, at.2, cpu.data.len());
    (worst, mean)
}

#[test]
fn the_gpu_picture_is_the_cpu_chain_pixel_for_pixel() {
    let mutate = std::env::var("MUTATE").is_ok_and(|v| v == "1");
    let dec = decoder();
    let frames = frames();
    let (n, lines) = (frames[0].lines[0].samples.len(), frames[0].lines.len());
    let mut worlds = vec![("authored", CrtParams::authored(3))];
    let mut on = CrtParams::authored(3);
    on.mask = Some(MaskParams { pitch: 1, off_gain: 0.7 });
    on.geometry = Some(GeometryParams { barrel_k: 0.05, corner_radius: 24.0 });
    worlds.push(("mask and geometry on", on));
    let mut reds = Vec::new();
    for (label, params) in &worlds {
        let Some(mut gpu) = GpuPicture::new(&dec, params, 1, 240, 2048, n, lines) else {
            if std::env::var_os("REQUIRE_GPU").is_some() {
                panic!("REQUIRE_GPU=1 but no adapter");
            }
            eprintln!("SKIP: no GPU adapter");
            return;
        };
        let mut cpu = CrtPipeline::new(params.clone());
        for (k, frame) in frames.iter().enumerate() {
            let want = cpu.process(&dec.decode(frame, 1, 240, 2048));
            gpu.run(frame);
            let got = gpu.read_back();
            let (worst, mean) = compare(&format!("{label}, frame {k} on {}", gpu.adapter_name), &want, &got);
            let held = worst <= 1e-3 && mean <= 1e-5;
            if !held {
                reds.push((label.to_string(), k));
            }
            if !mutate {
                assert!(held, "{label}, frame {k}: worst {worst:.2e}, mean {mean:.2e}");
            }
        }
        // The frame time, bracketed by the queue: recorded.
        let t = std::time::Instant::now();
        for _ in 0..30 {
            gpu.run(&frames[2]);
        }
        gpu.wait();
        eprintln!("{label}: {:.2} ms a frame on the GPU, upload included, over 30 frames", t.elapsed().as_secs_f64() * 1e3 / 30.0);
    }
    if mutate {
        assert!(reds.iter().any(|r| r.1 > 0), "MUTATE=1: no persistence and the later frames still agreed");
        panic!("MUTATE=1: red on {reds:?} (this panic is the red)");
    }
}
