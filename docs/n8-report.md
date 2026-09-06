# N8 report: the shell, built and gated where a box without a screen can gate it

Run stamp: 2026-09-06, rustc 1.97.1, wgpu 22.1, winit 0.30, cpal 0.15,
wasm-pack 0.13 under node 24. Pins as N7's, ntsc-crt v0.2.4. `cargo
test -p nes-shell`: 3 tests (the GPU picture, the paced loop, the
ring), 8 with the addendum's pad tests. Plan: `docs/n8-plan.md`, written first.

The console has a shell: a Linux binary that puts the picture in a
window with the decode and the CRT stages on the GPU, the console
paced by the wall clock on its own thread, the sound out through the
audio device and the keyboard as controller 1; and a wasm build of
the console with its sound, measured under node. The window ran here
under a virtual display on one of the box's two RTX 3070s; a desk
with a screen, a speaker and a hand is what remains.

## Where a frame's time went, and where it goes now

`picture-bench` (one core, full_palette.nes, this box under an
unrelated load average of eight): the console 7.1 ms a frame, with the
sound 13.0 at the start of the milestone and 9.3 at its end (the
sound's DACs tabulated, its resampler on a contiguous buffer with no
per-sample arithmetic: 1.7 ms where it was 5.9); the encoder 5.0; the
three-line comb decode 28; the CRT stages 55. The period is 16.64 ms.
The decode and the stages are the 83 ms that could not fit any core,
and they now take about 1 ms on the GPU, upload included.

## Step 1: the picture on the GPU

`nes_shell::gpu::GpuPicture`: eight compute passes over storage
buffers (`picture.wgsl`), every constant uploaded from the `Decoder`
and `CrtParams` instances rather than typed: the comb's chroma
demodulated at each line's phase and block-averaged by the
decimation; the decimated lowpass with replicated edges; the luma
scale, Catmull-Rom back to the grid, the matrix, the clamp and the
display gamma; the beam's Gaussian columns; the scanlines as the
gather form of the CPU's scatter with the same 1e-4 cutoff and the
same summation order; persistence against a state buffer; the mask;
the geometry with bilinear sampling. The window blits the last buffer
gamma-encoded (`blit.wgsl`).

Gate (`tests/gpu.rs`): two console frames and a black one after them
(where the held picture shows) through `Decoder::decode` and
`CrtPipeline::process` and through the passes, with the authored
parameters and again with the mask and the geometry on. Tolerance,
from the plan: every component within 1e-3 in linear light, the mean
within 1e-5.

| | worst component | mean | GPU time a frame |
|---|---|---|---|
| authored parameters | 4.8e-7 | 1.2e-8 | 1.02 ms |
| mask and geometry on | 2.8e-5 | 1.9e-7 | 1.08 ms |

The geometry's bilinear sample at a fractional position is where the
two f32 chains part by the most, and it is a hundred times inside the
tolerance. `MUTATE=1` skips persistence and the black frame is red at
0.53 on both worlds. SKIPs by name without an adapter; `REQUIRE_GPU=1`
insists.

## Step 2: the shell

`nes-shell rom.nes`. The console thread builds the console (its chips
share state through `Rc`, so it is built where it runs), and each
period runs what ntsc-wasm's `Pacing` says is due from the wall clock
(a tick that took two periods runs two frames and counts a drop; a
tick that took none presents the previous frame again and counts a
duplicate), publishes its newest frame, banks the sound in a ring
capped at a quarter second, and sleeps the rest of the period. The
display thread, paced by the same period from its own clock so a
surface without vsync cannot spin, encodes each new frame on the CPU,
runs the GPU picture and blits it; cpal drains the ring at 48 kHz;
the keyboard sets the pad the console thread reads each frame. On
exit the counters print. `NES_SHELL_TICKS=n` exits after n redraws.

Gate (`tests/pacing.rs`, no window): the loop on a synthetic clock. At
the period, 60 ticks run 59 frames with one duplicate (the period is
not a whole number of nanoseconds and the first tick falls a
nanosecond short); at half the period every other tick duplicates; at
twice it drops one per tick. One frame banks 798.7 samples; the ring
holds 12,000 and a drain past that counts the underrun.

The window, under Xvfb on this box, 600 redraws of full_palette.nes
with the load average at eight: the console thread's frame 11.4 ms
mean and 82 ms worst, 134 of 1,467 over 16 ms, 2 over 33; 74 drops in
1,394 periods, 4,800 audio underrun samples (the device does not
exist here; the ring's counter is the drain the shell's own smoke run
makes). Recorded, not held: the box was not idle and the display was
not real.

## Step 3: the second target

`cargo check --target wasm32-unknown-unknown -p nes-console` passes:
the console, both rungs and ntsc-crt compile for the browser, because
the chip tables are built on the host at build time. `nes-wasm` is the
bridge (`Nes::new(rom)`, `run_frames`, the colour and emphasis planes
and the parity ntsc-wasm's pipeline takes, the sound drained, the pad
as a byte); `tools/wasm-bench.mjs` under node: 300 frames of
full_palette.nes with the sound on in 3.29 s, 91.1 frames a second,
1.52x real time, 798.7 sound samples a frame. The page that hosts it
is the roof's item.

## What stays for the desk

- The window on a real display: the picture, the pacing counters at
  the end of a session, whether the scanlines and persistence read
  right at scale 3.
- A speaker: the ring's underrun counter at zero over a session.
- A hand on the keyboard, and gate 3's cartridge, which is also where
  N5's play gate closes.
- The wasm page in the roof (since built: `/nes/play`).
- A gamepad in hand: the mapping below ran without one.

## Addendum, 2026-09-06: a gamepad beside the keyboard

`nes_shell::pad` (gilrs 0.11) drains the pad's events each redraw
into a `Buttons` the window ORs with the keyboard's before the console
thread reads it. The layout is positional against the NES pad: the
east and north face buttons are A, the south and west are B, Start
and Select and the D-pad by name, the left stick past half deflection
on either axis as a direction (the same edge both ways, and a stick
that has not crossed the edge writes nothing, so it cannot undo a held
D-pad). No pad, or no enumerator, is not an error: the keyboard still
plays and the reason prints once. Gate (`tests/pad.rs`, 5 tests, no
pad): the layout table including what maps to nothing, the stick's
edge both ways on both axes and the right stick and triggers ignored,
the D-pad surviving stick noise, the OR, and `Pad::open` on this box
(the enumerator opens, no pad sends an event). The window's smoke run
under the virtual display is unchanged with the module in: 90 redraws,
88 new frames, 0 underrun samples, 0 gamepads. What a real pad feels
like, and whether east-is-A is the thumb's choice, is the desk's.
