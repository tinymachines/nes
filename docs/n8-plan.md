# N8 plan: the shell and packaging

Written before the code (2026-09-06). The sketch (section 3, N8): a
Linux native binary, the console, a window, the CRT stages in a shader
(the one place the GPUs earn their keep), controller input, audio out;
the die explorers stay in the browser; the WASM build kept alive as a
second target, measured separately. N6 deferred wall-clock pacing and
the drift policy here.

## What exists, and what a frame costs

`picture-bench` on full_palette.nes, one core (2026-09-06): the
console 7.15 ms a frame, 13.01 with the sound; the NES encoder 4.96;
the three-line comb decode 28.02; the CRT stages 54.76. The period is
16.64 ms. So the console and the encoder fit a core with room, and
the decode and the stages do not fit anywhere on the CPU: 83 ms of
the 101. Those two go to the GPU, which is the sketch's sentence.

- ntsc-crt's decoder and stages are the oracle: every pass below is
  held to them, not to a picture. The decoder's constants (comb
  weights, the decimated UV lowpass and its decimation, the matrix,
  the demodulation offset, black and white, the gamma) are its own
  public fields and are uploaded from the instance, never typed.
- ntsc-wasm's `Pacing` is the drift policy already tested: the source
  advances by whole frames of the exact period per wall-clock tick,
  duplicates and drops counted, never resampled in time. It is plain
  Rust and builds natively.
- The 6502's `v6502-gpu` is the family's wgpu precedent (wgpu 22,
  pollster, headless adapter; the tests SKIP without one).
- This box: two RTX 3070s, no display session, ALSA and the X11 and
  Wayland client libraries installed. So the GPU passes are gated
  here headlessly, the binary builds here, and running it in a window
  is a desk item.

## Steps

1. **The GPU picture** (`crates/nes-shell/src/gpu.rs`, WGSL beside it).
   A `GpuPicture` takes a `CompositeFrame` (the encoder's, on the CPU,
   5 ms) and runs, as compute passes: the three-line comb and the
   demodulation (rotated sine tables at the line's phase, the boxcar
   decimation, the decimated lowpass, Catmull-Rom back, the luma
   scale) to YUV; the matrix, clamp and display gamma to linear RGB
   on the 2048 x 240 grid; the beam (Gaussian columns to 256 x scale);
   the scanlines (the gather form of the CPU's scatter, the same
   1e-4 cutoff); persistence (a ping-pong state, max of excitation
   and the decayed previous); the mask and the geometry when their
   parameters are Some. Output linear RGB at (256 x scale) x (240 x
   scale), read back for the gate and gamma-encoded in the blit for
   the window.

   Gate (`tests/gpu.rs`, SKIPs by name without an adapter,
   `REQUIRE_GPU=1` insists): three consecutive console frames (the
   test cartridge, rendering on) through the CPU chain
   (`Decoder::decode`, `CrtPipeline::process`) and through the GPU
   passes, with the mask and geometry on for the second run so every
   pass is exercised. Tolerance, stated now: every component of every
   output pixel within 1e-3 in linear light, and the mean absolute
   difference within 1e-5 (the two are f32 with different summation
   orders and library `exp` and `pow`; a wrong pass is off by far
   more than that). `MUTATE=1` skips the persistence pass and the
   second and third frames must go red. Frame time measured on this
   box and recorded, not held.

2. **The shell** (`crates/nes-shell/src/main.rs`): `nes-shell rom.nes`.
   The console with the sound on runs on its own thread at the exact
   rate, `Pacing` deciding each display tick how many source frames
   to advance (zero presents the previous picture again, more than
   one drops, both counted and printed on exit); the render thread
   encodes the frame and runs the GPU picture into a winit window on
   a wgpu surface, integer scale 3. Audio through cpal at 48 kHz from
   a ring the console thread fills, underruns counted. The keyboard
   is controller 1 (arrows, Z and X for B and A, Enter and right
   shift for Start and Select, Escape to quit). Nothing here is
   measured against anything but its own counters; it is the thing
   the sketch asks for and its gate is a desk with a screen.

   What is gated here: the shell's core loop without a window (the
   console thread, the pacing, the ring) runs for 120 ticks with a
   synthetic clock and the counts come out as the arithmetic says
   (`tests/pacing.rs`: at exactly the period no duplicates and no
   drops; at half the period every other tick duplicates; at twice it
   drops one per tick).

3. **The WASM target.** `cargo check --target wasm32-unknown-unknown
   -p nes-console`: the console, its rungs and ntsc-crt compile for
   the browser (the chip tables are built at build time on the host,
   which is what makes this possible). A `nes-wasm` bridge with
   `run_frames` and a node bench measuring frames a second is the
   second target measured separately; the page that hosts it is the
   roof's item, not this repository's.

4. **Packaging and the report.** `cargo build --release -p nes-shell`
   is the binary; the README says how to run it and what the keys
   are; ROMs never committed. `docs/n8-report.md`: the GPU gate's
   figures, the frame time on this box, the pacing gate, the wasm
   check, and the desk items (the window, the audio device, a
   controller).

## Not in N8

A gamepad (winit gives the keyboard; gilrs is a registry dependency
for another day). Save states, rewind, the die explorers. Anything
the bench items of N5 to N7 wait on.
