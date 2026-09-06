# N6 plan: the picture

Written before the code (2026-09-06), the way P3's plan was. The
sketch (`docs/nes-end-to-end-v0_2.md`, section 3, N6) asks for two
things: the PPU ladder's `DotFrame` into ntsc-crt's NES source with the
Rung C decode by default and the CRT stages on, and a gate that
compares the console's picture with a real console's through the same
capture procedure ntsc-crt's M4 uses, on a colour-bars cartridge in
both, with tolerances stated before measuring.

## What exists

- ntsc-crt (v0.2.3) has every stage: `ntsc-source-nes` encodes a
  `DotFrame` to composite samples, direct synthesis like the PPU;
  `ntsc-decode` has the three-line comb (Rung C) with the NES-native
  weights; `ntsc-crt` has the beam, scanlines, persistence, mask and
  geometry; `ntsc-wasm`'s `NesPipeline` is the plain-Rust encode and
  decode the browser page runs, frame after frame with the subcarrier
  phase carried by parity (`CompositeFrame::next_origin`). Its bridge is
  wasm-only behind a cfg, so the console uses it natively.
- `ntsc-source-cap` reads a captured waveform into the same frame
  type (sync, burst lock, sinc resample, DC re-referenced) and carries
  the capture-card model M4 proved the roundtrip on; its
  `score-real-region` example scores a flat region of a real capture
  against the same colour synthesized, both sides decoded identically.
- The console (N5) produces `DotFrame`s with the parity the PPU ran,
  and its plumbing gate holds each to the standalone PPU's.
- Captures on hand: five records of Super Mario Bros. and Duck Hunt
  from the real console (M4). No bars capture yet, and no ROM for
  either game here. `full_palette.nes` (blargg, NROM) paints every
  palette entry as bars and runs on the console; `240pee.nes` has SMPTE
  bars but is mapper 2, out of the console's scope.

## Steps

1. **The picture through the console.** `nes-console` gains
   `picture`: a `Picture` that takes the console's frames in order,
   encodes each with the NES source at the phase the previous frame
   left, decodes on Rung C, and optionally runs the CRT stages; ntsc-crt
   pinned by tag, the same nes-bus underneath. `run-rom` gains
   `CRT=out.ppm` (the displayed picture) beside its palette dump.

   Gate (`tests/picture.rs`): a console frame through the picture is
   byte for byte what the standalone PPU's frame from the same world is
   through the same picture (the seam adds nothing, now past the
   encoder), and the phase the picture carries across the console's
   real parity sequence is what ntsc-grid's arithmetic gives that
   sequence (the odd frame's short line moves the phase; the console
   must pass the parity, not just the dots). A mutation (the parity
   forced Even) must go red on the second.

2. **The capture path, machine half.** An example `capture-score`
   runs a ROM in the console, encodes its frames, and either reads a
   real capture (the `.u8`/`.wav` the M4 tools read, at the recorder's
   rate) or runs the frames through ntsc-crt's capture-card model
   (synthetic, at the same rate), recovers it the way M4 does, and
   scores named flat regions against the console's own synthesis
   through the identical decoder: luma, hue, saturation. With
   `full_palette.nes` and the synthetic capture the whole path closes
   on this machine; the real capture of the same ROM on the real box is
   the bench item.

   Tolerances, stated now. Synthetic roundtrip (the card model's own
   noise and 5 ppm): luma within 0.01, hue within 1.0 degree,
   saturation within 5 percent of the synthesis, on every flat region
   scored; a miss fails. Real capture: recorded beside the synthetic
   figures and NOT held, because M4's one scored real region already
   reads 28 percent hot in saturation for a reason the sketch's section
   5 assigns to the bench (probe versus DAC); a hue or luma miss
   beyond the synthetic tolerance is named.

3. **The report** (`docs/n6-report.md`): the picture, the gate's
   figures, the capture path's synthetic figures per region, the real
   capture's status, and what stays for the bench.

## Not in N6

Wall-clock pacing and the drift policy (N8, with the shell). The CRT
parameters stay ntsc-crt's authored ones; nothing here fits them.
